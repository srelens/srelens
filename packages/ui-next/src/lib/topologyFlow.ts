import type { TopologyEdge, TopologyGraph, TopologyNode } from "@srelens/core";

/**
 * Where every node and edge of a topology graph goes, decided once and without
 * touching the DOM.
 *
 * **The x-axis is distance from where traffic enters, not the kind of the
 * object.** That is the whole of this module's argument with the one it
 * replaces, which gave each Kubernetes kind a fixed column — route, service,
 * workload, replicaset, dependency. That reads as a taxonomy and draws a call
 * chain as a wreck: `checkout` calling `payments` means a Deployment in column
 * three pointing at a Service in column two, so every single cross-service
 * call — the exact thing a topology screen exists to show — came out as a
 * backward line bowing under the diagram. Three hops deep and the picture was
 * a pile of bows.
 *
 * Ranking by flow puts `ingress -> checkout-svc -> checkout -> payments-svc ->
 * payments` in five columns pointing one way, which is what it is. Kind stops
 * being a position and becomes a glyph, which it should always have been:
 * nobody needs a column heading to know a Deployment from an Ingress.
 *
 * Separated from the screen for the same reason the joins live in Rust rather
 * than here: placement is the part that can be wrong, and `checkout-api is two
 * hops behind the ingress` is far easier to assert against a returned array
 * than against an SVG.
 */

export const NODE_WIDTH = 184;
export const NODE_HEIGHT = 60;
/** Between columns. Wide enough for a curve to read as one, and for a rate to
 *  be written along it without touching either end. */
export const COLUMN_GAP = 104;
/** Between two rows of the same tier — the Service and the pods answering it.
 *  Tight, because they are one thing. */
export const ROW_GAP = 10;
/** Between one tier and the next down the same column. Wide, for the same
 *  reason the other is tight. */
export const TIER_GAP = 34;
/** How far a tier's panel stands off the boxes inside it. */
export const TIER_PAD = 9;
/** Between grid columns in the band. No edges run there, so no room for any. */
export const GRID_GAP = 28;
/** Between a tier's Service lane and its workload lane — just room for the
 *  short arrow from one to the other. */
export const INNER_GAP = 44;
/** Between the bottom of the flow and the band's heading. */
export const BAND_GAP = 64;
/** Room above a namespace lane's first row for its name. */
export const LANE_LABEL = 26;
/** Between one namespace lane and the next, divider included. */
export const LANE_GAP = 58;
/** How far under a lane's last row the detour channel runs, and how much
 *  room it keeps below itself. */
export const CHANNEL_DROP = 24;
export const CHANNEL_CLEARANCE = 20;
/** Room above the first row for the column headings. */
export const HEADER_HEIGHT = 30;
/** Room under the last row for the edges that still have to bow below it. */
export const BACKWARD_CLEARANCE = 88;
/** Margin on every side, so an arrowhead or a bow is never against the edge of
 *  the box and clipped by the viewBox. */
export const PADDING = 24;

/** Thinnest and thickest a line gets. Volume is drawn between these. */
export const MIN_EDGE_WIDTH = 1;
export const MAX_EDGE_WIDTH = 5;

export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 2.5;

/** How many replicas are worth drawing one square each. */
export const MAX_PIPS = 12;

/**
 * A node with the ReplicaSets that belong to it folded in.
 *
 * See {@link fold} for why they are not drawn as nodes of their own.
 */
export interface FlowNode extends TopologyNode {
  /** Revisions of this workload, newest first. Empty for everything else. */
  revisions: string[];
}

/**
 * How an edge leaves a node that has other tiers standing to its right.
 *
 * ENTRY's calling tiers stand side by side, and a curve drawn straight out
 * of an inner one runs through its neighbour. So it goes down instead, along
 * a channel under the block at `y`, and only turns up towards its target once
 * it is past `until` — the right edge of the last tier in the way.
 */
export interface Detour {
  y: number;
  until: number;
}

export interface PlacedNode extends FlowNode {
  x: number;
  y: number;
  /** Hops from where traffic enters. Zero is an entry point; `null` is a node
   *  on no known path at all, drawn in the band below the flow. */
  rank: number | null;
  /** Set on a node whose outgoing edges must go round rather than through. */
  detour?: Detour;
}

export interface PlacedEdge extends TopologyEdge {
  /** Stable across renders, and what the trace highlight keys on. */
  key: string;
  /** An SVG cubic from the source to the target. */
  path: string;
  /** The arrowhead at the target end, as a polygon's `points`. */
  arrow: string;
  /** How thick to draw it — volume, where anything measured volume. */
  width: number;
  /** Where the edge's own label goes, for the edges that have one. */
  labelX: number;
  labelY: number;
  /** A point on the line halfway along, for a marker to sit on. */
  midX: number;
  midY: number;
  /** Whether the two ends are in different namespaces — a call leaving one
   *  team's territory for another's, which is worth a mark of its own. A
   *  call to something outside the cluster is not this. */
  crossesNamespace: boolean;
}

/**
 * One namespace's horizontal lane through the flow.
 *
 * Empty when only one namespace is drawn: the namespace is the heading then,
 * and a lane round everything would be a box drawn round the page.
 */
export interface NamespaceLane {
  namespace: string;
  y: number;
  height: number;
}

export interface Column {
  rank: number;
  /** `ENTRY`, then `HOP 1`. Teaching what the axis means is the point. */
  label: string;
  x: number;
  count: number;
}

/**
 * The panel drawn behind one tier — a Service and the pods answering it, or an
 * Ingress and both.
 *
 * Only for a tier of more than one. A panel round a single node is that node's
 * own box drawn twice, and the tier idea is worth nothing where there is
 * nothing to group.
 */
