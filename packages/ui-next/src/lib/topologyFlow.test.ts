import { describe, expect, it } from "vitest";
import type { TopologyEdge, TopologyGraph, TopologyLane, TopologyNode } from "@srelens/core";
import {
  MAX_EDGE_WIDTH,
  MIN_EDGE_WIDTH,
  MIN_ZOOM,
  NODE_WIDTH,
  arrowPoints,
  backEdges,
  edgeWidths,
  fitTransform,
  fold,
  layoutFlow,
  orderColumn,
  rankNodes,
  traceFrom,
  zoomAt,
} from "./topologyFlow";

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

function edge(from: string, to: string, over: Partial<TopologyEdge> = {}): TopologyEdge {
  return {
    from,
    to,
    kind: "routes",
    provenance: "topology",
    detail: "",
    health: "ok",
    weight: null,
    unit: null,
    ...over,
  };
}

function graph(nodes: TopologyNode[], edges: TopologyEdge[] = []): TopologyGraph {
  return { nodes, edges };
}

/** One tier: a way in, an address, and the pods that answer it. */
function tier(name: string, into?: string): TopologyGraph {
  return graph(
    [
      node(`Service/${name}`, "service", name),
      node(`Deployment/${name}`, "workload", name),
    ],
    [
      edge(`Service/${name}`, `Deployment/${name}`),
      ...(into
        ? [edge(`Deployment/${name}`, `Service/${into}`, { kind: "calls" as const })]
        : []),
    ],
  );
}

/** Two tiers, one calling the other. */
function twoTiers(): TopologyGraph {
  const a = tier("checkout", "payments");
  const b = tier("payments");
  return graph([...a.nodes, ...b.nodes], [...a.edges, ...b.edges]);
}

/** The shape almost every namespace has: a way in, a Service, a workload. */
function chain(): TopologyGraph {
  return graph(
    [
      node("Ingress/web", "route", "web"),
      node("Service/checkout", "service", "checkout"),
      node("Deployment/checkout", "workload", "checkout"),
    ],
    [edge("Ingress/web", "Service/checkout"), edge("Service/checkout", "Deployment/checkout")],
  );
}

describe("rankNodes", () => {
  it("counts calls as hops and routing as none", () => {
    // A Service is an address, not a stop. Charging `routes` a column each
    // drew the demo namespace eight columns wide and two rows tall, every
    // column a Service followed by the one Deployment it fronts — the same
    // tier written twice, and unreadable at the zoom it took to fit.
    const g = chain();
    const rank = rankNodes(fold(g).nodes, g.edges);
    expect(rank.get("Ingress/web")).toBe(0);
    expect(rank.get("Service/checkout")).toBe(0);
    expect(rank.get("Deployment/checkout")).toBe(0);
  });

  it("carries a cross-service call forward instead of backwards", () => {
    // The whole reason this layout exists. Under the old kind-per-column
    // layout `payments`' Service sat in the service column — to the LEFT of
    // the workload calling it — so the call was drawn as a backward bow. Here
    // it is simply the next tier.
    const rank = rankNodes(fold(twoTiers()).nodes, twoTiers().edges);
    expect(rank.get("Deployment/checkout")).toBe(0);
    expect(rank.get("Service/payments")).toBe(1);
    expect(rank.get("Deployment/payments")).toBe(1);
  });

  it("takes the longest way round, not the shortest", () => {
    // `late` is called directly and again at the end of a three-call chain.
    // Ranking it by the short way would put it in front of its own caller and
    // draw the long chain backwards — the exact failure this replaced.
    const call = { kind: "calls" as const };
    const g = graph(
      [node("a", "service"), node("b", "service"), node("c", "service"), node("late", "service")],
      [
        edge("a", "late", call),
        edge("a", "b", call),
        edge("b", "c", call),
        edge("c", "late", call),
      ],
    );
    const rank = rankNodes(fold(g).nodes, g.edges);
    expect(rank.get("late")).toBe(3);
  });

  it("moves a Service along when it is called as well as routed to", () => {
    // The same pair joined both ways takes the higher cost. A Service someone
    // calls is a tier of its own however it is also reached.
    const g = graph(
      [node("a", "workload"), node("b", "service")],
      [edge("a", "b"), edge("a", "b", { kind: "calls" })],
    );
    expect(rankNodes(fold(g).nodes, g.edges).get("b")).toBe(1);
  });

  it("terminates on a cycle rather than ranking forever", () => {
    // Two services that call each other is an ordinary mesh, and a longest
    // path over it does not exist.
    const g = graph(
      [node("a", "service"), node("b", "service")],
      [edge("a", "b", { kind: "calls" }), edge("b", "a", { kind: "calls" })],
    );
    const rank = rankNodes(fold(g).nodes, g.edges);
    expect(rank.get("a")).toBe(0);
    expect(rank.get("b")).toBe(1);
  });

  it("does not let ownership set a rank", () => {
    // A ReplicaSet is not a hop. It is folded away before this ever runs, and
    // an `owns` edge that survived would push its target a column right for a
    // reason that has nothing to do with traffic.
    const g = graph(
      [node("Deployment/a", "workload", "a"), node("Service/a", "service", "a")],
      [edge("Deployment/a", "Service/a", { kind: "owns" })],
    );
    const rank = rankNodes(fold(g).nodes, g.edges);
    expect(rank.get("Service/a")).toBe(0);
  });
});

