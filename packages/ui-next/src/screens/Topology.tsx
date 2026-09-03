import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  prometheusDiscover,
  topologyGraph,
  type ClusterContext,
  type TopologyEdge,
  type TopologyHealth,
  type TopologyLane,
} from "@srelens/core";
import { Button, EmptyState, Screen, StatusPill, type StatusKind } from "@srelens/ui-kit";
import { useNamespaceOptions } from "@srelens/core/react";
import { getKubeconfigFiles, useActiveContext } from "../lib/clusters";
import {
  NamespaceErrorAlert,
  NamespacePicker,
  NoClusterScreen,
  StaleSelectionAlert,
} from "./resourceShell";
import { FailureAlert } from "../lib/errorCopy";
import { setNamespaces, useNamespaces } from "../lib/workspace";
import { useResource } from "../lib/useResource";
import {
  FLOW_ANIMATION_LIMIT,
  LANE_LABEL,
  MAX_PIPS,
  NODE_HEIGHT,
  NODE_WIDTH,
  PADDING,
  clampZoom,
  fit,
  fitTransform,
  hubCounts,
  layoutFlow,
  traceFrom,
  zoomAt,
  type PlacedEdge,
  type PlacedNode,
  type TopologyLayout,
  type Trace,
  type Transform,
} from "../lib/topologyFlow";

/**
 * How traffic reaches a workload, and where it goes next, across the
 * namespaces a reader picks.
 *
 * **The picture is laid out by flow, not by kind.** A node's column is how
 * many hops it stands from where traffic enters — `ingress`, then the Service
 * it names, then the Deployment behind it, then whatever that Deployment
 * calls. Kind is a glyph on the box. The version this replaces gave each
 * Kubernetes kind a fixed column, which meant every call from one service to
 * another pointed backwards; see {@link ../lib/topologyFlow} for that argument
 * in full.
 *
 * **Every edge carries its provenance, and is drawn differently for it.** The
 * structural half — selectors, ownerReferences, Ingress rules — is the API
 * server's own word. The declared half is read out of configuration:
 * environment variables, arguments and ConfigMaps, which say a workload was
 * BUILT to talk to something, not that it ever has. Those two must not look
 * alike, or a reader trusts them equally.
 *
 * When the cluster already runs a Prometheus, `k8s.prometheusDiscover` finds
 * it and measured traffic joins the same graph: those edges are accented, they
 * carry a rate, they are drawn as thick as that rate is large, and they move.
 * A measurement UPGRADES the declared edge rather than sitting beside it — one
 * dependency now known better, not two. A cluster with no metrics backend is
 * the ordinary case and loses nothing but the rates.
 *
 * Read once per selection rather than polled. A topology changes on deploys,
 * not continuously, and a picture that rearranged itself under a reader
 * mid-trace would be worse than a slightly old one — which is also why the
 * layout is deliberately stable.
 */
export function Topology() {
  const context = useActiveContext();
  if (!context) return <NoClusterScreen title="Topology" noun="topology" />;
  return <TopologyGraph context={context} />;
}

/**
 * How many namespaces "All namespaces" will actually draw.
 *
 * The picker is the app's, and to it an empty selection means ALL — which the
 * lists can honour because a table of ten thousand rows still scrolls. A graph
 * cannot: every namespace on a real cluster is a picture of nothing, and the
 * capability makes seven list calls per namespace to build it. So "all" is
 * honoured up to here and the reader is told when it was cut, rather than the
 * screen either lying about its scope or hanging on a large cluster.
 */
const NAMESPACE_LIMIT = 12;