export interface Tier {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * The heading over the band of tiers that are on no known path.
 *
 * `null` when every tier is on one — which, without a mesh, a metrics
 * backend or the connection probe, is almost never.
 */
export interface Band {
  y: number;
  label: string;
  count: number;
}

export interface TopologyLayout {
  nodes: PlacedNode[];
  edges: PlacedEdge[];
  columns: Column[];
  tiers: Tier[];
  lanes: NamespaceLane[];
  band: Band | null;
  /** Which column the entry tiers stand in — one behind an Ingress column,
   *  zero when the picture has no Ingress. */
  entryRank: number;
  /** Every namespace drawn, sorted. The screen labels nodes with theirs only
   *  when there is more than one, because on a single-namespace graph the
   *  namespace is the title and repeating it on every box is noise. */
  namespaces: string[];
  width: number;
  height: number;
}

/** Within a tier, top to bottom: the way in, then the address, then what
 *  answers, then anything hanging off it. */
const LANE_ORDER: Record<string, number> = {
  route: 0,
  service: 1,
  workload: 2,
  replicaset: 3,
  external: 4,
};

/** Two decimal places, so the same graph produces byte-identical paths. */
function r(n: number): number {
  return Math.round(n * 100) / 100;
}

/**
 * Fold the ReplicaSet lane into the workloads that own it.
 *
 * A ReplicaSet is not a place traffic goes. It is how a Deployment records
 * that it rolled, and giving it a node put a dead-end box on the diagram for
 * every workload — on a busy namespace that was half the nodes, none of them
 * on any path. Folded, the revisions are still there: a chip on the workload
 * and the full list in the inspector.
 *
 * A ReplicaSet with no owning Deployment is kept as a node. That one is a real
 * finding — something applied a bare ReplicaSet, or its owner is gone — and
 * folding it into nothing would delete it.
 */
export function fold(graph: TopologyGraph): { nodes: FlowNode[]; edges: TopologyEdge[] } {
  const owner = new Map<string, string>();
  for (const edge of graph.edges) {
    if (edge.kind === "owns") owner.set(edge.to, edge.from);
  }

  const drawn = new Set(graph.nodes.map((n) => n.id));
  const folded = new Set<string>();
  const revisions = new Map<string, string[]>();
  for (const node of graph.nodes) {
    if (node.lane !== "replicaset") continue;
    const into = owner.get(node.id);
    // An owner that is not itself drawn — a Deployment deleted while its
    // ReplicaSet lingers still names it — has nowhere to take the revision,
    // so the ReplicaSet stays a node rather than folding into nothing.
    if (into === undefined || !drawn.has(into)) continue;
    folded.add(node.id);
    const list = revisions.get(into);
    if (list) list.push(node.name);
    else revisions.set(into, [node.name]);
  }

  const nodes = graph.nodes
    .filter((node) => !folded.has(node.id))
    .map((node) => ({ ...node, revisions: byRevision(revisions.get(node.id) ?? []) }));

  const kept = new Set(nodes.map((n) => n.id));
  const edges = graph.edges.filter(
    (edge) => edge.kind !== "owns" && kept.has(edge.from) && kept.has(edge.to),
  );
  return { nodes, edges };
}

/**
 * Newest revision first.
 *
 * The backend names these `rev 119`, so the number is what orders them — a
 * string sort would put `rev 9` above `rev 119`, which is the wrong end of a
 * rollout to show first.
 */
function byRevision(names: string[]): string[] {
  const number = (name: string) => {
    const found = /(\d+)/.exec(name);
    return found ? Number(found[1]) : -1;
  };
  return [...names].sort((a, b) => number(b) - number(a) || a.localeCompare(b));
}

/**
 * The edges that would make the graph cyclic, found by a depth-first walk.
 *
 * Meshes have cycles — two services that call each other, and a retry path
 * back to a gateway — and a longest-path ranking over a cycle does not
 * terminate. These are dropped for the purpose of RANKING only: they are still
 * drawn, as the backward bows this layout otherwise almost never needs.
 *
 * Iterative rather than recursive, because the recursion depth here is the
 * length of a call chain across every namespace in view and a blown stack
 * would take the tab down.
 */
export function backEdges(ids: string[], out: Map<string, string[]>): Set<string> {
  const OPEN = 1;
  const DONE = 2;
  const colour = new Map<string, number>();
  const back = new Set<string>();

  for (const root of ids) {
    if (colour.get(root) !== undefined) continue;
    colour.set(root, OPEN);
    const stack: { id: string; next: number }[] = [{ id: root, next: 0 }];
    while (stack.length > 0) {
      const top = stack[stack.length - 1];
      const children = out.get(top.id) ?? [];
      if (top.next >= children.length) {
        colour.set(top.id, DONE);
        stack.pop();
        continue;
      }
      const child = children[top.next++];
      const seen = colour.get(child);
      // Still open means it is an ancestor of this node on the current walk,
      // which is exactly what a cycle is.
      if (seen === OPEN) back.add(`${top.id}->${child}`);
      if (seen !== undefined) continue;
      colour.set(child, OPEN);
      stack.push({ id: child, next: 0 });
    }
  }
  return back;
}

/**
 * How many hops each node is from where traffic enters.
 *
 * Longest path rather than shortest, deliberately: when a Service is reached
 * both straight off the ingress and again at the end of a four-hop chain, it
 * belongs at the far end. Shortest-path ranking would pull it to the front and
 * draw the long chain backwards over everything — the same failure this
 * layout exists to fix, arrived at from the other side.
 *
 * A node nothing points at is rank zero. On most namespaces that is where the
 * picture starts: an Ingress, or, far more often, a Service that something
 * outside the cluster reaches without an Ingress object to say so.
 */
export function rankNodes(nodes: FlowNode[], edges: TopologyEdge[]): Map<string, number> {
  // Sorted everywhere a set is turned into an order, so the same graph ranks
  // identically every render. A picture that rearranged itself under a reader
  // mid-trace would be worse than a slightly old one.
  const ids = nodes.map((n) => n.id).sort();
  const present = new Set(ids);

  /**
   * Tiers are contracted BEFORE anything is ranked.
   *
   * Ranking nodes with `routes` costing zero was the first version, and a
   * real namespace broke it: a StatefulSet named its own headless Service in
   * its config, so the Service was called (rank one) while the StatefulSet
   * it routes to sat at rank zero beside the other Service fronting it. One
   * tier drawn across two columns, joined by a bow. Every `routes` edge is a
   * statement that its two ends are one tier, so the tier is the unit that
   * ranks, a call from a tier to itself is not a hop, and no ordering of
   * cycle-breaking can split what routing joined.
   */
  /**
   * An Ingress is not part of the tier it routes to. It is the way in from
   * outside, and it gets a column of its own, first — a reader asked for
   * exactly that, having seen ten `login.*` Ingresses drawn as the top of a
   * Service's tier. So only Service-to-pods routing joins a tier; an Ingress
   * stands alone, and the tiers it fronts are the entries.
   */
  const laneOf = new Map(nodes.map((n) => [n.id, n.lane]));
  const routed: [string, string][] = edges
    .filter(
      (e) =>
        e.kind === "routes" &&
        present.has(e.from) &&
        present.has(e.to) &&
        laneOf.get(e.from) !== "route",
    )
    .map((e) => [e.from, e.to]);
  const tierOf = new Map<string, string>();
  for (const group of groupColumn(nodes, routed)) {
    const key = group.map((n) => n.id).sort()[0];
    for (const node of group) tierOf.set(node.id, key);
  }
  const tiers = [...new Set(tierOf.values())].sort();

  // Only a CALL is a hop. Routing is an address being resolved — an Ingress
  // naming a Service, a Service selecting its pods — and none of that is a
  // network call the application makes. Charging it a column each drew the
  // demo namespace eight columns wide and two rows tall.
  const out = new Map<string, string[]>();
  for (const edge of edges) {
    if (edge.kind !== "calls" || !present.has(edge.from) || !present.has(edge.to)) continue;
    const from = tierOf.get(edge.from) as string;
    const to = tierOf.get(edge.to) as string;
    if (from === to) continue;
    const list = out.get(from);
    if (list) {
      if (!list.includes(to)) list.push(to);
    } else {
      out.set(from, [to]);
    }
  }
  for (const list of out.values()) list.sort();

  /**
   * A tier with an Ingress is an entry, whatever else calls it.
   *
   * Ten `login.*` Ingresses fronted an auth Service that half the namespace
   * also called internally, so the tier ranked one and the Ingresses were
   * drawn a hop in from the edge — which is not where traffic enters. Calls
   * INTO an Ingress-fronted tier are left out of the ranking, so the tier
   * stays at zero; they are still drawn, as the backward bows they now are,
   * which is the honest picture of a front door that is also called from
   * inside the house.
   */
  const entries = new Set<string>();
  for (const edge of edges) {
    if (edge.kind === "routes" && laneOf.get(edge.from) === "route" && present.has(edge.to)) {
      entries.add(tierOf.get(edge.to) as string);
    }
  }
  for (const [from, children] of out) {
    out.set(
      from,
      children.filter((to) => !entries.has(to)),
    );
  }

  const back = backEdges(tiers, out);
  const forward = new Map<string, string[]>();
  const incoming = new Map<string, number>(tiers.map((id) => [id, 0]));
  for (const [from, children] of out) {
    const kept = children.filter((to) => !back.has(`${from}->${to}`));
    forward.set(from, kept);
    for (const to of kept) incoming.set(to, (incoming.get(to) ?? 0) + 1);
  }

  const tierRank = new Map<string, number>(tiers.map((id) => [id, 0]));
  const queue = tiers.filter((id) => (incoming.get(id) ?? 0) === 0);
  while (queue.length > 0) {
    const id = queue.shift() as string;
    for (const to of forward.get(id) ?? []) {
      tierRank.set(to, Math.max(tierRank.get(to) ?? 0, (tierRank.get(id) ?? 0) + 1));
      const left = (incoming.get(to) ?? 0) - 1;
      incoming.set(to, left);
      if (left === 0) queue.push(to);
    }
  }
  // With any Ingress in the picture the first column is theirs, and every
  // tier moves one along: the Services they front, and the Services nothing
  // calls, together at the entry level behind them.
  const shift = nodes.some((n) => n.lane === "route") ? 1 : 0;
  return new Map(
    ids.map((id) => [
      id,
      laneOf.get(id) === "route" ? 0 : (tierRank.get(tierOf.get(id) as string) ?? 0) + shift,
    ]),
  );
}

/**
 * Order one column so its edges cross as little as possible, and so a tier
 * holds together.
 *
 * Two rules, in that order.
 *
 * **A tier stays whole.** Now that a Service and the pods answering it share a
 * column, they have to be adjacent rows or the picture is worse than the one
 * with twice the columns: `checkout-api`'s Service at the top and its
 * Deployment six rows down, joined by a line running past four unrelated
 * boxes. So the column is first cut into groups along the links inside it, and
 * a group is placed whole — the way in, then the address, then what answers.
 *
 * **Then the barycentre heuristic**, which is the standard answer for layered
 * graphs: a group sits at the average row of everything OUTSIDE the column
 * that its members are joined to, so the tier a caller calls lands beside the
 * caller. Groups joined to nothing keep their name order and go last — an
 * orphan has no opinion about where it belongs, and putting it first would
 * push everything with a reason to be somewhere away from that place.
 */

/**
 * Cut a column into tiers, each sorted the way traffic passes through it.
 *
 * Separate from {@link orderColumn} because the screen needs the boundaries
 * too: a tier is drawn inside a panel, and that panel has to know which rows
 * it holds.
 */
export function groupColumn(column: FlowNode[], within: [string, string][]): FlowNode[][] {
  const here = new Map(column.map((n) => [n.id, n]));
  const leader = new Map(column.map((n) => [n.id, n.id]));
  const find = (id: string): string => {
    let at = id;
    while (leader.get(at) !== at) at = leader.get(at) as string;
    return at;
  };
  for (const [a, b] of within) {
    if (!here.has(a) || !here.has(b)) continue;
    const ra = find(a);
    const rb = find(b);
    // The smaller id wins, so the same column groups identically every render.
    if (ra !== rb) leader.set(ra > rb ? ra : rb, ra > rb ? rb : ra);
  }

  const groups = new Map<string, FlowNode[]>();
  for (const node of column) {
    const root = find(node.id);
    const list = groups.get(root);
    if (list) list.push(node);
    else groups.set(root, [node]);
  }
  return [...groups.values()].map((members) =>
    [...members].sort(
      (a, b) =>
        (LANE_ORDER[a.lane] ?? 9) - (LANE_ORDER[b.lane] ?? 9) ||
        a.name.localeCompare(b.name) ||
        a.id.localeCompare(b.id),
    ),
  );
}

export function orderGroups(
  column: FlowNode[],
  neighbours: Map<string, string[]>,
  placedRows: Map<string, number>,
  /** Links whose two ends are both in this column. */
  within: [string, string][] = [],
): FlowNode[][] {
  const here = new Set(column.map((n) => n.id));
  const ordered = groupColumn(column, within).map((sorted) => {
    // Only rows OUTSIDE this column can say where the group belongs. A row
    // inside it is one this call is in the middle of deciding.
    const rows = sorted
      .flatMap((node) => neighbours.get(node.id) ?? [])
      .filter((other) => !here.has(other))
      .map((other) => placedRows.get(other))
      .filter((row): row is number => row !== undefined);
    return {
      members: sorted,
      name: sorted[0].name,
      barycentre: rows.length > 0 ? rows.reduce((a, b) => a + b, 0) / rows.length : undefined,
    };
  });

  ordered.sort((a, b) => {
    if (a.barycentre === undefined && b.barycentre === undefined) {
      return a.name.localeCompare(b.name);
    }
    if (a.barycentre === undefined) return 1;
    if (b.barycentre === undefined) return -1;
    return a.barycentre - b.barycentre || a.name.localeCompare(b.name);
  });
  return ordered.map((group) => group.members);
}

/** {@link orderGroups}, flattened to the rows it puts them in. */
export function orderColumn(
  column: FlowNode[],
  neighbours: Map<string, string[]>,
  placedRows: Map<string, number>,
  within: [string, string][] = [],
): FlowNode[] {
  return orderGroups(column, neighbours, placedRows, within).flat();
}

/**
 * `INGRESS` when there is one, `ENTRY` for the level behind it, then the hop
 * count from there. `entryRank` is the column the entry tiers stand in: one
 * when an Ingress column precedes them, zero when nothing does.
 */
export function columnLabel(rank: number, entryRank = 0): string {
  if (rank < entryRank) return "INGRESS";
  if (rank === entryRank) return "ENTRY";
  return `HOP ${rank - entryRank}`;
}

const TOP = PADDING + HEADER_HEIGHT;

/**
 * How thick each edge is drawn, from what was actually measured.
 *
 * Scaled within a unit and never across one: requests per second and open
 * connections are different quantities, and putting `5 conns` and `5 rps` on
 * one scale would draw a comparison that does not exist. Square-rooted so a
 * single very busy edge does not flatten every other measured line to the
 * minimum.
 *
 * A group with one member gets the middle width rather than the maximum.
 * Thickest-line-on-the-diagram is a claim about a comparison, and with nothing
 * to compare against there is no such claim to make.
 */
export function edgeWidths(edges: TopologyEdge[]): Map<string, number> {
  const groups = new Map<string, TopologyEdge[]>();
  for (const edge of edges) {
    if (edge.weight === null || edge.weight === undefined || edge.unit == null) continue;
    const list = groups.get(edge.unit);
    if (list) list.push(edge);
    else groups.set(edge.unit, [edge]);
  }

  const widths = new Map<string, number>();
  const middle = (MIN_EDGE_WIDTH + MAX_EDGE_WIDTH) / 2;
  for (const list of groups.values()) {
    const top = Math.max(...list.map((e) => e.weight ?? 0));
    for (const edge of list) {
      const key = edgeKey(edge);
      if (list.length < 2 || top <= 0) {
        widths.set(key, middle);
        continue;
      }
      const share = Math.sqrt(Math.max(edge.weight ?? 0, 0) / top);
      widths.set(key, r(MIN_EDGE_WIDTH + (MAX_EDGE_WIDTH - MIN_EDGE_WIDTH) * share));
    }
  }
  return widths;
}

export function edgeKey(edge: Pick<TopologyEdge, "from" | "to" | "kind">): string {
  return `${edge.kind}:${edge.from}->${edge.to}`;
}

/** More callers than this and a node is a hub: its incoming calls are drawn
 *  only when something selected puts them on a path. */
export const HUB_FAN_IN = 8;

/** Above this many measured edges the flow animation is dropped — hundreds
 *  of paths each re-rasterising every frame is most of what "laggy" was. */
export const FLOW_ANIMATION_LIMIT = 60;

/**
 * The nodes that too many things call, and how many.
 *
 * A production namespace had four external hosts each named in forty tiers'
 * configuration. Drawn in full that was a hundred and sixty dashed curves
 * fanning across the whole canvas — unreadable, most of the paint, and about
 * a fact better said as a number on the node. The edges still exist: the
 * inspector lists every caller, and selecting the hub or any caller draws
 * theirs.
 */
export function hubCounts(edges: Pick<TopologyEdge, "to" | "kind">[]): Map<string, number> {
  const fanIn = new Map<string, number>();
  for (const edge of edges) {
    if (edge.kind !== "calls") continue;
    fanIn.set(edge.to, (fanIn.get(edge.to) ?? 0) + 1);
  }
  return new Map([...fanIn].filter(([, n]) => n > HUB_FAN_IN));
}

/**
 * The curve between two nodes, and where its arrowhead points.
 *
 * Forward is the ordinary case and, now that columns are hops, very nearly the
 * only one: a cubic leaving the source's right edge and arriving at the
 * target's left, flat at both ends so it comes and goes horizontally however
 * far apart the rows are.
 *
 * Inside a tier — a Service and the pods that answer it, which now share a
 * column — it is a short vertical stub between two adjacent rows. Drawn small
 * and drawn last: this is the one line on the diagram that is not a network
 * call, and it should read as a bracket holding a group together rather than
 * as traffic going somewhere.
 *
 * What is left pointing backwards is a genuine cycle — two services that call
 * each other. Those leave from underneath and arrive from underneath, bowing
 * below the rows they pass, because a straight run backwards crosses every
 * column between the two and reads as noise.
 */
export function edgeGeometry(
  from: { x: number; y: number; detour?: Detour },
  to: { x: number; y: number },
): { path: string; arrow: string; labelX: number; labelY: number; midX: number; midY: number } {
  if (from.detour && to.x > from.detour.until) {
    // Down out of the tier, right along the channel under the block, then up
    // and in as any forward edge would arrive. The corner is rounded so the
    // line reads as one route and not as two lines meeting.
    const cx = from.x + NODE_WIDTH / 2;
    const yc = from.detour.y;
    const turn = from.detour.until + COLUMN_GAP * 0.35;
    const x2 = to.x - ARROW_SIZE;
    const y2 = to.y + NODE_HEIGHT / 2;
    const bend = Math.max((x2 - turn) / 2, 24);
    return {
      path: [
        `M ${r(cx)} ${r(from.y + NODE_HEIGHT)}`,
        `L ${r(cx)} ${r(yc - 10)}`,
        `Q ${r(cx)} ${r(yc)}, ${r(cx + 10)} ${r(yc)}`,
        `L ${r(turn)} ${r(yc)}`,
        `C ${r(turn + bend)} ${r(yc)}, ${r(x2 - bend)} ${r(y2)}, ${r(x2)} ${r(y2)}`,
      ].join(" "),
      arrow: arrowPoints(to.x, y2, 1, 0),
      labelX: r((cx + turn) / 2),
      labelY: r(yc - 7),
      midX: r((cx + turn) / 2),
      midY: r(yc),
    };
  }
  if (to.x === from.x) {
    const down = to.y > from.y;
    const x = from.x + NODE_WIDTH / 2;
    const y1 = down ? from.y + NODE_HEIGHT : from.y;
    const y2 = down ? to.y - STUB_ARROW : to.y + NODE_HEIGHT + STUB_ARROW;
    return {
      path: `M ${r(x)} ${r(y1)} L ${r(x)} ${r(y2)}`,
      // A smaller head than a call gets. Traffic does go this way, so it keeps
      // one — but at full size it filled the whole gap between two rows that
      // are meant to read as one thing.
      arrow: arrowPoints(x, down ? to.y : to.y + NODE_HEIGHT, 0, down ? 1 : -1, STUB_ARROW),
      labelX: r(x + 8),
      labelY: r((y1 + y2) / 2),
      midX: r(x),
      midY: r((y1 + y2) / 2),
    };
  }
  const forward = to.x > from.x;
  if (forward) {
    const x1 = from.x + NODE_WIDTH;
    const y1 = from.y + NODE_HEIGHT / 2;
    // Stop short of the box so the arrowhead sits in the gap rather than on
    // the border it points at.
    const x2 = to.x - ARROW_SIZE;
    const y2 = to.y + NODE_HEIGHT / 2;
    const bend = Math.max((x2 - x1) / 2, 24);
    return {
      path: `M ${r(x1)} ${r(y1)} C ${r(x1 + bend)} ${r(y1)}, ${r(x2 - bend)} ${r(y2)}, ${r(x2)} ${r(y2)}`,
      arrow: arrowPoints(to.x, y2, 1, 0),
      labelX: r((x1 + x2) / 2),
      // Clear of the line rather than on it. A busy edge is drawn thick, and
      // at six pixels the first render had `2.4k rpm` sitting in the stroke.
      labelY: r((y1 + y2) / 2 - 9),
      // The halfway point of this cubic is exactly the midpoint of its ends.
      midX: r((x1 + x2) / 2),
      midY: r((y1 + y2) / 2),
    };
  }
  const x1 = from.x + NODE_WIDTH / 2;
  const y1 = from.y + NODE_HEIGHT;
  const x2 = to.x + NODE_WIDTH / 2;
  const y2 = to.y + NODE_HEIGHT + ARROW_SIZE;
  // Deeper the further it has to travel, so long back-references do not lie on
  // top of short ones.
  const drop = Math.max(NODE_HEIGHT, Math.abs(x2 - x1) / 6) + ROW_GAP;
  return {
    path: `M ${r(x1)} ${r(y1)} C ${r(x1)} ${r(y1 + drop)}, ${r(x2)} ${r(y2 + drop)}, ${r(x2)} ${r(y2)}`,
    arrow: arrowPoints(to.x + NODE_WIDTH / 2, to.y + NODE_HEIGHT, 0, -1),
    labelX: r((x1 + x2) / 2),
    labelY: r(Math.max(y1, y2) + drop * 0.62),
    midX: r((x1 + x2) / 2),
    // Halfway along a bow whose control points both hang `drop` below.
    midY: r((y1 + y2) / 2 + drop * 0.75),
  };
}

export const ARROW_SIZE = 9;
/** The head on a link inside a tier, which has a ten-pixel gap to live in. */
export const STUB_ARROW = 6;

/**
 * A solid triangle at the target end, pointing the way traffic goes.
 *
 * The version this replaces drew no arrowheads at all, which for a diagram
 * whose entire subject is direction was the single worst thing about it: `a
 * calls b` and `b calls a` were the same picture, and the reader was left to
 * infer direction from which column a box happened to be in.
 *
 * Drawn as a polygon rather than an SVG `marker` so it takes the same `fill-*`
 * class as the line takes `stroke-*`, and so follows the theme.
 */
export function arrowPoints(
  x: number,
  y: number,
  dx: number,
  dy: number,
  size = ARROW_SIZE,
): string {
  const length = Math.hypot(dx, dy) || 1;
  const ux = dx / length;
  const uy = dy / length;
  const backX = x - ux * size;
  const backY = y - uy * size;
  const half = size * 0.4;
  return [
    `${r(x)},${r(y)}`,
    `${r(backX - uy * half)},${r(backY + ux * half)}`,
    `${r(backX + uy * half)},${r(backY - ux * half)}`,
  ].join(" ");
}

/**
 * Which nodes stand on a path anyone knows about.
 *
 * Touched by a `calls` edge from either end, or an Ingress — traffic enters
 * there whether or not anything downstream has been seen — and then everything
 * a `routes` edge joins to those, so a tier goes into the flow whole rather
 * than its Service going one way and its pods the other.
 *
 * Everything else is what the first real cluster showed up: on a cluster with
 * no mesh, no metrics backend and the probe off, almost no calls are known, so
 * almost every tier ranked zero and the picture was one column, twenty tiers
 * tall, with a ragged two-column flow off to its right. Every one of those
 * placements was correct, and the picture said nothing. A tier no call touches
 * has no hop to be at — it belongs in the band below, packed to the pane,
 * where it reads as the inventory it is instead of as the start of a chain.
 */
export function onAPath(nodes: FlowNode[], edges: TopologyEdge[]): Set<string> {
  const present = new Set(nodes.map((n) => n.id));
  const found = new Set<string>();
  for (const edge of edges) {
    if (edge.kind !== "calls") continue;
    if (present.has(edge.from)) found.add(edge.from);
    if (present.has(edge.to)) found.add(edge.to);
  }
  for (const node of nodes) if (node.lane === "route") found.add(node.id);

  const routed = new Map<string, string[]>();
  const join = (a: string, b: string) => {
    const list = routed.get(a);
    if (list) list.push(b);
    else routed.set(a, [b]);
  };
  for (const edge of edges) {
    if (edge.kind !== "routes") continue;
    join(edge.from, edge.to);
    join(edge.to, edge.from);
  }
  const queue = [...found];
  while (queue.length > 0) {
    const at = queue.shift() as string;
    for (const next of routed.get(at) ?? []) {
      if (found.has(next) || !present.has(next)) continue;
      found.add(next);
      queue.push(next);
    }
  }
  return found;
}

/**
 * Place a whole graph.
 *
 * Every node the backend returned is placed, ReplicaSets aside: a Service
 * fronting nothing and a workload no Service selects are both real findings,
 * and dropping them would quietly answer "there is nothing there". The tiers
 * on a known path are laid out by hop; the rest go in a band beneath, see
 * {@link onAPath} for why.
 */
export function layoutFlow(graph: TopologyGraph): TopologyLayout {
  const { nodes, edges } = fold(graph);
  const routed = onAPath(nodes, edges);
  const flow = placeFlow(
    nodes.filter((n) => routed.has(n.id)),
    edges.filter((e) => routed.has(e.from) && routed.has(e.to)),
  );
  const band = placeBand(
    nodes.filter((n) => !routed.has(n.id)),
    edges.filter((e) => !routed.has(e.from) && !routed.has(e.to)),
    flow.placed.length === 0 ? null : flow.bottom,
    flow.right,
  );

  const placed = [...flow.placed, ...band.placed];
  const byId = new Map(placed.map((n) => [n.id, n]));
  const widths = edgeWidths(edges);
  const drawn: PlacedEdge[] = [];
  for (const edge of edges) {
    const from = byId.get(edge.from);
    const to = byId.get(edge.to);
    // An edge with an endpoint that is not drawn has nothing to connect. The
    // backend does not emit these; drawing a line into empty space if it ever
    // did would be worse than dropping it.
    if (!from || !to) continue;
    const key = edgeKey(edge);
    drawn.push({
      ...edge,
      key,
      ...edgeGeometry(from, to),
      width: widths.get(key) ?? MIN_EDGE_WIDTH,
      crossesNamespace:
        from.namespace !== "" && to.namespace !== "" && from.namespace !== to.namespace,
    });
  }

  return {
    nodes: placed,
    edges: drawn,
    columns: flow.columns,
    tiers: [...flow.tiers, ...band.tiers],
    lanes: flow.lanes,
    band: band.header,
    entryRank: flow.entryRank,
    namespaces: [...new Set(placed.map((n) => n.namespace))].filter(Boolean).sort(),
    width: Math.max(flow.right, band.right) + PADDING,
    height: (band.header ? band.bottom : flow.bottom) + PADDING,
  };
}

/**
 * How a tier is laid out inside its panel.
 *
 * The Ingress, if any, on top; beneath it the Service lane on the left and the
 * workload lane on the right, each a short stack. So a tier reads the way a
 * request goes — in at the top, to the address, across to the pods — and a
 * call to the next tier leaves from the pods on the right, which is where the
 * next tier is. The first version stacked all three down one column, which
 * made the eye go down and then right at every tier.
 *
 * The Ingress is ABOVE the Service rather than left of it on purpose: a
 * Service that is both Ingress-fronted and called internally would otherwise
 * have its incoming call drawn straight through the Ingress card.
 */
export interface TierShape {
  routes: FlowNode[];
  left: FlowNode[];
  right: FlowNode[];
  /** In pixels: one node, or two and the gap between. */
  width: number;
  rows: number;
}

export function shapeTier(group: FlowNode[]): TierShape {
  const routes = group.filter((n) => n.lane === "route");
  const services = group.filter((n) => n.lane === "service");
  const others = group.filter((n) => n.lane !== "route" && n.lane !== "service");
  // Without a Service the pods take the left lane themselves; without pods the
  // Service stands alone. An external host or an orphan ReplicaSet is a tier
  // of one and goes left too.
  const left = services.length > 0 ? services : others;
  const right = services.length > 0 ? others : [];
  return {
    routes,
    left,
    right,
    width: right.length > 0 ? NODE_WIDTH * 2 + INNER_GAP : NODE_WIDTH,
    rows: routes.length + Math.max(left.length, right.length),
  };
}

/** Place one tier with its top-left at (`x`, `top`). Answers its bottom. */
function placeTier(
  group: FlowNode[],
  x: number,
  top: number,
  rank: number | null,
  placed: PlacedNode[],
  tiers: Tier[],
): { nodes: PlacedNode[]; bottom: number } {
  const shape = shapeTier(group);
  const nodes: PlacedNode[] = [];
  const put = (node: FlowNode, px: number, py: number) => {
    const at: PlacedNode = { ...node, x: px, y: py, rank };
    placed.push(at);
    nodes.push(at);
  };
  let y = top;
  for (const node of shape.routes) {
    put(node, x, y);
    y += NODE_HEIGHT + ROW_GAP;
  }
  let yl = y;
  let yr = y;
  for (const node of shape.left) {
    put(node, x, yl);
    yl += NODE_HEIGHT + ROW_GAP;
  }
  for (const node of shape.right) {
    put(node, x + NODE_WIDTH + INNER_GAP, yr);
    yr += NODE_HEIGHT + ROW_GAP;
  }
  const bottom = nodes.length > 0 ? Math.max(y, yl, yr) - ROW_GAP : top;
  // A tier of one is just a node; a panel round it would be a box drawn twice.
  if (group.length > 1) {
    tiers.push({
      x: x - TIER_PAD,
      y: top - TIER_PAD,
      width: shape.width + TIER_PAD * 2,
      height: bottom - top + TIER_PAD * 2,
    });
  }
  return { nodes, bottom };
}

/**
 * Deal tiers into `k` sub-columns, each going to whichever is shortest, and
 * say how wide each sub-column has to be. Deterministic: the same tiers in
 * the same order deal the same way every render.
 */
function assign(groups: FlowNode[][], k: number): { groups: FlowNode[][]; width: number }[] {
  const subs = Array.from({ length: Math.max(1, k) }, () => ({
    groups: [] as FlowNode[][],
    rows: 0,
    width: 0,
  }));
  for (const group of groups) {
    let s = 0;
    for (let i = 1; i < subs.length; i++) if (subs[i].rows < subs[s].rows) s = i;
    const shape = shapeTier(group);
    subs[s].groups.push(group);
    subs[s].rows += shape.rows;
    subs[s].width = Math.max(subs[s].width, shape.width);
  }
  return subs.filter((s) => s.groups.length > 0).map((s) => ({ groups: s.groups, width: s.width }));
}

/**
 * The tiers on no known path, packed into a grid under the flow.
 *
 * As wide as the flow above it when there is one, so the two read as one
 * picture, and otherwise roughly square-ish in tiers — the shape that fits a
 * pane, which is the whole reason these are not a column. Each tier goes into
 * whichever grid column is shortest, in namespace-then-name order, so the same
 * namespace's tiers sit near each other and the same graph packs identically
 * every render.
 */
function placeBand(
  nodes: FlowNode[],
  edges: TopologyEdge[],
  /** Where the flow ends, or `null` when there is no flow above. */
  below: number | null,
  /** The flow's right edge, which the band grows to meet. */
  minRight: number,
): { placed: PlacedNode[]; tiers: Tier[]; header: Band | null; bottom: number; right: number } {
  const bottom = below ?? TOP;
  if (nodes.length === 0) {
    return { placed: [], tiers: [], header: null, bottom, right: minRight };
  }
  const groups = groupColumn(
    nodes,
    edges.map((e) => [e.from, e.to]),
  ).sort(
    (a, b) =>
      a[0].namespace.localeCompare(b[0].namespace) ||
      a[0].name.localeCompare(b[0].name) ||
      a[0].id.localeCompare(b[0].id),
  );

  const widest = Math.max(...groups.map((g) => shapeTier(g).width));
  const squarish = Math.ceil(Math.sqrt(groups.length * 1.6));
  const fillsFlow = Math.floor((minRight - PADDING + GRID_GAP) / (widest + GRID_GAP));
  const cols = Math.max(1, Math.min(groups.length, Math.max(squarish, fillsFlow)));

  const y = below === null ? PADDING : below + BAND_GAP;
  const top = y + HEADER_HEIGHT;
  const placed: PlacedNode[] = [];
  const tiers: Tier[] = [];
  let x = PADDING;
  let end = top;
  for (const sub of assign(groups, cols)) {
    let at = top;
    for (const group of sub.groups) {
      if (at > top) at += TIER_GAP;
      at = placeTier(group, x, at, null, placed, tiers).bottom;
    }
    end = Math.max(end, at);
    x += sub.width + GRID_GAP;
  }
  return {
    placed,
    tiers,
    header: {
      y,
      label: `NO KNOWN CALLS · ${groups.length}`,
      count: nodes.length,
    },
    bottom: end,
    right: x - GRID_GAP,
  };
}

/** The tiers on a known path, by hop. */
function placeFlow(
  nodes: FlowNode[],
  edges: TopologyEdge[],
): {
  placed: PlacedNode[];
  tiers: Tier[];
  columns: Column[];
  lanes: NamespaceLane[];
  entryRank: number;
  bottom: number;
  right: number;
} {
  const rank = rankNodes(nodes, edges);

  /**
   * Both directions, deliberately.
   *
   * Ordering by incoming edges alone lays dependencies out badly: a Service
   * that exists only because something calls it has one edge, pointing in, and
   * ordering its column without looking at that edge's other end drops it to
   * the bottom while its caller sits at the top of the column before. A node
   * belongs beside everything it is joined to, whichever way the arrow points.
   */
  const neighbours = new Map<string, string[]>();
  const join = (a: string, b: string) => {
    const list = neighbours.get(a);
    if (list) list.push(b);
    else neighbours.set(a, [b]);
  };
  for (const edge of edges) {
    join(edge.to, edge.from);
    join(edge.from, edge.to);
  }

  const deepest = nodes.reduce((max, n) => Math.max(max, rank.get(n.id) ?? 0), 0);
  const members: FlowNode[][] = [];
  // An empty graph gets no columns at all. A lone `ENTRY` heading over nothing
  // reads as a column whose contents failed to load.
  for (let i = 0; nodes.length > 0 && i <= deepest; i++) {
    members.push(nodes.filter((n) => (rank.get(n.id) ?? 0) === i));
  }

  /**
   * Two sweeps, not one.
   *
   * On the first, a column can only be positioned by neighbours already placed
   * to its left. The second runs with every row known, so a column is finally
   * placed against BOTH sides — which is what actually lines a caller up with
   * what it calls. Two is enough at this size, and a fixed number of passes
   * keeps the layout deterministic.
   */
  // The links that hold a tier together — a Service and the pods answering it
  // stand in one column, and those two have to end up adjacent.
  const isRoute = new Set(nodes.filter((n) => n.lane === "route").map((n) => n.id));
  const within: [string, string][] = edges
    .filter((edge) => edge.kind === "routes" && !isRoute.has(edge.from))
    .map((edge) => [edge.from, edge.to]);
  // The Ingresses have the first column to themselves when there are any.
  const entryIndex = isRoute.size > 0 ? 1 : 0;

  let rows = new Map<string, number>();
  let ordered: FlowNode[][][] = members.map((column) => [column]);
  for (let sweep = 0; sweep < 2; sweep++) {
    const next = new Map<string, number>();
    ordered = members.map((column) => {
      const groups = orderGroups(column, neighbours, sweep === 0 ? next : rows, within);
      groups.flat().forEach((node, row) => next.set(node.id, row));
      return groups;
    });
    rows = next;
  }

  /**
   * Down the column, tight inside a tier and loose between them.
   *
   * The gap is the grouping. Uniform rows gave the Service and the pods
   * answering it exactly as much air as two unrelated tiers got, which said
   * nothing about what belongs with what — and left no room to draw the panel
   * that says it outright.
   */
  const placed: PlacedNode[] = [];
  const tiers: Tier[] = [];

  /**
   * Stack tiers into `k` sub-columns from `x0`, down from `top`, each tier
   * going to whichever sub-column is shortest. One sub-column is the ordinary
   * hop column; several is how ENTRY spreads out. A sub-column is as wide as
   * the widest tier in it. `mark` hears about every node placed and which
   * sub-column it landed in. Answers the right edge and the bottom.
   */
  const pack = (
    groups: FlowNode[][],
    k: number,
    x0: number,
    rankIndex: number,
    top: number,
    mark?: (node: PlacedNode, sub: number) => void,
  ): { right: number; bottom: number } => {
    const subs = assign(groups, k);
    let x = x0;
    let bottom = top;
    subs.forEach((sub, s) => {
      let y = top;
      for (const group of sub.groups) {
        if (y > top) y += TIER_GAP;
        const out = placeTier(group, x, y, rankIndex, placed, tiers);
        for (const node of out.nodes) mark?.(node, s);
        y = out.bottom;
      }
      bottom = Math.max(bottom, y);
      x += sub.width + GRID_GAP;
    });
    return { right: x - GRID_GAP, bottom };
  };
  const blockWidth = (groups: FlowNode[][], k: number) =>
    assign(groups, k).reduce((w, sub, i) => w + sub.width + (i > 0 ? GRID_GAP : 0), 0);

  /**
   * One lane per namespace, top to bottom.
   *
   * Several namespaces drawn together used to interleave down every column,
   * which put `checkout`'s storefront directly under `shop`'s and left the
   * reader to sort them apart by the small print. Each namespace now has its
   * own horizontal lane, the hop columns run through all of them, and a call
   * that leaves one lane for another is marked as the thing it is. A host
   * outside the cluster has no namespace and goes in the lane of whichever
   * namespace calls it first, by name, so it sits beside its caller.
   */
  const callersOf = new Map<string, Set<string>>();
  const nsOf = new Map(nodes.map((n) => [n.id, n.namespace]));
  for (const edge of edges) {
    if (edge.kind !== "calls") continue;
    const from = nsOf.get(edge.from);
    if (!from) continue;
    const set = callersOf.get(edge.to);
    if (set) set.add(from);
    else callersOf.set(edge.to, new Set([from]));
  }
  const laneOf = (group: FlowNode[]): string => {
    const own = group[0]?.namespace ?? "";
    if (own) return own;
    const callers = [...new Set(group.flatMap((n) => [...(callersOf.get(n.id) ?? [])]))].sort();
    return callers[0] ?? "";
  };
  const laneKeys = [...new Set(ordered.flat().map(laneOf))].sort(
    // Anything left without a namespace goes last.
    (a, b) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b)),
  );
  const multi = laneKeys.length > 1;

  /**
   * ENTRY spreads sideways.
   *
   * It is the one column nothing arrives at from the left, and it is where
   * the height went: on a real namespace three Ingress-fronted tiers stacked
   * ten rows tall beside a two-hop flow. The tiers that call nothing fan out
   * to the LEFT of the ones that do, in as many sub-columns as it takes to
   * come no taller than them. The callers stand side by side too, against
   * HOP 1 — and because a curve drawn straight out of an inner one would run
   * through its neighbour, an inner caller's edges leave downwards, along a
   * channel under the lane, and only turn up once past the block. All of it
   * is still rank zero, and the heading spans the lot.
   *
   * The block is right-aligned against a common edge across every lane, so
   * the hop columns line up however wide one lane's ENTRY is.
   */
  const callsOut = new Set(
    edges
      .filter((e) => e.kind === "calls" && (rank.get(e.from) ?? 0) !== (rank.get(e.to) ?? 0))
      .map((e) => e.from),
  );
  const isCaller = (group: FlowNode[]) => group.some((n) => callsOut.has(n.id));
  const rowsIn = (groups: FlowNode[][]) => groups.reduce((n, g) => n + shapeTier(g).rows, 0);
  const squarish = (n: number) => Math.max(1, Math.ceil(Math.sqrt(n * 1.6)));
  const plans = laneKeys.map((lane) => {
    const entry = (ordered[entryIndex] ?? []).filter((g) => laneOf(g) === lane);
    const callers = entry.filter(isCaller);
    const leaves = entry.filter((g) => !isCaller(g));
    const kCallers = callers.length > 0 ? Math.min(callers.length, squarish(callers.length)) : 0;
    const callerRows = kCallers > 0 ? Math.ceil(rowsIn(callers) / kCallers) : 0;
    const kLeaves =
      leaves.length > 0
        ? Math.min(
            leaves.length,
            callerRows > 0 ? Math.ceil(rowsIn(leaves) / callerRows) : squarish(leaves.length),
          )
        : 0;
    return { lane, callers, leaves, kCallers, kLeaves };
  });
  // Every lane's ENTRY block is right-aligned against one edge, so the hop
  // columns line up however wide one lane's block is; and each hop column is
  // as wide as the widest tier in it, across every lane.
  const callersWidth = (p: (typeof plans)[number]) =>
    p.kCallers > 0 ? blockWidth(p.callers, p.kCallers) : 0;
  const leavesWidth = (p: (typeof plans)[number]) =>
    p.kLeaves > 0 ? blockWidth(p.leaves, p.kLeaves) : 0;
  const entryWidth = Math.max(
    0,
    ...plans.map((p) => {
      const c = callersWidth(p);
      const l = leavesWidth(p);
      return c + l + (c > 0 && l > 0 ? GRID_GAP : 0);
    }),
  );
  const entryLeft = PADDING + (entryIndex === 1 ? NODE_WIDTH + COLUMN_GAP : 0);
  const entryRight = entryLeft + entryWidth;
  const colWidth = ordered.map((column, c) =>
    c === entryIndex
      ? entryWidth
      : Math.max(NODE_WIDTH, ...column.map((g) => shapeTier(g).width)),
  );
  const hopX = (c: number) => {
    let x = entryRight + COLUMN_GAP;
    for (let i = entryIndex + 1; i < c; i++) x += colWidth[i] + COLUMN_GAP;
    return x;
  };

  const lanes: NamespaceLane[] = [];
  let top = TOP + (multi ? LANE_LABEL : 0);
  for (const plan of plans) {
    let bottom = top;
    const inner: PlacedNode[] = [];
    if (entryIndex === 1) {
      // The Ingresses, in one stack against the entries they route to.
      const gates = ordered[0].filter((g) => laneOf(g) === plan.lane);
      if (gates.length > 0) bottom = Math.max(bottom, pack(gates, 1, PADDING, 0, top).bottom);
    }
    if (plan.kCallers > 0) {
      const out = pack(
        plan.callers,
        plan.kCallers,
        entryRight - callersWidth(plan),
        entryIndex,
        top,
        (node, sub) => {
          if (sub < plan.kCallers - 1) inner.push(node);
        },
      );
      bottom = Math.max(bottom, out.bottom);
    }
    if (plan.kLeaves > 0) {
      const x0 =
        entryRight - (plan.kCallers > 0 ? callersWidth(plan) + GRID_GAP : 0) - leavesWidth(plan);
      bottom = Math.max(bottom, pack(plan.leaves, plan.kLeaves, x0, entryIndex, top).bottom);
    }
    for (let c = entryIndex + 1; c < ordered.length; c++) {
      const groups = ordered[c].filter((g) => laneOf(g) === plan.lane);
      if (groups.length === 0) continue;
      bottom = Math.max(bottom, pack(groups, 1, hopX(c), c, top).bottom);
    }
    if (inner.length > 0) {
      const y = bottom + CHANNEL_DROP;
      for (const node of inner) node.detour = { y, until: entryRight };
      bottom = y + CHANNEL_CLEARANCE;
    }
    lanes.push({ namespace: plan.lane, y: top, height: bottom - top });
    top = bottom + (multi ? LANE_GAP : 0);
  }

  const columns: Column[] = ordered.map((column, c) => ({
    rank: c,
    label: columnLabel(c, entryIndex),
    x: c < entryIndex ? PADDING : c === entryIndex ? entryLeft : hopX(c),
    count: column.reduce((n, g) => n + g.length, 0),
  }));
  const last = lanes.at(-1);
  const bottom = last ? last.y + last.height : TOP;

  // Only a genuinely backward edge bows under the last row — a link inside a
  // tier is a short stub between two adjacent rows and needs no clearance —
  // and the room for it is claimed here, above the band, or the bow would run
  // straight through it.
  const bowing = edges.some(
    (edge) => (rank.get(edge.to) ?? 0) < (rank.get(edge.from) ?? 0),
  );
  return {
    placed,
    tiers,
    columns,
    lanes: multi ? lanes : [],
    entryRank: entryIndex,
    bottom: bottom + (bowing ? BACKWARD_CLEARANCE : 0),
    right:
      ordered.length > entryIndex + 1
        ? hopX(ordered.length - 1) + colWidth[ordered.length - 1]
        : entryRight,
  };
}