describe("backEdges", () => {
  it("finds the arc that closes a loop and no other", () => {
    const out = new Map([
      ["a", ["b"]],
      ["b", ["c"]],
      ["c", ["a"]],
    ]);
    expect([...backEdges(["a", "b", "c"], out)]).toEqual(["c->a"]);
  });

  it("does not mistake a diamond for a cycle", () => {
    // Two paths to the same node is the commonest shape on a real graph, and
    // calling the second one a cycle would drop a real edge from the ranking.
    const out = new Map([
      ["a", ["b", "c"]],
      ["b", ["d"]],
      ["c", ["d"]],
    ]);
    expect([...backEdges(["a", "b", "c", "d"], out)]).toEqual([]);
  });
});

describe("fold", () => {
  it("folds a ReplicaSet into the workload that owns it", () => {
    const g = graph(
      [
        node("Deployment/checkout", "workload", "checkout"),
        node("RS/checkout-a", "replicaset", "rev 9"),
        node("RS/checkout-b", "replicaset", "rev 119"),
      ],
      [
        edge("Deployment/checkout", "RS/checkout-a", { kind: "owns" }),
        edge("Deployment/checkout", "RS/checkout-b", { kind: "owns" }),
      ],
    );
    const out = fold(g);
    expect(out.nodes).toHaveLength(1);
    // Newest first, by number — a string sort would put `rev 9` above
    // `rev 119`, which is the wrong end of a rollout to show first.
    expect(out.nodes[0].revisions).toEqual(["rev 119", "rev 9"]);
    expect(out.edges).toEqual([]);
  });

  it("keeps a ReplicaSet nobody owns", () => {
    // Something applied a bare ReplicaSet, or its Deployment is gone. Either
    // is a real finding, and folding it into nothing would delete it.
    const g = graph([node("RS/orphan", "replicaset", "rev 1")]);
    expect(fold(g).nodes.map((n) => n.id)).toEqual(["RS/orphan"]);
  });
});

