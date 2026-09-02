import type { TopologyEdge, TopologyGraph, TopologyLane, TopologyNode } from "@srelens/core";

/**
 * Where every node and edge of a topology graph goes, decided once and
 * without touching the DOM.
 *
 * Separated from the screen for the same reason the joins are separated from
 * the capability: placement is the part that can be wrong, and it is far
 * easier to assert `checkout-api sits below checkout-web` against a returned
 * array than against an SVG. The screen does no arithmetic of its own — it
 * draws what this returns.
 */

/** Left to right. The backend names the same four; this fixes their order. */
export const LANES: readonly TopologyLane[] = ["route", "service", "workload", "replicaset", "external"];

/** The column headings, in the design's voice. */
export const LANE_LABELS: Record<TopologyLane, string> = {
  route: "ROUTE",
  service: "SERVICE",
  workload: "WORKLOAD",
  replicaset: "REPLICA SET",
  external: "DEPENDENCY",
};

export const NODE_WIDTH = 168;
export const NODE_HEIGHT = 52;
/** Horizontal gap between lanes — wide enough for an edge to read as a curve. */
export const LANE_GAP = 96;
export const ROW_GAP = 20;
/** Room under the last row for edges that bow below it. */
export const BACKWARD_CLEARANCE = 72;

/** Room above the first row for the lane headings. */
export const HEADER_HEIGHT = 28;

export interface PlacedNode extends TopologyNode {
  x: number;
  y: number;
}

export interface PlacedEdge extends TopologyEdge {
  /** An SVG cubic path from the source's right edge to the target's left. */
  path: string;
}

export interface LaneHeading {
  lane: TopologyLane;
  label: string;
  x: number;
  /** How many nodes stand in this lane. A lane with none is still drawn, so
   *  the reader can see that a namespace has no Ingress rather than wonder
   *  where the column went. */
  count: number;
}

export interface TopologyLayout {
  nodes: PlacedNode[];
  edges: PlacedEdge[];
  lanes: LaneHeading[];
  width: number;
  height: number;
}

function laneX(index: number): number {
  return index * (NODE_WIDTH + LANE_GAP);
}

function rowY(index: number): number {
  return HEADER_HEIGHT + index * (NODE_HEIGHT + ROW_GAP);
}

/**
 * Order one lane so its edges cross as little as possible.
 *
 * The barycentre heuristic, which is the standard answer for layered graphs and
 * is enough here: a node sits at the average row of everything it is joined to,
 * so a Deployment lands beside the Service that fronts it instead of wherever
 * the API happened to list it. Nodes joined to nothing keep their name order
 * and go last — an orphan has no opinion about where it belongs, and putting it
 * first would push everything with a reason to be somewhere away from that
 * place.
 *
 * `placedRows` is whatever is known so far, which is why {@link layoutGraph}
 * calls this twice: neighbours to the left on the first sweep, all of them on
 * the second. A fixed number of sweeps rather than iterating to a fixed point,
 * because the same graph must lay out identically every render or the picture
 * moves under the reader.
 */
export function orderLane(
  lane: TopologyNode[],
  neighbours: Map<string, string[]>,
  placedRows: Map<string, number>,
): TopologyNode[] {
  const byName = [...lane].sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
  const barycentre = new Map<string, number>();
  for (const node of byName) {
    const rows = (neighbours.get(node.id) ?? [])
      .map((other) => placedRows.get(other))
      .filter((row): row is number => row !== undefined);
    if (rows.length > 0) {
      barycentre.set(node.id, rows.reduce((a, b) => a + b, 0) / rows.length);
    }
  }
  return byName.sort((a, b) => {
    const av = barycentre.get(a.id);
    const bv = barycentre.get(b.id);
    // An anchored node always precedes a free one, so orphans collect at the
    // bottom of their lane rather than splitting a run of connected rows.
    if (av === undefined && bv === undefined) return 0;
    if (av === undefined) return 1;
    if (bv === undefined) return -1;
    return av - bv;
  });
}

/**
 * A cubic from one node's right edge to another's left, flat at both ends so it
 * leaves and arrives horizontally however far apart the rows are.
 *
 * An edge that points LEFT — a workload calling a Service that stands in an
 * earlier column, which is what a declared dependency on something in this same
 * namespace looks like — cannot be drawn that way: a straight run backwards
 * crosses every column between the two and reads as noise. Those leave from
 * underneath and arrive from underneath instead, bowing below the rows they
 * pass, so a reader can follow one without tracing it through the middle of
 * everything else.
 */