/**
 * Everything on a path through one node, both ways.
 *
 * The version this replaces highlighted one hop and faded the rest, which
 * answers "what does this touch" and not the question an SRE actually has,
 * which is "if this is broken, what else is". That one is transitive: the
 * whole chain upstream is what could be causing it, and the whole chain
 * downstream is what it takes with it.
 *
 * An edge is in the trace when both its ends are — so a call between two
 * things the selection reaches is part of what it reaches, which is true.
 */
export interface Trace {
  nodes: Set<string>;
  edges: Set<string>;
  /** Everything that can reach the selection, excluding it. */
  upstream: Set<string>;
  /** Everything the selection can reach, excluding it. */
  downstream: Set<string>;
}

export function traceFrom(id: string, edges: PlacedEdge[]): Trace {
  const out = new Map<string, string[]>();
  const into = new Map<string, string[]>();
  for (const edge of edges) {
    const o = out.get(edge.from);
    if (o) o.push(edge.to);
    else out.set(edge.from, [edge.to]);
    const i = into.get(edge.to);
    if (i) i.push(edge.from);
    else into.set(edge.to, [edge.from]);
  }

  const walk = (adjacency: Map<string, string[]>): Set<string> => {
    const found = new Set<string>();
    const queue = [id];
    while (queue.length > 0) {
      const at = queue.shift() as string;
      for (const next of adjacency.get(at) ?? []) {
        // The guard is also what makes a cycle terminate here.
        if (next === id || found.has(next)) continue;
        found.add(next);
        queue.push(next);
      }
    }
    return found;
  };

  const downstream = walk(out);
  const upstream = walk(into);
  const nodes = new Set<string>([id, ...downstream, ...upstream]);
  const traced = new Set<string>();
  for (const edge of edges) {
    if (nodes.has(edge.from) && nodes.has(edge.to)) traced.add(edge.key);
  }
  return { nodes, edges: traced, upstream, downstream };
}

