import { useMemo, useState } from "react";
import {
  topologyGraph,
  type TopologyEdge,
  type TopologyHealth,
  type TopologyNode,
  type ClusterContext,
} from "@srelens/core";
import { EmptyState, Screen, StatusPill, type StatusKind } from "@srelens/ui-kit";
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
  LANE_LABELS,
  NODE_HEIGHT,
  NODE_WIDTH,
  fit,
  layoutGraph,
  type PlacedNode,
  type TopologyLayout,
} from "../lib/topologyLayout";

/**
 * How traffic reaches a workload, across the namespaces a reader picks.
 *
 * Five lanes: route, service, workload, revision, and the dependencies a
 * workload was configured to reach — other Services, and systems outside the
 * cluster entirely.
 *
 * **Every edge carries its provenance, and is drawn differently for it.** The
 * structural half — selectors, ownerReferences, Ingress rules — is the API
 * server's own word. The flow half is read out of configuration: environment
 * variables, arguments and ConfigMaps, which say a workload was BUILT to talk
 * to something, not that it ever has. Those two must not look alike, or a
 * reader trusts them equally; {@link dashFor} is where that is decided.
 *
 * What is still missing is the RATE — the design's `41.2k rpm` and
 * `12.4% 5xx`. That needs measured telemetry (Istio, Linkerd, Hubble, or any
 * Prometheus scraping them), and srelens has no source for it yet:
 * `k8s.podMetrics` is CPU millicores and memory MiB. The `observed`
 * provenance is already defined for it, so that source is an addition rather
 * than a reshaping of this.
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
   * This screen first grew its own `listNamespaces` call and its own `Select`,
   * which made it the one place in the app where the namespace control looked
   * and behaved differently. Not `useNamespaces` either: that store holds the
   * reader's standing FILTER, and reading it as the list of namespaces gave a
   * picker with no options on a perfectly healthy cluster.
   */
  const { namespaces, error: namespaceError } = useNamespaceOptions(
    context.name,
    getKubeconfigFiles(),
  );

  /**
   * The reader's namespace selection — the workspace's, not this screen's.
   *
   * One selection per cluster, shared by every screen looking at it, which is
   * what that store is for: narrowing to `payments` here narrows the resource
   * lists and the events screen too, and arriving from one of those lands on
   * what they were already looking at. A picker with private state would have
   * made this screen the one place the choice did not travel.
   */
  const scoped = useNamespaces(cluster?.stableId);

  /**
   * Which namespaces are drawn.
   *
   * Several at once, because a dependency rarely respects a namespace boundary:
   * a `checkout` that calls `payments-api.payments.svc` is only half a picture
   * with `payments` left out, and the design's own header reads
   * `CHECKOUT · PAYMENTS · IDENTITY`.
   *
   * An empty selection is the picker's "All namespaces", honoured up to
   * {@link NAMESPACE_LIMIT}. Nothing is written back for it: persisting a
   * guess would silently narrow every other screen on behalf of a reader who
   * chose nothing.
   */
  const all = namespaces ?? [];
  const chosen = scoped.length > 0 ? scoped : all.slice(0, NAMESPACE_LIMIT);
  const cut = scoped.length === 0 && all.length > NAMESPACE_LIMIT;
  // Sorted and joined for the dependency array, so re-picking the same set in a
  // different order does not re-read the cluster.
  const key = [...chosen].sort().join(",");

  const graph = useResource(
    async () => {
      if (!cluster || chosen.length === 0) return { nodes: [], edges: [] };
      const out = await topologyGraph(cluster.name, chosen);
      if (out.error) throw new Error(out.error);
      return out.graph ?? { nodes: [], edges: [] };
    },
    [cluster?.name, key],
    (g) => g.nodes.length === 0,
  );

  const layout = useMemo(() => layoutGraph(graph.data ?? { nodes: [], edges: [] }), [graph.data]);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Found rather than held, so the panel follows a reload instead of showing
  // the node as it was when it was clicked.
  const selected = layout.nodes.find((n) => n.id === selectedId) ?? null;

  return (
    <Screen
      title="Topology"
      eyebrow={cluster?.name}
      actions={
        // Inherited, not grown here: the same control and the same selection
        // every list on this cluster uses, so narrowing in one place narrows
        // the rest.
        <NamespacePicker
          namespaces={namespaces}
          selection={scoped}
          onChange={(next) => {
            setNamespaces(context.stableId, next);
            // The old selection may name a node in a namespace no longer drawn.
            setSelectedId(null);
          }}
        />
      }
      fill
    >
      <div className="flex h-full min-h-0">
        <div className="scroll min-w-0 flex-1 p-4">
          {/* Both the app's, so a namespace listing that failed and a selection
              naming namespaces the cluster no longer has read the same here as
              they do on every list. */}
          <NamespaceErrorAlert error={namespaceError} />
          <StaleSelectionAlert
            selection={scoped}
            namespaces={namespaces}
            onReset={() => setNamespaces(context.stableId, [])}
          />
          {cut && (
            // Said rather than done quietly: the picker reads "All namespaces"
            // and this is not all of them.
            <p className="mb-3 text-sm text-muted">
              Showing {NAMESPACE_LIMIT} of {all.length} namespaces. Pick the ones you want to see.
            </p>
          )}
          <div className="grid min-h-0 place-items-center">
            <>
              {graph.status === "loading" && (
                <p className="text-sm text-muted">Reading the cluster…</p>
              )}
              {graph.status === "error" && (
                <FailureAlert title="Could not draw this topology" error={graph.error} />
              )}
              {graph.status === "empty" && (
                <EmptyState
                  title={chosen.length === 1 ? `Nothing to draw in ${chosen[0]}` : "Nothing to draw here"}
                  hint={
                    chosen.length === 1
                      ? "This namespace has no ingresses, services or workloads."
                      : "None of these namespaces has an ingress, service or workload."
                  }
                />
              )}
              {graph.status === "ready" && (
                <Canvas layout={layout} selectedId={selectedId} onSelect={setSelectedId} />
              )}
            </>
          </div>
        </div>
        <aside className="rule-l scroll w-[280px] shrink-0 p-4" aria-label="Selected node">
          <Inspector node={selected} layout={layout} onSelect={setSelectedId} />
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
  return "stroke-rule";
}