describe("layoutFlow", () => {
  it("puts each hop in its own column, left to right", () => {
    const out = layoutFlow(twoTiers());
    const x = (id: string) => out.nodes.find((n) => n.id === id)!.x;
    expect(x("Deployment/checkout")).toBeLessThan(x("Service/payments"));
    expect(out.columns.map((c) => c.label)).toEqual(["ENTRY", "HOP 1"]);
  });

  it("keeps a tier together, in the order traffic passes through it", () => {
    // A Service and the pods answering it share a column, so they have to be
    // adjacent rows: the same two six rows apart, joined by a line running
    // past four unrelated boxes, would be worse than the layout with twice the
    // columns.
    const g = graph(
      [
        ...chain().nodes,
        node("Service/other", "service", "other"),
        node("Deployment/other", "workload", "other"),
      ],
      [...chain().edges, edge("Service/other", "Deployment/other")],
    );
    const out = layoutFlow(g);
    const rows = out.nodes
      .slice()
      .sort((a, b) => a.y - b.y)
      .map((n) => n.id);
    // Each tier is consecutive, and inside one the order is the order traffic
    // passes through: the way in, then the address, then what answers.
    const at = (id: string) => rows.indexOf(id);
    expect(at("Service/checkout")).toBe(at("Ingress/web") + 1);
    expect(at("Deployment/checkout")).toBe(at("Ingress/web") + 2);
    expect(at("Deployment/other")).toBe(at("Service/other") + 1);
  });

  it("joins a tier with a short stub rather than a backward bow", () => {
    // The one line on the diagram that is not a network call. It reads as a
    // bracket holding a group together, and it must not be mistaken for the
    // bow that means a cycle.
    const out = layoutFlow(chain());
    const stub = out.edges.find((e) => e.from === "Service/checkout")!;
    const from = out.nodes.find((n) => n.id === "Service/checkout")!;
    // Straight down the middle of the column, not out to the side and back.
    expect(stub.path.startsWith(`M ${from.x + NODE_WIDTH / 2} `)).toBe(true);
    expect(stub.path).toContain(" L ");
  });

  it("puts a tier no call touches in the band, not in a column", () => {
    // The first real cluster: no mesh, no metrics, probe off, so almost no
    // calls were known and every tier ranked zero. Every placement was
    // correct and the picture was one column twenty tiers tall.
    const out = layoutFlow(graph([node("a", "service"), node("b", "service")]));
    expect(out.columns).toEqual([]);
    expect(out.band?.count).toBe(2);
    expect(out.nodes.every((n) => n.rank === null)).toBe(true);
  });

  it("packs the band into a grid rather than a column", () => {
    const many = Array.from({ length: 12 }, (_, i) => node(`s${i}`, "service", `svc-${i}`));
    const out = layoutFlow(graph(many));
    const xs = new Set(out.nodes.map((n) => n.x));
    const ys = new Set(out.nodes.map((n) => n.y));
    expect(xs.size).toBeGreaterThan(1);
    expect(ys.size).toBeLessThan(12);
    expect(out.width).toBeGreaterThan(out.height);
  });

  it("keeps an Ingress-fronted tier in the flow even before any call is known", () => {
    // Traffic enters there whether or not anything downstream has been seen.
    const out = layoutFlow(chain());
    expect(out.columns.map((c) => c.label)).toEqual(["ENTRY"]);
    expect(out.band).toBeNull();
  });

  it("grows the band to the width of the flow above it", () => {
    // So the two read as one picture rather than a wide flow over a narrow
    // stack.
    const g = twoTiers();
    const loose = Array.from({ length: 4 }, (_, i) => node(`l${i}`, "service", `loose-${i}`));
    const out = layoutFlow(graph([...g.nodes, ...loose], g.edges));
    const flowRight = Math.max(...out.nodes.filter((n) => n.rank !== null).map((n) => n.x));
    const bandRight = Math.max(...out.nodes.filter((n) => n.rank === null).map((n) => n.x));
    expect(bandRight).toBeGreaterThanOrEqual(flowRight);
    // And it sits below the flow, not beside it.
    const flowBottom = Math.max(...out.nodes.filter((n) => n.rank !== null).map((n) => n.y));
    expect(out.band?.y).toBeGreaterThan(flowBottom);
  });

  it("places a tier beside the one that calls it", () => {
    // Without the barycentre pass the called tiers would sit in name order — b
    // above a — and both calls would cross.
    const call = { kind: "calls" as const };
    const out = layoutFlow(
      graph(
        [
          node("Caller/a", "workload", "a"),
          node("Caller/b", "workload", "b"),
          node("Called/b", "service", "b"),
          node("Called/a", "service", "a"),
        ],
        [edge("Caller/a", "Called/a", call), edge("Caller/b", "Called/b", call)],
      ),
    );
    const y = (id: string) => out.nodes.find((n) => n.id === id)!.y;
    expect(y("Called/a")).toBe(y("Caller/a"));
    expect(y("Called/b")).toBe(y("Caller/b"));
  });

  it("keeps a node nothing points at", () => {
    // A Service fronting no workload is a real finding. Dropping it would
    // answer "there is nothing there".
    const out = layoutFlow(
      graph(
        [node("Service/orphan", "service", "orphan"), node("Route/r", "route", "r")],
        [],
      ),
    );
    expect(out.nodes).toHaveLength(2);
  });

  it("is stable — the same graph lays out identically twice", () => {
    // The screen re-reads when the selection changes. A layout that shuffled
    // would move the picture under the reader every time.
    const g = chain();
    expect(layoutFlow(g)).toEqual(layoutFlow(g));
  });

  it("draws a forward edge from the source's right side to the target's left", () => {
    const out = layoutFlow(twoTiers());
    const from = out.nodes.find((n) => n.id === "Deployment/checkout")!;
    const drawn = out.edges.find((e) => e.kind === "calls")!;
    expect(drawn.path.startsWith(`M ${from.x + NODE_WIDTH} `)).toBe(true);
    expect(drawn.arrow.split(" ")).toHaveLength(3);
  });

  it("drops an edge whose endpoints are not both drawn", () => {
    const out = layoutFlow(graph([node("s", "service")], [edge("s", "gone")]));
    expect(out.edges).toEqual([]);
  });

  it("lists the namespaces it drew, so the screen knows to label them", () => {
    const out = layoutFlow(
      graph([node("a", "service"), { ...node("b", "service"), namespace: "payments" }]),
    );
    expect(out.namespaces).toEqual(["checkout", "payments"]);
  });

  it("reports an empty graph as empty rather than as a negative box", () => {
    const out = layoutFlow(graph([]));
    expect(out.nodes).toEqual([]);
    expect(out.columns).toEqual([]);
    expect(out.width).toBeGreaterThan(0);
  });
});