export interface Transform {
  k: number;
  tx: number;
  ty: number;
}

export function clampZoom(k: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, k));
}

/**
 * The transform that puts the whole graph in view, centred.
 *
 * Never zooms IN past 1:1. A two-node namespace scaled up to fill a wide pane
 * looks like a bug — enormous boxes, a single enormous arrow — and the fact
 * that there is very little here is itself worth seeing.
 *
 * Allowed to go BELOW {@link MIN_ZOOM}, which the wheel and the buttons are
 * not. That floor exists so a reader cannot zoom themselves into an unreadable
 * smear by accident; a button labelled Fit that then does not fit would be a
 * worse lie than a small picture, and on a graph that wide the shape is the
 * only thing there was to see anyway.
 */
export function fitTransform(
  layout: { width: number; height: number },
  viewport: { width: number; height: number },
): Transform {
  if (
    !(viewport.width > 0) ||
    !(viewport.height > 0) ||
    !(layout.width > 0) ||
    !(layout.height > 0)
  ) {
    return { k: 1, tx: 0, ty: 0 };
  }
  const k = Math.min(viewport.width / layout.width, viewport.height / layout.height, 1);
  return {
    k,
    tx: r((viewport.width - layout.width * k) / 2),
    ty: r((viewport.height - layout.height * k) / 2),
  };
}