function fillFor(health: TopologyHealth): string {
  if (health === "ok") return "fill-ok";
  if (health === "degraded") return "fill-warn";
  if (health === "failing") return "fill-sev";
  return "fill-rule";
}

function Canvas({
  layout,
  selectedId,
  onSelect,
}: {
  layout: TopologyLayout;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  /**
   * Everything one hop from the selection, itself included.
   *
   * A namespace of any size draws edges that cross, and no amount of ordering
   * removes them all. Rather than fight that, selecting a node fades everything
   * it does not touch — which turns "trace this line through the tangle" into
   * "read the two things still bright". Empty when nothing is selected, and
   * then nothing is faded.
   */
  const focus = useMemo(() => {
    if (!selectedId) return null;
    const near = new Set<string>([selectedId]);
    for (const edge of layout.edges) {
      if (edge.from === selectedId) near.add(edge.to);
      if (edge.to === selectedId) near.add(edge.from);
    }
    return near;
  }, [selectedId, layout.edges]);

  const dimmed = (id: string) => focus !== null && !focus.has(id);

  return (
    <div>
      <svg
        role="img"
        aria-label="Namespace topology"
        width={layout.width}
        height={layout.height}
        viewBox={`0 0 ${layout.width} ${layout.height}`}
        className="max-w-none"
      >
        {layout.lanes.map((lane) => (
          <text key={lane.lane} x={lane.x} y={12} className="fill-faint text-[10px] uppercase tracking-wide">
            {lane.label}
          </text>
        ))}
        {/* Edges first, so a node always sits on top of the lines into it. */}
        {layout.edges.map((edge) => {
          const related = focus === null || edge.from === selectedId || edge.to === selectedId;
          return (
            <path
              key={`${edge.kind}:${edge.from}->${edge.to}`}
              d={edge.path}
              fill="none"
              className={edge.provenance === "declared" ? "stroke-faint" : strokeFor(edge.health)}
              strokeWidth={edge.health === "failing" ? 2 : 1}
              strokeDasharray={dashFor(edge)}
              opacity={related ? 1 : 0.12}
            />
          );
        })}
        {layout.nodes.map((node) => (
          <Node
            key={node.id}
            node={node}
            selected={node.id === selectedId}
            dimmed={dimmed(node.id)}
            onSelect={onSelect}
          />
        ))}
      </svg>
      <Legend />
    </div>
  );
}

function Node({
  node,
  selected,
  dimmed,
  onSelect,
}: {
  node: PlacedNode;
  selected: boolean;
  dimmed: boolean;
  onSelect: (id: string) => void;
}) {
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
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(node.id);
        }
      }}
      className="cursor-pointer"
      opacity={dimmed ? 0.25 : 1}
    >
      {/* The full name, for a pointer and for anything reading the tree — the
          drawn one is cut to the box. */}
      <title>{`${node.kind} ${node.name}${node.detail ? ` · ${node.detail}` : ""}`}</title>
      <rect
        x={node.x}
        y={node.y}
        width={NODE_WIDTH}
        height={NODE_HEIGHT}
        rx={7}
        className={`fill-surface ${selected ? "stroke-accent" : strokeFor(node.health)}`}
        strokeWidth={selected ? 2 : 1}
      />
      {/* A health bar down the leading edge. The border alone carried this and
          a 1px outline is not enough to find a failing node by at a glance. */}
      {node.health !== "unknown" && (
        <rect
          x={node.x}
          y={node.y + 8}
          width={3}
          height={NODE_HEIGHT - 16}
          rx={1.5}
          className={fillFor(node.health)}
        />
      )}
      <text x={node.x + 12} y={node.y + 17} className="fill-faint text-[9px] uppercase">
        {node.kind}
      </text>
      <text x={node.x + 12} y={node.y + 32} className="fill-ink text-[12px]">
        {fit(node.name)}
      </text>
      {node.detail && (
        <text x={node.x + 12} y={node.y + 45} className="fill-muted text-[10px]">
          {fit(node.detail, 24)}
        </text>
      )}
    </g>
  );
}