describe("orderColumn", () => {
  it("falls back to name order when nothing points at anything", () => {
    const column = fold(graph([node("b", "service", "b"), node("a", "service", "a")])).nodes;
    expect(orderColumn(column, new Map(), new Map()).map((n) => n.name)).toEqual(["a", "b"]);
  });

  it("averages several neighbouring rows rather than taking the first", () => {
    const column = fold(graph([node("low", "workload", "low"), node("mid", "workload", "mid")]))
      .nodes;
    const neighbours = new Map([
      ["mid", ["s0", "s4"]],
      ["low", ["s1"]],
    ]);
    const rows = new Map([
      ["s0", 0],
      ["s1", 1],
      ["s4", 4],
    ]);
    // mid averages 2, low is 1 — so low comes first.
    expect(orderColumn(column, neighbours, rows).map((n) => n.name)).toEqual(["low", "mid"]);
  });
});

describe("edgeWidths", () => {
  it("draws a busier edge thicker than a quiet one", () => {
    const edges = [
      edge("a", "b", { weight: 100, unit: "rps" }),
      edge("c", "d", { weight: 1, unit: "rps" }),
    ];
    const widths = edgeWidths(edges);
    expect(widths.get("routes:a->b")).toBe(MAX_EDGE_WIDTH);
    expect(widths.get("routes:c->d")).toBeLessThan(MAX_EDGE_WIDTH);
    expect(widths.get("routes:c->d")).toBeGreaterThanOrEqual(MIN_EDGE_WIDTH);
  });

  it("never scales connections against a rate", () => {
    // Five open connections and five requests a second are different
    // quantities. Putting them on one scale would draw a comparison that does
    // not exist — so each is the only thing in its group and both get the
    // middle width.
    const widths = edgeWidths([
      edge("a", "b", { weight: 5, unit: "rps" }),
      edge("c", "d", { weight: 5, unit: "connections" }),
    ]);
    expect(widths.get("routes:a->b")).toBe(widths.get("routes:c->d"));
  });

  it("gives a lone measurement the middle width, not the maximum", () => {
    // Thickest-line-on-the-diagram is a claim about a comparison, and with one
    // measured edge there is nothing to compare it against.
    const widths = edgeWidths([edge("a", "b", { weight: 900, unit: "rps" })]);
    expect(widths.get("routes:a->b")).toBe((MIN_EDGE_WIDTH + MAX_EDGE_WIDTH) / 2);
  });

  it("leaves an unmeasured edge out entirely", () => {
    expect(edgeWidths([edge("a", "b")]).size).toBe(0);
  });
});