/**
 * Zoom about a point, so the thing under the pointer stays under it.
 *
 * Zooming about the origin instead is the small wrongness that makes a canvas
 * feel broken: the reader aims at a node, scrolls, and the node they were
 * looking at leaves the screen.
 */
export function zoomAt(current: Transform, factor: number, px: number, py: number): Transform {
  // Fit may have put the view below the manual floor for a graph too wide to
  // fit otherwise; zooming OUT from there must not snap it back up to the
  // floor, which made a graph 2.5x larger on a click that asked for smaller.
  const floor = Math.min(MIN_ZOOM, current.k);
  const k = Math.min(MAX_ZOOM, Math.max(floor, current.k * factor));
  const ratio = k / current.k;
  return {
    k,
    tx: r(px - (px - current.tx) * ratio),
    ty: r(py - (py - current.ty) * ratio),
  };
}

/**
 * Cut a name to what the box can hold.
 *
 * SVG text does not wrap and does not clip to its parent, so a long host runs
 * straight out of its node and over the next column — `orders-db.internal.
 * example.com` drawn as itself, lying across the border. The full name stays
 * reachable: the node carries it as a `title`, and the inspector never
 * truncates.
 */
export function fit(name: string, max = 24): string {
  return name.length <= max ? name : `${name.slice(0, max - 1)}…`;
}