/**
 * How an edge is drawn, which encodes HOW IT IS KNOWN rather than what it says.
 *
 * Solid is the API server's own word — a selector, an ownerReference, an
 * Ingress rule. Long-dashed is ownership, which is a fact about who made a
 * thing rather than a path traffic takes. Dotted is `declared`: a host named in
 * configuration, meaning this workload was built to reach that, not that it
 * ever has. When telemetry lands, `observed` takes the solid line and declared
 * stays dotted beneath it, so a reader can always tell a measurement from a
 * string in an environment variable.
 */
function dashFor(edge: { kind: string; provenance: string }): string | undefined {
  if (edge.provenance === "declared") return "2 4";
  if (edge.kind === "owns") return "4 3";
  return undefined;
}

/** One legend row: the mark as it is actually drawn, then what it means. */
function Key({ dash, className, children }: { dash?: string; className?: string; children: string }) {
  return (
    <li className="flex items-center gap-1.5">
      <svg width="22" height="6" aria-hidden="true">
        <path d="M 0 3 L 22 3" fill="none" strokeWidth={1.5} strokeDasharray={dash} className={className} />
      </svg>
      {children}
    </li>
  );
}

/**
 * The legend draws its own marks.
 *
 * It used to be five words — "OWNS (DASHED)" — which asks the reader to hold a
 * mapping in their head and check it against the canvas. A sample of the actual
 * stroke costs one small `svg` each and removes the translation step.
 */
function Legend() {
  return (
    <ul className="mt-4 flex flex-wrap items-center gap-x-5 gap-y-2 text-[10px] uppercase text-faint">
      <Key className="stroke-ok">Routes</Key>
      <Key dash="4 3" className="stroke-rule">
        Owns
      </Key>
      {/* Named for what it MEANS, not for how it is drawn: a reader has to know
          this edge is config rather than traffic, or the diagram overstates
          itself. */}
      <Key dash="2 4" className="stroke-faint">
        Declared in config
      </Key>
      <Key className="stroke-warn">Degraded</Key>
      <Key className="stroke-sev">Failing</Key>
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
  onSelect,
}: {
  node: TopologyNode | null;
  layout: TopologyLayout;
  onSelect: (id: string) => void;
}) {
  // Both directions, named from the reader's point of view rather than the
  // graph's: "what does this reach" and "what reaches this" is the question
  // someone tracing a dependency actually has.
  const reaches = layout.edges.filter((e) => e.from === node?.id);
  const reachedBy = layout.edges.filter((e) => e.to === node?.id);
  const label = (id: string) => layout.nodes.find((n) => n.id === id)?.name ?? id;

  if (!node) {
    return <EmptyState compact title="No node selected" hint="Pick a node to see what it is." />;
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
        <dt className="text-muted">Lane</dt>
        <dd>{LANE_LABELS[node.lane]}</dd>
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
      </dl>
      {node.lane === "external" && (
        // Said in words, not just by a dotted line: this node exists because
        // something named the host in its configuration. srelens has not seen
        // a byte go to it, and the panel must not imply otherwise.
        <p className="mt-3 text-xs text-faint">
          Named in configuration. srelens has not observed traffic to it.
        </p>
      )}
      <Connections title="Reaches" edges={reaches} other={(e) => e.to} label={label} onSelect={onSelect} />
      <Connections title="Reached by" edges={reachedBy} other={(e) => e.from} label={label} onSelect={onSelect} />
    </div>
  );
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
  edges: TopologyEdge[];
  other: (edge: TopologyEdge) => string;
  label: (id: string) => string;
  onSelect: (id: string) => void;
}) {
  if (edges.length === 0) return null;
  return (
    <section className="mt-4">
      <h3 className="text-[10px] uppercase text-faint">{title}</h3>
      <ul className="mt-1 space-y-0.5">
        {edges.map((edge) => (
          <li key={`${edge.kind}:${edge.from}->${edge.to}`}>
            <button
              type="button"
              className="text-btn w-full text-left text-sm"
              onClick={() => onSelect(other(edge))}
            >
              <span className="break-all">{label(other(edge))}</span>{" "}
              <span className="text-faint">
                {edge.provenance === "declared" ? "declared" : edge.kind}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