describe("traceFrom", () => {
  it("follows the whole chain both ways, not one hop", () => {
    // "If this is broken, what else is" is transitive. One hop answers a
    // different and less useful question.
    const out = layoutFlow(
      graph(
        [node("a", "route"), node("b", "service"), node("c", "workload"), node("d", "service")],
        [edge("a", "b"), edge("b", "c"), edge("c", "d")],
      ),
    );
    const trace = traceFrom("b", out.edges);
    expect([...trace.downstream].sort()).toEqual(["c", "d"]);
    expect([...trace.upstream]).toEqual(["a"]);
    expect(trace.nodes.has("b")).toBe(true);
  });

  it("leaves out what the selection has nothing to do with", () => {
    const out = layoutFlow(
      graph(
        [node("a", "service"), node("b", "service"), node("far", "service")],
        [edge("a", "b")],
      ),
    );
    expect(traceFrom("a", out.edges).nodes.has("far")).toBe(false);
  });

  it("terminates on a cycle", () => {
    const out = layoutFlow(
      graph(
        [node("a", "service"), node("b", "service")],
        [edge("a", "b", { kind: "calls" }), edge("b", "a", { kind: "calls" })],
      ),
    );
    const trace = traceFrom("a", out.edges);
    expect(trace.nodes.has("b")).toBe(true);
    expect(trace.edges.size).toBe(2);
  });
});

describe("fitTransform", () => {
  it("centres the drawing in the frame", () => {
    const out = fitTransform({ width: 100, height: 100 }, { width: 400, height: 200 });
    expect(out.k).toBe(1);
    expect(out.tx).toBe(150);
    expect(out.ty).toBe(50);
  });

  it("shrinks a graph too big for the frame", () => {
    const out = fitTransform({ width: 2000, height: 100 }, { width: 400, height: 400 });
    expect(out.k).toBeCloseTo(0.2);
  });

  it("never blows a small graph up past 1:1", () => {
    // Two nodes scaled to fill a wide pane looks like a bug, and the fact that
    // there is very little here is itself worth seeing.
    expect(fitTransform({ width: 10, height: 10 }, { width: 900, height: 900 }).k).toBe(1);
  });

  it("survives a frame that has not been measured yet", () => {
    // jsdom, and the first paint in a real browser, both report zero.
    expect(fitTransform({ width: 100, height: 100 }, { width: 0, height: 0 })).toEqual({
      k: 1,
      tx: 0,
      ty: 0,
    });
  });

  it("fits a graph too wide even for the manual zoom floor", () => {
    // Fit is allowed below MIN_ZOOM. That floor stops a reader zooming
    // themselves into an unreadable smear by accident; a button labelled Fit
    // that then does not fit would be the worse lie.
    const out = fitTransform({ width: 100000, height: 10 }, { width: 100, height: 100 });
    expect(out.k).toBeLessThan(MIN_ZOOM);
    expect(100000 * out.k).toBeLessThanOrEqual(100);
  });
});

describe("zoomAt", () => {
  it("keeps the point under the pointer where it is", () => {
    // Zooming about the origin instead is the small wrongness that makes a
    // canvas feel broken: the reader aims at a node and it leaves the screen.
    const before = { k: 1, tx: 0, ty: 0 };
    const after = zoomAt(before, 2, 100, 50);
    // The graph coordinate under (100, 50) was (100, 50); it must still be.
    expect((100 - after.tx) / after.k).toBeCloseTo(100);
    expect((50 - after.ty) / after.k).toBeCloseTo(50);
  });

  it("stops at the floor and the ceiling", () => {
    expect(zoomAt({ k: 1, tx: 0, ty: 0 }, 100, 0, 0).k).toBeLessThanOrEqual(2.5);
    expect(zoomAt({ k: 1, tx: 0, ty: 0 }, 0.001, 0, 0).k).toBe(MIN_ZOOM);
  });
});

describe("arrowPoints", () => {
  it("points the way it is given, and sits at the tip", () => {
    // Direction is the entire subject of this diagram; the layout it replaced
    // drew no arrowheads at all.
    const right = arrowPoints(100, 50, 1, 0).split(" ").map((p) => p.split(",").map(Number));
    expect(right[0]).toEqual([100, 50]);
    // Both other corners sit behind the tip, on the same side.
    expect(right[1][0]).toBeLessThan(100);
    expect(right[2][0]).toBeLessThan(100);
    expect(right[1][1]).not.toBe(right[2][1]);
  });

  it("turns with the line", () => {
    const up = arrowPoints(0, 0, 0, -1).split(" ").map((p) => p.split(",").map(Number));
    expect(up[1][1]).toBeGreaterThan(0);
    expect(up[2][1]).toBeGreaterThan(0);
  });
});
