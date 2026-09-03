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
/** Between the bottom of the flow and the band's heading. */
export const BAND_GAP = 64;
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

export interface PlacedNode extends FlowNode {
  x: number;
  y: number;
  /** Hops from where traffic enters. Zero is an entry point; `null` is a node
   *  on no known path at all, drawn in the band below the flow. */
  rank: number | null;
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
  band: Band | null;
  /** Every namespace drawn, sorted. The screen labels nodes with theirs only
   *  when there is more than one, because on a single-namespace graph the
   *  namespace is the title and repeating it on every box is noise. */
  namespaces: string[];
  width: number;
  height: number;
}

/** An edge that carries traffic. `owns` is a fact about who built a thing. */
function isFlow(edge: TopologyEdge): boolean {
  return edge.kind === "routes" || edge.kind === "calls";
}

/**
 * What an edge costs in hops.
 *
 * **Only a call is a hop.** `routes` is an address being resolved — an Ingress
 * naming a Service, a Service selecting the pods behind it — and none of those
 * is a network call the application makes. Charging them a column each was the
 * first version of this layout and it drew the demo namespace 3112 pixels wide
 * and 220 tall: eight columns, every one of them a Service directly followed
 * by the single Deployment it fronts, and the whole thing fitted to 37% before
 * it would go in a pane. Unreadable, and about nothing — half those columns
 * were the same tier written twice.
 *
 * Zero-costing them turns a column into a SERVICE TIER: the way in, the
 * address, and the pods that answer it, all standing together, with the
 * arrows between columns being exactly the calls one tier makes on the next.
 * Which is the thing the diagram is for.
 */
function hopCost(edge: TopologyEdge): number {
  return edge.kind === "calls" ? 1 : 0;
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

  const folded = new Set<string>();
  const revisions = new Map<string, string[]>();
  for (const node of graph.nodes) {
    if (node.lane !== "replicaset") continue;
    const into = owner.get(node.id);
    if (into === undefined) continue;
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
  const out = new Map<string, string[]>();
  const cost = new Map<string, number>();
  for (const edge of edges) {
    if (!isFlow(edge)) continue;
    if (!present.has(edge.from) || !present.has(edge.to)) continue;
    const pair = `${edge.from}->${edge.to}`;
    if (edge.from === edge.to) continue;
    // A pair joined twice is one arc as far as ranking is concerned, and it
    // costs the most any of them costs: a Service both routed to and called is
    // being called, and the call is what moves it along.
    cost.set(pair, Math.max(cost.get(pair) ?? 0, hopCost(edge)));
    if (out.has(edge.from)) {
      if (!out.get(edge.from)?.includes(edge.to)) out.get(edge.from)?.push(edge.to);
    } else {
      out.set(edge.from, [edge.to]);
    }
  }
  for (const list of out.values()) list.sort();

  const back = backEdges(ids, out);
  const forward = new Map<string, string[]>();
  const incoming = new Map<string, number>(ids.map((id) => [id, 0]));
  for (const [from, children] of out) {
    const kept = children.filter((to) => !back.has(`${from}->${to}`));
    forward.set(from, kept);
    for (const to of kept) incoming.set(to, (incoming.get(to) ?? 0) + 1);
  }

  const rank = new Map<string, number>(ids.map((id) => [id, 0]));
  const queue = ids.filter((id) => (incoming.get(id) ?? 0) === 0);
  while (queue.length > 0) {
    const id = queue.shift() as string;
    for (const to of forward.get(id) ?? []) {
      const step = cost.get(`${id}->${to}`) ?? 1;
      rank.set(to, Math.max(rank.get(to) ?? 0, (rank.get(id) ?? 0) + step));
      const left = (incoming.get(to) ?? 0) - 1;
      incoming.set(to, left);
      if (left === 0) queue.push(to);
    }
  }
  return rank;
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

/** `ENTRY` for the first column, then the hop count. */
export function columnLabel(rank: number): string {
  return rank === 0 ? "ENTRY" : `HOP ${rank}`;
}

function columnX(rank: number): number {
  return PADDING + rank * (NODE_WIDTH + COLUMN_GAP);
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
  from: { x: number; y: number },
  to: { x: number; y: number },
): { path: string; arrow: string; labelX: number; labelY: number } {
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
    });
  }

  return {
    nodes: placed,
    edges: drawn,
    columns: flow.columns,
    tiers: [...flow.tiers, ...band.tiers],
    band: band.header,
    namespaces: [...new Set(placed.map((n) => n.namespace))].filter(Boolean).sort(),
    width: Math.max(flow.right, band.right) + PADDING,
    height: (band.header ? band.bottom : flow.bottom) + PADDING,
  };
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

  const pitch = NODE_WIDTH + GRID_GAP;
  const squarish = Math.ceil(Math.sqrt(groups.length * 1.6));
  const fillsFlow = Math.floor((minRight - PADDING + GRID_GAP) / pitch);
  const cols = Math.max(1, Math.min(groups.length, Math.max(squarish, fillsFlow)));

  const y = below === null ? PADDING : below + BAND_GAP;
  const top = y + HEADER_HEIGHT;
  const heights: number[] = new Array<number>(cols).fill(top);
  const placed: PlacedNode[] = [];
  const tiers: Tier[] = [];
  for (const group of groups) {
    let column = 0;
    for (let i = 1; i < cols; i++) if (heights[i] < heights[column]) column = i;
    const x = PADDING + column * pitch;
    let at = heights[column];
    if (at > top) at += TIER_GAP;
    const groupTop = at;
    group.forEach((node, row) => {
      if (row > 0) at += ROW_GAP;
      placed.push({ ...node, x, y: at, rank: null });
      at += NODE_HEIGHT;
    });
    if (group.length > 1) {
      tiers.push({
        x: x - TIER_PAD,
        y: groupTop - TIER_PAD,
        width: NODE_WIDTH + TIER_PAD * 2,
        height: at - groupTop + TIER_PAD * 2,
      });
    }
    heights[column] = at;
  }
  return {
    placed,
    tiers,
    header: {
      y,
      label: `NO KNOWN CALLS · ${groups.length}`,
      count: nodes.length,
    },
    bottom: Math.max(...heights),
    right: PADDING + cols * pitch - GRID_GAP,
  };
}