function TopologyGraph({ context }: { context: ClusterContext }) {
  const cluster = context;
  /**
   * Which namespaces there ARE — the same hook, picker and error surface the
   * resource lists and the events screen use.
   *
   * Not `useNamespaces`: that store holds the reader's standing FILTER, and
   * reading it as the list of namespaces gives a picker with no options on a
   * perfectly healthy cluster.
   */
  const { namespaces, error: namespaceError } = useNamespaceOptions(
    context.name,
    getKubeconfigFiles(),
  );

  /**
   * The reader's namespace selection — the workspace's, not this screen's.
   *
   * One selection per cluster, shared by every screen looking at it: narrowing
   * to `payments` here narrows the resource lists and the events screen too,
   * and arriving from one of those lands on what they were already looking at.
   */
  const scoped = useNamespaces(cluster?.stableId);

  /**
   * A metrics backend, if the cluster already runs one.
   *
   * Read once per cluster and never blocking: the graph is built from the API
   * either way, and telemetry only adds observed edges and rates on top.
   */
  const metrics = useResource(async () => {
    const out = await prometheusDiscover(context.name);
    return out.candidates ?? [];
  }, [context.name]);
  const source = metrics.data?.[0];

  /**
   * Whether to read each pod's socket table.
   *
   * Off until asked, and this screen's own state rather than the workspace's:
   * it is one `kubectl exec` per pod and an audit-log entry on each, so it is
   * something a reader does once while looking at something, not a preference
   * that follows them around.
   */
  const [probeConnections, setProbeConnections] = useState(false);

  const all = namespaces ?? [];
  const chosen = scoped.length > 0 ? scoped : all.slice(0, NAMESPACE_LIMIT);
  const cut = scoped.length === 0 && all.length > NAMESPACE_LIMIT;
  // Sorted and joined for the dependency array, so re-picking the same set in a
  // different order does not re-read the cluster.
  const key = [...chosen].sort().join(",");

  const graph = useResource(
    async () => {
      if (!cluster || chosen.length === 0) return { nodes: [], edges: [] };
      const out = await topologyGraph(cluster.name, chosen, source, probeConnections);
      if (out.error) throw new Error(out.error);
      return out.graph ?? { nodes: [], edges: [] };
    },
    // Re-read when a metrics source appears: discovery resolves after the first
    // draw, and the graph would otherwise stay structural until something else
    // moved.
    [cluster?.name, key, source?.service, source?.namespace, probeConnections],
    (g) => g.nodes.length === 0,
  );

  const layout = useMemo(() => layoutFlow(graph.data ?? { nodes: [], edges: [] }), [graph.data]);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Found rather than held, so the panel follows a reload instead of showing
  // the node as it was when it was clicked.
  const selected = layout.nodes.find((n) => n.id === selectedId) ?? null;

  /**
   * Everything on a path through the selection, in both directions.
   *
   * Computed once here and used by both halves of the screen, so the canvas
   * and the panel can never disagree about what the selection reaches.
   */
  const trace = useMemo(
    () => (selected ? traceFrom(selected.id, layout.edges) : null),
    [selected, layout.edges],
  );

  return (
    <Screen
      title="Topology"
      eyebrow={cluster?.name}
      actions={
        <>
          {/* Named for what it COSTS, not for what it shows: a reader has to
              know this runs an exec in every pod before they turn it on. */}
          <Button
            variant={probeConnections ? "primary" : "secondary"}
            onClick={() => setProbeConnections((on) => !on)}
            title="Runs `cat /proc/net/tcp` in each pod, over pods/exec"
          >
            {probeConnections ? "Probing connections" : "Probe connections"}
          </Button>
          {/* Inherited, not grown here: the same control and the same selection
              every list on this cluster uses. */}
          <NamespacePicker
            namespaces={namespaces}
            selection={scoped}
            onChange={(next) => {
              setNamespaces(context.stableId, next);
              // The old selection may name a node in a namespace no longer drawn.
              setSelectedId(null);
            }}
          />
        </>
      }
      fill
    >
      <div className="flex h-full min-h-0">
        <div className="flex min-w-0 flex-1 flex-col">
          {/* Both the app's, so a namespace listing that failed and a selection
              naming namespaces the cluster no longer has read the same here as
              they do on every list. Collapsed when all three are quiet, rather
              than holding a band of empty padding above the canvas. */}
          <div className="px-4 pt-4 empty:hidden">
            <NamespaceErrorAlert error={namespaceError} />
            <StaleSelectionAlert
              selection={scoped}
              namespaces={namespaces}
              onReset={() => setNamespaces(context.stableId, [])}
            />
            {cut && (
              // Said rather than done quietly: the picker reads "All
              // namespaces" and this is not all of them.
              <p className="text-sm text-muted">
                Showing {NAMESPACE_LIMIT} of {all.length} namespaces. Pick the ones you want to see.
              </p>
            )}
          </div>
          <div className="relative min-h-0 flex-1">
            {graph.status === "loading" && (
              <p className="grid h-full place-items-center text-sm text-muted">
                Reading the cluster…
              </p>
            )}
            {graph.status === "error" && (
              <div className="p-4">
                <FailureAlert title="Could not draw this topology" error={graph.error} />
              </div>
            )}
            {graph.status === "empty" && (
              <div className="grid h-full place-items-center">
                <EmptyState
                  title={
                    chosen.length === 1 ? `Nothing to draw in ${chosen[0]}` : "Nothing to draw here"
                  }
                  hint={
                    chosen.length === 1
                      ? "This namespace has no ingresses, services or workloads."
                      : "None of these namespaces has an ingress, service or workload."
                  }
                />
              </div>
            )}
            {graph.status === "ready" && (
              <Canvas
                layout={layout}
                selectedId={selectedId}
                trace={trace}
                onSelect={setSelectedId}
              />
            )}
          </div>
        </div>
        <aside className="rule-l scroll w-[300px] shrink-0 p-4" aria-label="Selected node">
          <Inspector node={selected} layout={layout} trace={trace} onSelect={setSelectedId} />
        </aside>
      </div>
    </Screen>
  );
}

/** The kit's tone for a node's health. */
function pillKind(health: TopologyHealth): StatusKind {
  if (health === "ok") return "success";
  if (health === "degraded") return "warning";
  if (health === "failing") return "danger";
  return "neutral";
}

/**
 * The stroke a health takes.
 *
 * Classes rather than `var(--sev)` inline, so the graph moves with the theme
 * the same way every other surface does.
 */
function strokeFor(health: TopologyHealth): string {
  if (health === "ok") return "stroke-ok";
  if (health === "degraded") return "stroke-warn";
  if (health === "failing") return "stroke-sev";
  return "stroke-rule-strong";
}

function fillFor(health: TopologyHealth): string {
  if (health === "ok") return "fill-ok";
  if (health === "degraded") return "fill-warn";
  if (health === "failing") return "fill-sev";
  return "fill-rule";
}

/**
 * The card's own ground.
 *
 * Healthy is plain white, because healthy is the background state of a cluster
 * and tinting every node its own colour would leave nothing standing out. What
 * is NOT healthy takes the matching wash, so a failing workload is a coloured
 * card in a field of white ones rather than the same card with a different
 * hairline round it — which, at the zoom a real namespace needs, was no
 * difference at all.
 */
function cardFill(health: TopologyHealth): string {
  if (health === "degraded") return "fill-warn-wash";
  if (health === "failing") return "fill-sev-wash";
  return "fill-raised";
}