export function edgePath(from: PlacedNode, to: PlacedNode): string {
  const forward = to.x > from.x;
  if (forward) {
    const x1 = from.x + NODE_WIDTH;
    const y1 = from.y + NODE_HEIGHT / 2;
    const x2 = to.x;
    const y2 = to.y + NODE_HEIGHT / 2;
    const bend = Math.max((x2 - x1) / 2, 24);
    return `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`;
  }
  const x1 = from.x + NODE_WIDTH / 2;
  const y1 = from.y + NODE_HEIGHT;
  const x2 = to.x + NODE_WIDTH / 2;
  const y2 = to.y + NODE_HEIGHT;
  // Deep enough to clear the taller of the two nodes' rows, and deeper the
  // further it has to travel, so long back-references do not sit on top of
  // short ones.
  const drop = Math.max(NODE_HEIGHT, Math.abs(x2 - x1) / 6) + ROW_GAP;
  return `M ${x1} ${y1} C ${x1} ${y1 + drop}, ${x2} ${y2 + drop}, ${x2} ${y2}`;
}

/**
 * Place a whole graph.
 *
 * Every node the backend returned is placed, including ones no edge touches:
 * a Service fronting nothing and a workload no Service selects are both real
 * findings, and dropping them would quietly answer "there is nothing there".
 */
export function layoutGraph(graph: TopologyGraph): TopologyLayout {
  /**
   * Both directions, deliberately.
   *
   * Ordering by INCOMING edges alone was the first version and it laid the
   * declared dependencies out badly: a Service that exists only because
   * something calls it — `payments-api` in another namespace — has no incoming
   * edge from a lane to its left, so it fell to the bottom of the service
   * column while its caller sat at the top of the workload column, and the
   * edge cut diagonally across the whole diagram. A node belongs beside
   * everything it is joined to, whichever way the arrow points.
   */
  const neighbours = new Map<string, string[]>();
  const join = (a: string, b: string) => {
    const list = neighbours.get(a);
    if (list) list.push(b);
    else neighbours.set(a, [b]);
  };
  for (const edge of graph.edges) {
    join(edge.to, edge.from);
    join(edge.from, edge.to);
  }

  const lanes: LaneHeading[] = [];
  const members = LANES.map((lane) => graph.nodes.filter((n) => n.lane === lane));
  LANES.forEach((lane, i) => {
    lanes.push({ lane, label: LANE_LABELS[lane], x: laneX(i), count: members[i].length });
  });

  /**
   * Two sweeps, not one.
   *
   * On the first, a lane can only be positioned by neighbours already placed to
   * its left. The second runs with every row known, so a lane is finally placed
   * against BOTH sides — which is what actually lines a caller up with the
   * dependency it calls. Two is enough at this size, and a fixed number of
   * passes keeps the layout deterministic: the same graph must lay out
   * identically every render, or the picture moves under the reader.
   */
  let rows = new Map<string, number>();
  let ordered: TopologyNode[][] = members;
  for (let sweep = 0; sweep < 2; sweep++) {
    const next = new Map<string, number>();
    ordered = members.map((lane) => {
      const laid = orderLane(lane, neighbours, sweep === 0 ? next : rows);
      laid.forEach((node, row) => next.set(node.id, row));
      return laid;
    });
    rows = next;
  }

  const placed: PlacedNode[] = [];
  let tallest = 0;
  ordered.forEach((lane, laneIndex) => {
    lane.forEach((node, row) => {
      placed.push({ ...node, x: laneX(laneIndex), y: rowY(row) });
    });
    tallest = Math.max(tallest, lane.length);
  });

  const byId = new Map(placed.map((n) => [n.id, n]));
  const edges: PlacedEdge[] = [];
  for (const edge of graph.edges) {
    const from = byId.get(edge.from);
    const to = byId.get(edge.to);
    // An edge whose endpoints are not both drawn has nothing to connect. The
    // backend does not emit these, and drawing a line into empty space if it
    // ever did would be worse than dropping it.
    if (!from || !to) continue;
    edges.push({ ...edge, path: edgePath(from, to) });
  }

  // A back-reference bows below the last row, so the box has to leave room for
  // it or the curve is clipped by the viewBox.
  const bowing = edges.some((e) => {
    const from = byId.get(e.from);
    const to = byId.get(e.to);
    return from !== undefined && to !== undefined && to.x <= from.x;
  });
  const rowsHeight = tallest === 0 ? HEADER_HEIGHT : rowY(tallest - 1) + NODE_HEIGHT;
  return {
    nodes: placed,
    edges,
    lanes,
    width: laneX(LANES.length - 1) + NODE_WIDTH,
    height: rowsHeight + (bowing ? BACKWARD_CLEARANCE : 0),
  };
}

/**
 * Cut a name to what the box can hold.
 *
 * SVG text does not wrap and does not clip to its parent, so a long host ran
 * straight out of its node and over the next column — `orders-db.internal.
 * example.com` was drawn as `orders-db.internal.example.co` lying across the
 * border. The full name stays reachable: the node carries it as a `title`, and
 * the inspector never truncates.
 */
export function fit(name: string, max = 22): string {
  return name.length <= max ? name : `${name.slice(0, max - 1)}…`;
}