/** The tiers on a known path, by hop. */
function placeFlow(
  nodes: FlowNode[],
  edges: TopologyEdge[],
): { placed: PlacedNode[]; tiers: Tier[]; columns: Column[]; bottom: number; right: number } {
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
  // now stand in one column, and those two have to end up adjacent.
  const within: [string, string][] = edges
    .filter((edge) => (rank.get(edge.from) ?? 0) === (rank.get(edge.to) ?? 0))
    .map((edge) => [edge.from, edge.to]);

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
  let bottom = TOP;
  const columns: Column[] = [];
  ordered.forEach((column, index) => {
    const x = columnX(index);
    columns.push({
      rank: index,
      label: columnLabel(index),
      x,
      count: column.reduce((n, group) => n + group.length, 0),
    });
    let y = TOP;
    column.forEach((group, g) => {
      if (g > 0) y += TIER_GAP;
      const top = y;
      group.forEach((node, row) => {
        if (row > 0) y += ROW_GAP;
        placed.push({ ...node, x, y, rank: index });
        y += NODE_HEIGHT;
      });
      // A tier of one is just a node; a panel round it would be a box drawn
      // twice.
      if (group.length > 1) {
        tiers.push({
          x: x - TIER_PAD,
          y: top - TIER_PAD,
          width: NODE_WIDTH + TIER_PAD * 2,
          height: y - top + TIER_PAD * 2,
        });
      }
    });
    bottom = Math.max(bottom, y);
  });

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
    bottom: bottom + (bowing ? BACKWARD_CLEARANCE : 0),
    right: (columns.at(-1)?.x ?? PADDING) + (columns.length > 0 ? NODE_WIDTH : 0),
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
  const k = clampZoom(current.k * factor);
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
