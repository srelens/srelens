import { describe, expect, it } from "vitest";
import type { TopologyEdge, TopologyGraph, TopologyLane, TopologyNode } from "@srelens/core";
import { HEADER_HEIGHT, LANES, NODE_WIDTH, layoutGraph, orderLane } from "./topologyLayout";

function node(id: string, lane: TopologyLane, name = id): TopologyNode {
  return {
    id,
    kind: lane,
    name,
    namespace: "checkout",
    lane,
    detail: "",
    ready: null,
    desired: null,
    health: "unknown",
  };
}

function edge(from: string, to: string): TopologyEdge {
  return { from, to, kind: "routes", health: "ok" };
}

function graph(nodes: TopologyNode[], edges: TopologyEdge[] = []): TopologyGraph {
  return { nodes, edges };
}

describe("layoutGraph", () => {
  it("puts each lane in its own column, in the fixed order", () => {
    const out = layoutGraph(
      graph([
        node("r", "route"),
        node("s", "service"),
        node("w", "workload"),
        node("rs", "replicaset"),
      ]),
    );
    const x = (id: string) => out.nodes.find((n) => n.id === id)!.x;
    expect(x("r")).toBeLessThan(x("s"));
    expect(x("s")).toBeLessThan(x("w"));
    expect(x("w")).toBeLessThan(x("rs"));
    expect(out.lanes.map((l) => l.lane)).toEqual([...LANES]);
  });

  it("draws every lane heading, including one with nothing in it", () => {
    // A namespace with no Ingress should read as "no route", not as a column
    // that vanished.
    const out = layoutGraph(graph([node("w", "workload")]));
    expect(out.lanes).toHaveLength(5);
    expect(out.lanes.find((l) => l.lane === "route")?.count).toBe(0);
    expect(out.lanes.find((l) => l.lane === "workload")?.count).toBe(1);
  });

  it("places a node beside the thing that points at it", () => {
    // Two services, two deployments. Without the barycentre pass the
    // deployments would sit in name order — b above a — and both edges would
    // cross.
    const out = layoutGraph(
      graph(
        [
          node("Service/a", "service", "a"),
          node("Service/b", "service", "b"),
          node("Deploy/b", "workload", "b"),
          node("Deploy/a", "workload", "a"),
        ],
        [edge("Service/a", "Deploy/a"), edge("Service/b", "Deploy/b")],
      ),
    );
    const y = (id: string) => out.nodes.find((n) => n.id === id)!.y;
    expect(y("Deploy/a")).toBe(y("Service/a"));
    expect(y("Deploy/b")).toBe(y("Service/b"));
  });

  it("keeps a node nothing points at, and sends it to the end of its lane", () => {
    // A Service fronting no workload is a real finding. Dropping it would
    // answer "there is nothing there"; putting it first would push the rows
    // that do have a reason to be somewhere away from it.
    const out = layoutGraph(
      graph(
        [
          node("Service/anchored", "service", "anchored"),
          node("Service/orphan", "service", "orphan"),
          node("Route/r", "route", "r"),
        ],
        [edge("Route/r", "Service/anchored")],
      ),
    );
    const y = (id: string) => out.nodes.find((n) => n.id === id)!.y;
    expect(out.nodes).toHaveLength(3);
    expect(y("Service/anchored")).toBeLessThan(y("Service/orphan"));
  });

  it("is stable — the same graph lays out identically twice", () => {
    // The screen re-reads on a poll. A layout that shuffled would move the
    // picture under the reader every time.
    const g = graph(
      [node("Service/a", "service", "a"), node("Service/b", "service", "b"), node("Deploy/a", "workload", "a")],
      [edge("Service/a", "Deploy/a")],
    );
    expect(layoutGraph(g)).toEqual(layoutGraph(g));
  });

  it("draws an edge from the source's right side to the target's left", () => {
    const out = layoutGraph(
      graph([node("s", "service"), node("w", "workload")], [edge("s", "w")]),
    );
    const from = out.nodes.find((n) => n.id === "s")!;
    const to = out.nodes.find((n) => n.id === "w")!;
    expect(out.edges).toHaveLength(1);
    expect(out.edges[0].path.startsWith(`M ${from.x + NODE_WIDTH} `)).toBe(true);
    expect(out.edges[0].path.endsWith(` ${to.x} ${to.y + 26}`)).toBe(true);
  });

  it("drops an edge whose endpoints are not both drawn", () => {
    const out = layoutGraph(graph([node("s", "service")], [edge("s", "gone")]));
    expect(out.edges).toEqual([]);
  });

  it("reports an empty graph as empty rather than as a negative box", () => {
    const out = layoutGraph(graph([]));
    expect(out.nodes).toEqual([]);
    expect(out.height).toBe(HEADER_HEIGHT);
    expect(out.lanes).toHaveLength(5);
  });
});

describe("orderLane", () => {
  it("falls back to name order when nothing points at anything", () => {
    const lane = [node("b", "service", "b"), node("a", "service", "a")];
    expect(orderLane(lane, new Map(), new Map()).map((n) => n.name)).toEqual(["a", "b"]);
  });

  it("averages several incoming rows rather than taking the first", () => {
    // A workload two Services front belongs between them, not beside whichever
    // Service the API listed first.
    const lane = [node("low", "workload", "low"), node("mid", "workload", "mid")];
    const incoming = new Map([
      ["mid", ["s0", "s4"]],
      ["low", ["s1"]],
    ]);
    const rows = new Map([
      ["s0", 0],
      ["s1", 1],
      ["s4", 4],
    ]);
    // mid averages 2, low is 1 — so low comes first.
    expect(orderLane(lane, incoming, rows).map((n) => n.name)).toEqual(["low", "mid"]);
  });
});