/**
 * How an edge is drawn, which encodes HOW IT IS KNOWN rather than what it says.
 *
 * A failing path outranks everything else: it takes the severity colour and a
 * long dash, so it is marked twice and stays legible in the high-contrast
 * theme and to a reader who cannot separate the colours. Then measurement —
 * accented, moving, and as thick as the rate is large. Then `declared`, a host
 * named in configuration, kept faint and dotted because it says only that this
 * workload was built to reach that. Everything left is the API server's own
 * word, drawn in the health of what it points at.
 */
function edgeTone(edge: TopologyEdge): { stroke: string; fill: string; dash?: string; flow: boolean } {
  if (edge.health === "failing") {
    return { stroke: "stroke-sev", fill: "fill-sev", dash: "6 3", flow: false };
  }
  if (edge.provenance === "observed") {
    return { stroke: "stroke-accent", fill: "fill-accent", flow: true };
  }
  if (edge.provenance === "declared") {
    return { stroke: "stroke-faint", fill: "fill-faint", dash: "2 4", flow: false };
  }
  if (edge.provenance === "allowed") {
    // A NetworkPolicy permitting it: weaker still than a host in a config
    // file, because it says only that the call would be let through if made.
    // The same faint ink as declared and a longer dash, so the two read as
    // kin — neither is traffic — but not as the same thing.
    return { stroke: "stroke-faint", fill: "fill-faint", dash: "7 4", flow: false };
  }
  return { stroke: strokeFor(edge.health), fill: fillFor(edge.health), flow: false };
}

/** How far a node outside the trace fades. Legible, but plainly not the subject. */
const DIM = 0.13;

function Canvas({
  layout,
  selectedId,
  trace,
  onSelect,
}: {
  layout: TopologyLayout;
  selectedId: string | null;
  trace: Trace | null;
  onSelect: (id: string | null) => void;
}) {
  const frame = useRef<HTMLDivElement>(null);
  const layer = useRef<HTMLDivElement>(null);
  const grid = useRef<SVGPatternElement>(null);
  const view = useRef<Transform>({ k: 1, tx: 0, ty: 0 });
  const readout = useRef<HTMLSpanElement>(null);

  /**
   * The view is a CSS transform on a wrapper, written by hand, not React
   * state on an SVG group.
   *
   * It was the latter, and on a few hundred nodes every wheel tick and every
   * pointer move re-rendered the component and then had the browser
   * re-rasterise every path and glyph under a new SVG transform. Memoising the
   * children fixed the first half and left the second, which is most of it.
   * A CSS transform on a `will-change` element is applied by the compositor:
   * the graph is rasterised once and panned and scaled as a texture, and
   * nothing is re-rendered or re-drawn until the reader stops. The dot grid
   * behind it gets the same transform on its pattern, one attribute on one
   * element. Even the zoom readout is written by hand, so a gesture involves
   * no React at all.
   *
   * The layer also carries a level of detail. Zoomed out, a card's small
   * print is unreadable anyway, and hundreds of glyphs that nobody can read
   * still cost the same to rasterise as ones they can — the kit's stylesheet
   * hides them below 70% and the names too below 40%, which is most of what
   * re-rasterising a zoom step used to cost.
   */
  const apply = useCallback((next: Transform) => {
    view.current = next;
    if (layer.current) {
      layer.current.style.transform = `translate(${next.tx}px, ${next.ty}px) scale(${next.k})`;
      layer.current.dataset.lod = next.k < 0.4 ? "far" : next.k < 0.7 ? "mid" : "near";
    }
    grid.current?.setAttribute(
      "patternTransform",
      `translate(${next.tx} ${next.ty}) scale(${next.k})`,
    );
    if (readout.current) readout.current.textContent = `${Math.round(next.k * 100)}%`;
  }, []);

  const fitToFrame = useCallback(() => {
    const box = frame.current?.getBoundingClientRect();
    apply(fitTransform(layout, { width: box?.width ?? 0, height: box?.height ?? 0 }));
  }, [layout, apply]);

  /**
   * Whenever the graph itself changes — a different namespace, a reload that
   * added nodes — the view goes back to showing all of it. Holding a pan
   * across a graph that is no longer the same one leaves the reader looking at
   * empty space and wondering where the cluster went.
   *
   * **Waits for the pane to have a size.** Measuring it in the layout effect
   * and fitting to whatever came back was the first version, and it silently
   * did nothing: the pane reads as zero by zero at that moment, `fitTransform`
   * answers 1:1 to an unmeasurable frame rather than dividing by zero, and the
   * graph opened at 100% — three times the width of the pane it was supposed
   * to fit inside. The Fit button worked, which is exactly why this survived
   * being looked at: the only broken case was the one nobody clicks.
   *
   * A resize observer answers both halves — it reports the first real size and
   * every later one — and it is disconnected on the first fit so that a reader
   * who has since panned is not yanked back when the window is dragged.
   */
  useLayoutEffect(() => {
    const node = frame.current;
    if (!node) return;
    let observer: ResizeObserver | undefined;
    const tryFit = () => {
      const box = node.getBoundingClientRect();
      if (!(box.width > 0) || !(box.height > 0)) return;
      apply(fitTransform(layout, box));
      observer?.disconnect();
    };
    tryFit();
    // `ResizeObserver` is not in every environment this renders in; without it
    // the direct measurement above is all there is, which is what the Fit
    // button is for.
    if (typeof ResizeObserver === "function") {
      observer = new ResizeObserver(tryFit);
      observer.observe(node);
    }
    return () => observer?.disconnect();
  }, [layout, apply]);

  /**
   * Wheel to zoom, bound by hand rather than through `onWheel`.
   *
   * React attaches its wheel listener passively at the root, so a handler
   * given as a prop cannot call `preventDefault` — and without that the page
   * behind the canvas scrolls at the same time as the canvas zooms.
   */
  useEffect(() => {
    const node = frame.current;
    if (!node) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const box = node.getBoundingClientRect();
      const factor = Math.exp(-event.deltaY * 0.0015);
      apply(zoomAt(view.current, factor, event.clientX - box.left, event.clientY - box.top));
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return () => node.removeEventListener("wheel", onWheel);
  }, [apply]);

  /**
   * Drag to pan.
   *
   * `moved` is the whole reason this is a ref: a drag that starts on a node
   * would otherwise also select it on release, so the node's own click asks
   * this first and stands down when the pointer actually travelled.
   */
  const drag = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const startPan = (event: React.PointerEvent) => {
    if (event.button !== 0) return;
    drag.current = { x: event.clientX, y: event.clientY, moved: false };
  };
  useEffect(() => {
    const onMove = (event: PointerEvent) => {
      const at = drag.current;
      if (!at) return;
      const dx = event.clientX - at.x;
      const dy = event.clientY - at.y;
      if (!at.moved && Math.hypot(dx, dy) < 4) return;
      at.moved = true;
      at.x = event.clientX;
      at.y = event.clientY;
      apply({ ...view.current, tx: view.current.tx + dx, ty: view.current.ty + dy });
    };
    const onUp = () => {
      window.setTimeout(() => {
        drag.current = null;
      }, 0);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [apply]);

  const dragged = () => drag.current?.moved === true;

  const inTrace = (id: string) => trace === null || trace.nodes.has(id);
  const multi = layout.namespaces.length > 1;

  /**
   * What a real namespace taught about drawing everything.
   *
   * Four external hosts each named in forty tiers' configuration made a
   * hundred and sixty long dashed curves — unreadable, and most of the paint
   * on every frame. A call into a hub is drawn only when a selection puts it
   * on a path; the hub itself says how many call it. And past a few dozen
   * measured edges the flow animation stops: hundreds of paths each
   * re-rasterising every frame was the rest of the lag.
   */
  const hubs = useMemo(() => hubCounts(layout.edges), [layout.edges]);
  const animate = useMemo(
    () => layout.edges.filter((e) => e.provenance === "observed").length <= FLOW_ANIMATION_LIMIT,
    [layout.edges],
  );
  const edgeVisible = (edge: PlacedEdge) =>
    edge.kind !== "calls" || !hubs.has(edge.to) || (trace !== null && trace.edges.has(edge.key));
  const edgeShown = (edge: PlacedEdge) => trace === null || trace.edges.has(edge.key);

  // Stable, so a memoised node is not re-rendered on every pan for a new
  // closure — which is what made a graph of a few hundred nodes lag under
  // the pointer. The drag ref is read at click time, not captured.
  const select = useCallback(
    (id: string) => {
      if (drag.current?.moved) return;
      onSelect(id);
    },
    [onSelect],
  );

  return (
    <div
      ref={frame}
      className="relative h-full w-full touch-none overflow-hidden"
      onPointerDown={startPan}
      /**
       * Clicking the ground lets a selection go.
       *
       * On the frame, and asking what was actually hit, rather than on a
       * backdrop rectangle behind the graph — which was the first version and
       * only covered the graph's own bounding box, so the empty canvas around
       * it, which is most of the pane on a small namespace, did nothing at
       * all. A click that landed on a node, or on the zoom controls, is
       * theirs; so is one at the end of a drag.
       */
      onClick={(event) => {
        if (dragged()) return;
        if ((event.target as Element).closest("[role='button'],button")) return;
        onSelect(null);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") onSelect(null);
      }}
    >
      {/* The ground, in screen space and behind everything: something for
          the cards to sit ON. The canvas was the same white as the cards,
          which is most of why the picture read as a diagram printed on a
          page rather than as objects on a surface. */}
      <svg className="absolute inset-0 h-full w-full" aria-hidden="true">
        <defs>
          {/* The grid is a screen-space fill whose PATTERN carries the view
              transform. One rectangle covers a pane of any size at any zoom,
              where a tiled shape inside the graph would have to be sized to
              it and would run out at the edges of a pan. */}
          <pattern
            ref={grid}
            id="topo-grid"
            width={24}
            height={24}
            patternUnits="userSpaceOnUse"
          >
            <circle cx={1} cy={1} r={1} className="fill-rule-strong" opacity={0.55} />
          </pattern>
        </defs>
        <rect width="100%" height="100%" className="fill-canvas" />
        <rect width="100%" height="100%" fill="url(#topo-grid)" />
      </svg>
      {/* The graph, drawn once at 1:1 and moved as a texture — see `apply`. */}
      <div
        ref={layer}
        className="absolute top-0 left-0 origin-top-left will-change-transform"
        style={{ width: layout.width, height: layout.height }}
      >
        <svg
          role="group"
          aria-label="Namespace topology"
          className="block select-none"
          width={layout.width}
          height={layout.height}
        >
          {layout.columns.map((column) => (
            <g key={column.rank}>
              {column.rank > 0 && (
                // A hairline between hops. The columns are the argument this
                // layout makes, and unmarked they read as an accident of
                // spacing.
                <line
                  x1={column.x - PADDING}
                  y1={PADDING}
                  x2={column.x - PADDING}
                  y2={layout.height - PADDING}
                  className="stroke-rule"
                  strokeDasharray="2 6"
                  opacity={0.7}
                />
              )}
              <text x={column.x} y={PADDING + 10} className="lane">
                {column.label}
              </text>
            </g>
          ))}
          {layout.band && (
            // The inventory under the flow: every tier no known call touches.
            // Headed like a column, because to a reader it is the answer to
            // the same question — "where does this stand?" — and the answer
            // is "nowhere anyone has measured".
            <g>
              {layout.columns.length > 0 && (
                <line
                  x1={PADDING}
                  y1={layout.band.y - 22}
                  x2={layout.width - PADDING}
                  y2={layout.band.y - 22}
                  className="stroke-rule-strong"
                />
              )}
              <text x={PADDING} y={layout.band.y + 10} className="lane">
                {layout.band.label}
              </text>
            </g>
          )}
          {/* One lane per namespace when several are drawn: its name above
              its first row, and a rule between it and the next. The hop
              columns run through every lane, so a reader can still read
              depth straight down the page. */}
          {layout.lanes.map((lane, i) => (
            <g key={lane.namespace || "outside"}>
              {i > 0 && (
                <line
                  x1={PADDING}
                  y1={lane.y - LANE_LABEL - 16}
                  x2={layout.width - PADDING}
                  y2={lane.y - LANE_LABEL - 16}
                  className="stroke-rule-strong"
                />
              )}
              <text x={PADDING} y={lane.y - 9} className="lane fill-soft" data-lane={lane.namespace}>
                {lane.namespace || "outside the cluster"}
              </text>
            </g>
          ))}
          {/* One panel per tier — the address and the pods that answer it,
              which now share a column and have to read as one thing. Behind
              everything, and quiet: it groups, it does not decorate. */}
          {layout.tiers.map((tier) => (
            <rect
              key={`${tier.x},${tier.y}`}
              x={tier.x}
              y={tier.y}
              width={tier.width}
              height={tier.height}
              rx={12}
              className="fill-surface stroke-rule"
              opacity={0.5}
            />
          ))}
          {/* Edges first, so a node always sits on top of the lines into it. */}
          {layout.edges.map((edge) => (
            <Edge
              key={edge.key}
              edge={edge}
              visible={edgeVisible(edge)}
              shown={edgeShown(edge)}
              animate={animate}
            />
          ))}
          {layout.nodes.map((node) => (
            <Node
              key={node.id}
              node={node}
              selected={node.id === selectedId}
              dimmed={!inTrace(node.id)}
              showNamespace={multi}
              callers={hubs.get(node.id)}
              onSelect={select}
            />
          ))}
        </svg>
      </div>
      <Zoom
        readout={readout}
        onZoom={(factor) => {
          const box = frame.current?.getBoundingClientRect();
          apply(zoomAt(view.current, factor, (box?.width ?? 0) / 2, (box?.height ?? 0) / 2));
        }}
        onFit={fitToFrame}
      />
      <Legend />
    </div>
  );
}

/**
 * Memoised, as is {@link Node}: a pan changes one transform attribute, and
 * without this React re-reconciled every edge and node on every pointer move.
 */
const Edge = memo(function Edge({
  edge,
  visible,
  shown,
  animate,
}: {
  edge: PlacedEdge;
  visible: boolean;
  shown: boolean;
  animate: boolean;
}) {
  if (!visible) return null;
  const tone = edgeTone(edge);
  const opacity = shown ? 1 : DIM;
  return (
    <g opacity={opacity}>
      <path
        d={edge.path}
        fill="none"
        className={`${tone.stroke}${tone.flow && animate ? " flow" : ""}`}
        strokeWidth={edge.width}
        strokeDasharray={tone.dash}
        strokeLinecap="round"
      />
      {/* Drawn as a polygon rather than an SVG marker, so it takes the same
          theme colour as the line and fades with it. */}
      <polygon points={edge.arrow} className={tone.fill} />
      {edge.crossesNamespace && (
        // A call leaving one namespace for another, marked halfway along.
        // A mark rather than a colour, because colour on an edge already
        // says how it is known, and a cross-namespace call can be any of
        // those.
        <rect
          x={edge.midX - 4}
          y={edge.midY - 4}
          width={8}
          height={8}
          transform={`rotate(45 ${edge.midX} ${edge.midY})`}
          className="fill-info stroke-canvas"
          strokeWidth={1.5}
          data-crossing="true"
        />
      )}
      {edge.detail && (
        // Stroked in the ground colour and painted underneath, so a rate is
        // readable where it crosses its own line — which on a steep curve is
        // most of the time, and a busy edge is drawn five pixels thick.
        <text
          x={edge.labelX}
          y={edge.labelY}
          className="edge-label fill-muted stroke-canvas"
          strokeWidth={3}
          paintOrder="stroke"
        >
          {edge.detail}
        </text>
      )}
    </g>
  );
});

/**
 * What kind of thing a node is, as a mark rather than a column.
 *
 * Small, monochrome and to one side of the name: kind is the least surprising
 * thing about any node on this diagram — a reader knows what a Deployment is —
 * and it earns a glance, not a fifth of the width.
 */
function Glyph({ lane, x, y }: { lane: TopologyLane; x: number; y: number }) {
  const line = { className: "fill-none stroke-faint", strokeWidth: 1.3 } as const;
  if (lane === "route") {
    return (
      <path
        d={`M ${x} ${y + 5} h 7 m -3 -3 l 3 3 l -3 3`}
        strokeLinecap="round"
        strokeLinejoin="round"
        {...line}
      />
    );
  }
  if (lane === "service") {
    return <path d={`M ${x + 5} ${y} l 5 5 l -5 5 l -5 -5 z`} {...line} />;
  }
  if (lane === "external") {
    return <circle cx={x + 5} cy={y + 5} r={4.4} strokeDasharray="2 2" {...line} />;
  }
  if (lane === "replicaset") {
    return <path d={`M ${x} ${y + 2} h 10 M ${x} ${y + 5} h 10 M ${x} ${y + 8} h 10`} {...line} />;
  }
  return <rect x={x + 0.6} y={y + 0.6} width={8.8} height={8.8} rx={2} {...line} />;
}

/**
 * Replicas, one square each.
 *
 * `9/12` is a number a reader has to subtract before it means anything. Twelve
 * squares with three of them hollow is the same fact, read without arithmetic
 * and from further away. Above {@link MAX_PIPS} the squares stop being
 * countable and a proportional bar says the same thing honestly.
 */
function Replicas({ node, x, y }: { node: PlacedNode; x: number; y: number }) {
  const desired = node.desired ?? 0;
  const ready = node.ready ?? 0;
  if (node.desired === null || desired <= 0) return null;
  if (desired > MAX_PIPS) {
    const share = Math.max(0, Math.min(1, ready / desired));
    return (
      <g className="pips">
        <rect x={x} y={y} width={92} height={5} rx={2.5} className="fill-rule" />
        <rect x={x} y={y} width={92 * share} height={5} rx={2.5} className={fillFor(node.health)} />
      </g>
    );
  }
  return (
    <g className="pips">
      {Array.from({ length: desired }, (_, i) => (
        <rect
          key={i}
          x={x + i * 8.5}
          y={y}
          width={6}
          height={5}
          rx={1.5}
          className={i < ready ? fillFor(node.health) : "fill-rule"}
        />
      ))}
    </g>
  );
}

const Node = memo(function Node({
  node,
  selected,
  dimmed,
  showNamespace,
  callers,
  onSelect,
}: {
  node: PlacedNode;
  selected: boolean;
  dimmed: boolean;
  showNamespace: boolean;
  /** Set when this is a hub whose incoming calls are not drawn until asked. */
  callers?: number;
  onSelect: (id: string) => void;
}) {
  // A hub says in words what its hidden edges would have drawn. It shares
  // the metric slot with the detail, and for a workload with the replica
  // squares — so it is cut to what the row has left.
  const metric = [node.detail, callers ? `${callers} callers` : ""].filter(Boolean).join(" · ");
  /**
   * The top-right slot: the namespace when there is more than one drawn,
   * otherwise the current revision.
   *
   * Both were tried on that line at once and the first render settled it —
   * `DEPLOYMENT · REV …` ran straight into `CHECKOUT` and neither could be
   * read. They also do not compete: a graph spanning namespaces needs the
   * namespace to be legible above all else, and a graph inside one has the
   * namespace in its heading already and the space going spare.
   */
  const aside = showNamespace ? node.namespace : (node.revisions[0] ?? "");
  return (
    // Given a role and a tab stop rather than left as decoration: selecting a
    // node is an action, and the graph has to be reachable by keyboard like
    // every other list on a screen.
    <g
      role="button"
      tabIndex={0}
      aria-label={`${node.kind} ${node.name}`}
      aria-pressed={selected}
      onClick={() => onSelect(node.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onSelect(node.id);
        }
      }}
      className="cursor-pointer"
      opacity={dimmed ? DIM : 1}
    >
      {/* The full name, for a pointer and for anything reading the tree — the
          drawn one is cut to the box. */}
      <title>{`${node.kind} ${node.name}${node.detail ? ` · ${node.detail}` : ""}`}</title>
      {/* The card's own shadow, and the whole of what lifts it off the ground.
          An offset rectangle rather than a `feDropShadow`: a filter is a raster
          pass per node, and a namespace draws hundreds. An external host casts
          none — it is not a thing this cluster runs, and it should not sit at
          the same height as the things that are. */}
      {node.lane !== "external" && (
        <rect
          x={node.x}
          y={node.y + 1.5}
          width={NODE_WIDTH}
          height={NODE_HEIGHT}
          rx={8}
          className="fill-rule"
          opacity={0.55}
        />
      )}
      <rect
        x={node.x}
        y={node.y}
        width={NODE_WIDTH}
        height={NODE_HEIGHT}
        rx={8}
        className={`${cardFill(node.health)} ${selected ? "stroke-accent" : strokeFor(node.health)}`}
        strokeWidth={selected ? 2 : 1}
        // A dependency is drawn as a broken outline because it is not a thing
        // this cluster runs — srelens knows its name and nothing else about it.
        strokeDasharray={node.lane === "external" ? "4 3" : undefined}
      />
      {/* A health spine down the leading edge, the full height of the card
          rather than a floating tick inside it — the short version read as a
          detail on the card, and this reads as the card's own edge. */}
      {node.health !== "unknown" && (
        <rect
          x={node.x + 1.5}
          y={node.y + 5}
          width={4}
          height={NODE_HEIGHT - 10}
          rx={2}
          className={fillFor(node.health)}
        />
      )}
      <Glyph lane={node.lane} x={node.x + 14} y={node.y + 12} />
      <text x={node.x + 30} y={node.y + 20} className="node-kind">
        {fit(node.kind, 13)}
      </text>
      {aside && (
        <text x={node.x + NODE_WIDTH - 12} y={node.y + 20} className="node-kind" textAnchor="end">
          {fit(aside, 13)}
        </text>
      )}
      <text x={node.x + 14} y={node.y + 39} className="node-label">
        {fit(node.name)}
      </text>
      <Replicas node={node} x={node.x + 14} y={node.y + 46} />
      {metric && (
        <text x={node.x + NODE_WIDTH - 12} y={node.y + 51} className="node-metric fill-muted">
          {/* The detail shares its row with the replica squares, so it gets
              what they leave — which for a workload is `9/12` and for the
              Ingress and Service that have no replicas is the whole host. */}
          {fit(metric, node.desired === null || node.desired <= 0 ? 22 : 8)}
        </text>
      )}
    </g>
  );
});

/**
 * Zoom, because a real namespace does not fit.
 *
 * The version this replaces drew the graph at 1:1 into a scrolling box, which
 * is fine for the eight nodes of a demo and useless for the two hundred of a
 * production namespace: there was no way to see the shape of the thing, only
 * to scroll around inside it. Fit is the button that matters and is why the
 * canvas opens on it.
 */
function Zoom({
  readout,
  onZoom,
  onFit,
}: {
  /** Written to directly by the canvas on every change of view, so a zoom
   *  gesture renders nothing through React. */
  readout: React.RefObject<HTMLSpanElement | null>;
  onZoom: (factor: number) => void;
  onFit: () => void;
}) {
  return (
    <div className="absolute bottom-3 left-3 flex items-center gap-1 rounded-md border border-rule bg-surface/90 p-1">
      <button
        type="button"
        className="text-btn px-2 py-0.5 text-sm"
        aria-label="Zoom out"
        onClick={() => onZoom(1 / 1.25)}
      >
        −
      </button>
      <span ref={readout} className="num w-10 text-center text-[11px]">
        100%
      </span>
      <button
        type="button"
        className="text-btn px-2 py-0.5 text-sm"
        aria-label="Zoom in"
        onClick={() => onZoom(1.25)}
      >
        +
      </button>
      <button type="button" className="text-btn px-2 py-0.5 text-[11px]" onClick={onFit}>
        Fit
      </button>
    </div>
  );
}

/** One legend row: the mark as it is actually drawn, then what it means. */
function Key({
  dash,
  className,
  width = 1.5,
  flow = false,
  children,
}: {
  dash?: string;
  className?: string;
  width?: number;
  flow?: boolean;
  children: string;
}) {
  return (
    <li className="flex items-center gap-1.5">
      <svg width="22" height="6" aria-hidden="true">
        <path
          d="M 0 3 L 22 3"
          fill="none"
          strokeWidth={width}
          strokeDasharray={dash}
          className={`${className ?? ""}${flow ? " flow" : ""}`}
        />
      </svg>
      {children}
    </li>
  );
}

/**
 * The legend draws its own marks.
 *
 * It used to be five words — "OWNS (DASHED)" — which asks the reader to hold a
 * mapping in their head and check it against the canvas. A sample of the
 * actual stroke costs one small `svg` each and removes the translation step.
 *
 * `Owns` is gone from it because ownership is gone from the canvas: a
 * ReplicaSet is now a chip on the workload that owns it rather than a node of
 * its own, so there is no line left to explain.
 */
function Legend() {
  return (
    <ul className="absolute right-3 bottom-3 flex flex-wrap items-center gap-x-4 gap-y-1.5 rounded-md border border-rule bg-surface/90 px-2.5 py-1.5 text-[10px] uppercase text-faint">
      <Key className="stroke-rule">Routes</Key>
      {/* Named for what it MEANS, not for how it is drawn: a reader has to know
          this edge is config rather than traffic, or the diagram overstates
          itself. */}
      <Key dash="2 4" className="stroke-faint">
        Declared in config
      </Key>
      <Key dash="7 4" className="stroke-faint">
        Allowed by policy
      </Key>
      <Key className="stroke-accent" width={2.5} flow>
        Observed traffic
      </Key>
      <Key dash="6 3" className="stroke-sev" width={2}>
        Failing
      </Key>
      <li className="flex items-center gap-1.5">
        <svg width="22" height="10" aria-hidden="true">
          <path d="M 0 5 L 22 5" fill="none" strokeWidth={1.5} className="stroke-rule" />
          <rect x={7} y={1} width={8} height={8} transform="rotate(45 11 5)" className="fill-info" />
        </svg>
        Crosses a namespace
      </li>
      {/* Said here because it is the one thing the canvas withholds: a hub's
          callers exist, and a reader has to know that selecting draws them. */}
      <li className="text-faint normal-case">Hubs draw their callers on selection</li>
    </ul>
  );
}

/**
 * What one node is, in full.
 *
 * Only the fields the graph actually carries. The design's inspector also
 * shows an image, a revision age and a selector, and none of those are on
 * `TopologyNode` — a row with nothing behind it is the thing this migration
 * keeps catching.
 */
function Inspector({
  node,
  layout,
  trace,
  onSelect,
}: {
  node: PlacedNode | null;
  layout: TopologyLayout;
  trace: Trace | null;
  onSelect: (id: string) => void;
}) {
  // Both directions, named from the reader's point of view rather than the
  // graph's: "what does this reach" and "what reaches this" is the question
  // someone tracing a dependency actually has.
  const reaches = layout.edges.filter((e) => e.from === node?.id);
  const reachedBy = layout.edges.filter((e) => e.to === node?.id);
  // The other end's namespace is said only when it differs — a call across
  // namespaces is the one a reader most needs to place, and on a
  // single-namespace graph the suffix would be on every row.
  const label = (id: string) => {
    const other = layout.nodes.find((n) => n.id === id);
    if (!other) return id;
    return other.namespace && other.namespace !== node?.namespace
      ? `${other.name} · ${other.namespace}`
      : other.name;
  };

  if (!node) {
    return (
      <EmptyState
        compact
        title="No node selected"
        hint="Pick a node to trace what it reaches, and what reaches it."
      />
    );
  }
  return (
    <div>
      <p className="text-[10px] uppercase text-faint">
        {node.kind} · {node.namespace}
      </p>
      <h2 className="mt-1 text-base font-semibold break-all">{node.name}</h2>
      {/* An Ingress, a Service and an external host have no replicas, so a
          health pill on them would read "unknown" forever — a word that looks
          like a finding and is not one. */}
      {node.health !== "unknown" && (
        <div className="mt-2">
          <StatusPill kind={pillKind(node.health)} status={node.health} />
        </div>
      )}
      <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted">Depth</dt>
        <dd>
          {node.rank === null
            ? // Said as a fact about what is KNOWN, not about the node: nothing
              // declared, routed or measured touches it, which is a different
              // thing from it talking to nobody.
              "On no known path"
            : node.rank < layout.entryRank
              ? "Entry point"
              : node.rank === layout.entryRank
                ? layout.entryRank > 0
                  ? "Entry service"
                  : "Entry point"
                : `${node.rank - layout.entryRank} hop${node.rank - layout.entryRank === 1 ? "" : "s"} in`}
        </dd>
        {node.ready !== null && node.desired !== null && (
          <>
            <dt className="text-muted">Ready</dt>
            <dd>
              {node.ready} / {node.desired}
            </dd>
          </>
        )}
        {node.detail && (
          <>
            <dt className="text-muted">Detail</dt>
            <dd>{node.detail}</dd>
          </>
        )}
        {trace && (
          <>
            {/* The question an SRE actually has about a node is not what it
                touches but what goes with it. That is transitive, and it is
                what the trace already computed for the canvas. */}
            <dt className="text-muted">Downstream</dt>
            <dd>{count(trace.downstream.size, "node")}</dd>
            <dt className="text-muted">Upstream</dt>
            <dd>{count(trace.upstream.size, "node")}</dd>
          </>
        )}
      </dl>
      {node.revisions.length > 0 && (
        // Where the folded ReplicaSets went. Newest first, which is the one
        // serving.
        <section className="mt-4">
          <h3 className="text-[10px] uppercase text-faint">Revisions</h3>
          <p className="num mt-1 text-sm text-soft">{node.revisions.join(" · ")}</p>
        </section>
      )}
      {node.lane === "external" && (
        // Said in words, not just by a dotted line: this node exists because
        // something named the host in its configuration. srelens has not seen a
        // byte go to it, and the panel must not imply otherwise.
        <p className="mt-3 text-xs text-faint">
          Named in configuration. srelens has not observed traffic to it.
        </p>
      )}
      <Connections
        title="Reaches"
        edges={reaches}
        other={(e) => e.to}
        label={label}
        onSelect={onSelect}
      />
      <Connections
        title="Reached by"
        edges={reachedBy}
        other={(e) => e.from}
        label={label}
        onSelect={onSelect}
      />
    </div>
  );
}

function count(n: number, noun: string): string {
  return `${n} ${noun}${n === 1 ? "" : "s"}`;
}

/**
 * One side of a node's connections, as buttons rather than text.
 *
 * The panel is the way to walk the graph when the canvas is crowded: reading an
 * edge off a picture full of crossings is hard, and following a list is not.
 * Each row says how the link is known, so a declared dependency is never
 * mistaken here for something measured.
 */
function Connections({
  title,
  edges,
  other,
  label,
  onSelect,
}: {
  title: string;
  edges: PlacedEdge[];
  other: (edge: PlacedEdge) => string;
  label: (id: string) => string;
  onSelect: (id: string) => void;
}) {
  if (edges.length === 0) return null;
  return (
    <section className="mt-4">
      <h3 className="text-[10px] uppercase text-faint">{title}</h3>
      <ul className="mt-1 space-y-0.5">
        {edges.map((edge) => (
          <li key={edge.key}>
            {/* Not the kit's `text-btn`, which is an `inline-flex` of fixed
                height with `nowrap`: it ate the spaces between these three
                parts outright — `payments-apicalls88 rpm` — and a host that
                does not fit on one line has nowhere to go in it. Laid out
                instead, so the name takes the room and the rate keeps to the
                right where it can be read down the column. */}
            <button
              type="button"
              className="flex w-full items-baseline gap-2 rounded px-1.5 py-1 text-left text-sm hover:bg-sunk"
              onClick={() => onSelect(other(edge))}
            >
              <span className="min-w-0 flex-1 break-all">{label(other(edge))}</span>
              <span className="shrink-0 text-[10px] uppercase text-faint">
                {/* How the link is KNOWN, where that is weaker than the edge
                    kind implies: a "call" that is only permitted by policy
                    must not be listed as a call. */}
                {edge.provenance === "declared" || edge.provenance === "allowed"
                  ? edge.provenance
                  : edge.kind}
              </span>
              {edge.detail && (
                <span className="num shrink-0 text-xs text-muted">{edge.detail}</span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
