import { describe, expect, it } from "vitest";
import type { TopologyEdge, TopologyGraph, TopologyLane, TopologyNode } from "@srelens/core";
import {
  GRID_GAP,
  MAX_EDGE_WIDTH,
  MIN_EDGE_WIDTH,
  MIN_ZOOM,
  NODE_HEIGHT,
  NODE_WIDTH,
  arrowPoints,
  backEdges,
  edgeWidths,
  fitTransform,
  fold,
  hubCounts,
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
    // The Ingress has the first column to itself; the tier it fronts is the
    // entry level behind it, Service and pods together.
    const g = chain();
    const rank = rankNodes(fold(g).nodes, g.edges);
    expect(rank.get("Ingress/web")).toBe(0);
    expect(rank.get("Service/checkout")).toBe(1);
    expect(rank.get("Deployment/checkout")).toBe(1);
    // Without any Ingress there is no such column, and entries are rank zero.
    const bare = rankNodes(fold(twoTiers()).nodes, twoTiers().edges);
    expect(bare.get("Service/checkout")).toBe(0);
    expect(bare.get("Service/payments")).toBe(1);
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

  it("does not let a call inside a tier split the tier", () => {
    // A real namespace: a StatefulSet names its own headless Service in its
    // config. Ranked node by node, the Service was called — rank one — while
    // the StatefulSet it routes to sat at rank zero beside the other Service
    // fronting it: one tier drawn across two columns, joined by a bow. The
    // tier is the unit that ranks, and a call from a tier to itself is not a
    // hop.
    const g = graph(
      [
        node("Service/db", "service", "db"),
        node("Service/db-h", "service", "db-h"),
        node("StatefulSet/db", "workload", "db"),
        node("Deployment/app", "workload", "app"),
      ],
      [
        edge("Service/db", "StatefulSet/db"),
        edge("Service/db-h", "StatefulSet/db"),
        edge("StatefulSet/db", "Service/db-h", { kind: "calls", provenance: "declared" }),
        edge("Deployment/app", "Service/db-h", { kind: "calls", provenance: "declared" }),
      ],
    );
    const rank = rankNodes(fold(g).nodes, g.edges);
    expect(rank.get("Deployment/app")).toBe(0);
    expect(rank.get("Service/db")).toBe(1);
    expect(rank.get("Service/db-h")).toBe(1);
    expect(rank.get("StatefulSet/db")).toBe(1);
  });

  it("keeps an Ingress-fronted tier at the entry however much calls it from inside", () => {
    // Ten login.* Ingresses fronted an auth Service half the namespace also
    // called, so the tier ranked one and the front door was drawn a hop in.
    const g = graph(
      [
        node("Ingress/login", "route", "login"),
        node("Service/auth", "service", "auth"),
        node("Deployment/auth", "workload", "auth"),
        node("Service/app", "service", "app"),
        node("Deployment/app", "workload", "app"),
      ],
      [
        edge("Ingress/login", "Service/auth"),
        edge("Service/auth", "Deployment/auth"),
        edge("Service/app", "Deployment/app"),
        edge("Deployment/app", "Service/auth", { kind: "calls", provenance: "declared" }),
      ],
    );
    const rank = rankNodes(fold(g).nodes, g.edges);
    expect(rank.get("Ingress/login")).toBe(0);
    expect(rank.get("Service/auth")).toBe(1);
    expect(rank.get("Deployment/app")).toBe(1);
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
    // Both Ingress-fronted, so both stand in ENTRY rather than one of them
    // going to the band for having no known call.
    const g = graph(
      [
        ...chain().nodes,
        node("Ingress/other", "route", "other"),
        node("Service/other", "service", "other"),
        node("Deployment/other", "workload", "other"),
      ],
      [
        ...chain().edges,
        edge("Ingress/other", "Service/other"),
        edge("Service/other", "Deployment/other"),
      ],
    );
    const out = layoutFlow(g);
    const at = (id: string) => out.nodes.find((n) => n.id === id)!;
    // The order traffic passes through: the way in, then the address with
    // what answers beside it.
    expect(at("Service/checkout").x).toBeGreaterThan(at("Ingress/web").x);
    expect(at("Deployment/checkout").y).toBe(at("Service/checkout").y);
    expect(at("Deployment/checkout").x).toBeGreaterThan(at("Service/checkout").x);
    expect(at("Deployment/other").y).toBe(at("Service/other").y);
    // Two tiers, two panels, and they do not overlap.
    expect(out.tiers).toHaveLength(2);
    const [a, b] = out.tiers.slice().sort((p, q) => p.x - q.x);
    expect(b.x).toBeGreaterThanOrEqual(a.x + a.width);
  });

  it("fans the entry tiers that call nothing out beside the ones that do", () => {
    // Three Ingress-fronted tiers stacked ten rows tall beside a two-hop flow
    // was a picture taller than it was wide. Nothing arrives at ENTRY from
    // the left, so its leaf tiers can stand there without crossing an edge.
    const flow = twoTiers();
    const leaf = (name: string) => ({
      nodes: [
        node(`Ingress/${name}`, "route", name),
        node(`Service/${name}`, "service", name),
        node(`Deployment/${name}`, "workload", name),
      ],
      edges: [edge(`Ingress/${name}`, `Service/${name}`), edge(`Service/${name}`, `Deployment/${name}`)],
    });
    const leaves = ["docs", "status", "legacy"].map(leaf);
    const out = layoutFlow(
      graph(
        [...flow.nodes, ...leaves.flatMap((l) => l.nodes)],
        [...flow.edges, ...leaves.flatMap((l) => l.edges)],
      ),
    );
    const x = (id: string) => out.nodes.find((n) => n.id === id)!.x;
    // The caller stays against HOP 1; every leaf stands to its left, spread
    // across more than one sub-column; all of them are still entry points.
    for (const name of ["docs", "status", "legacy"]) {
      expect(x(`Deployment/${name}`)).toBeLessThan(x("Deployment/checkout"));
      expect(out.nodes.find((n) => n.id === `Deployment/${name}`)!.rank).toBe(out.entryRank);
      // And its Ingress stands in the Ingress column, left of everything.
      expect(x(`Ingress/${name}`)).toBeLessThan(x(`Deployment/${name}`));
    }
    expect(new Set(["docs", "status", "legacy"].map((n) => x(`Deployment/${n}`))).size).toBe(3);
    expect(x("Deployment/checkout")).toBeLessThan(x("Service/payments"));
    expect(out.columns.map((c) => c.label)).toEqual(["INGRESS", "ENTRY", "HOP 1"]);
    // And the picture is now wider than it is tall.
    expect(out.width).toBeGreaterThan(out.height);
  });

  it("reads left to right: Ingress, then the Service with its pods beside it", () => {
    // Internet, Ingress, Service, pods, then the call to the next tier — the
    // order a request takes, and the order the eye should. The Ingress is a
    // column of its own; the Service and its pods are one tier behind it.
    const out = layoutFlow(chain());
    const at = (id: string) => out.nodes.find((n) => n.id === id)!;
    expect(at("Ingress/web").x).toBeLessThan(at("Service/checkout").x);
    expect(at("Ingress/web").y).toBe(at("Service/checkout").y);
    expect(at("Deployment/checkout").x).toBeGreaterThan(at("Service/checkout").x);
    expect(at("Deployment/checkout").y).toBe(at("Service/checkout").y);
    // Both links are short forward arrows; neither is a bow.
    const into = out.edges.find((e) => e.from === "Ingress/web")!;
    expect(into.path.startsWith(`M ${at("Ingress/web").x + NODE_WIDTH} `)).toBe(true);
    const across = out.edges.find((e) => e.from === "Service/checkout")!;
    expect(across.path.startsWith(`M ${at("Service/checkout").x + NODE_WIDTH} `)).toBe(true);
    // One panel, round the Service and its pods; the Ingress stands outside it.
    expect(out.tiers).toHaveLength(1);
    expect(out.tiers[0].x).toBeGreaterThan(at("Ingress/web").x);
    expect(out.tiers[0].width).toBeGreaterThan(NODE_WIDTH * 2);
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
    expect(out.columns.map((c) => c.label)).toEqual(["INGRESS", "ENTRY"]);
    expect(out.entryRank).toBe(1);
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
    // As wide as it can be without a column running past the flow.
    expect(bandRight).toBeGreaterThan(flowRight - (NODE_WIDTH + GRID_GAP));
    // And it sits below the flow, not beside it.
    const flowBottom = Math.max(...out.nodes.filter((n) => n.rank !== null).map((n) => n.y));
    expect(out.band?.y).toBeGreaterThan(flowBottom);
  });

  it("places a called tier beside its caller down a hop column", () => {
    // Without the barycentre pass the called tiers at HOP 2 would sit in name
    // order — b above a — and both calls would cross.
    const call = { kind: "calls" as const };
    const out = layoutFlow(
      graph(
        [
          node("Entry/e", "workload", "e"),
          node("Mid/a", "service", "a"),
          node("Mid/b", "service", "b"),
          node("Far/b", "service", "b"),
          node("Far/a", "service", "a"),
        ],
        [
          edge("Entry/e", "Mid/a", call),
          edge("Entry/e", "Mid/b", call),
          edge("Mid/a", "Far/a", call),
          edge("Mid/b", "Far/b", call),
        ],
      ),
    );
    const y = (id: string) => out.nodes.find((n) => n.id === id)!.y;
    expect(y("Far/a")).toBe(y("Mid/a"));
    expect(y("Far/b")).toBe(y("Mid/b"));
  });

  it("stands ENTRY's calling tiers side by side, and routes the inner one round", () => {
    // Two same-named tiers from two namespaces stacked five rows tall in
    // ENTRY. Side by side is what a reader asked for; a curve drawn straight
    // out of the inner one would run through its neighbour, so it leaves
    // downwards along a channel under the lane and turns up past the block.
    const call = { kind: "calls" as const };
    const out = layoutFlow(
      graph(
        [
          node("Caller/a", "workload", "a"),
          node("Caller/b", "workload", "b"),
          node("Called/c", "service", "c"),
        ],
        [edge("Caller/a", "Called/c", call), edge("Caller/b", "Called/c", call)],
      ),
    );
    const at = (id: string) => out.nodes.find((n) => n.id === id)!;
    expect(at("Caller/a").y).toBe(at("Caller/b").y);
    expect(at("Caller/a").x).not.toBe(at("Caller/b").x);
    const [inner, outer] = [at("Caller/a"), at("Caller/b")].sort((p, q) => p.x - q.x);
    expect(inner.detour).toBeDefined();
    expect(outer.detour).toBeUndefined();
    const via = out.edges.find((e) => e.from === inner.id)!;
    const straight = out.edges.find((e) => e.from === outer.id)!;
    expect(via.path.startsWith(`M ${inner.x + NODE_WIDTH / 2} ${inner.y + NODE_HEIGHT}`)).toBe(true);
    expect(via.path).toContain(" Q ");
    expect(straight.path).not.toContain(" Q ");
    // The channel lies under the lane and the box makes room for it.
    expect(inner.detour!.y).toBeGreaterThan(inner.y + NODE_HEIGHT);
    expect(out.height).toBeGreaterThan(inner.detour!.y);
  });

  it("gives each namespace its own lane, and marks the calls between them", () => {
    const call = { kind: "calls" as const, provenance: "declared" as const };
    const inPayments = (n: TopologyNode) => ({ ...n, namespace: "payments" });
    const out = layoutFlow(
      graph(
        [
          node("Service/checkout", "service", "checkout"),
          node("Deployment/checkout", "workload", "checkout"),
          inPayments(node("Service/payments", "service", "payments")),
          inPayments(node("Deployment/payments", "workload", "payments")),
        ],
        [
          edge("Service/checkout", "Deployment/checkout"),
          edge("Service/payments", "Deployment/payments"),
          edge("Deployment/checkout", "Service/payments", call),
        ],
      ),
    );
    expect(out.lanes.map((l) => l.namespace)).toEqual(["checkout", "payments"]);
    expect(out.lanes[1].y).toBeGreaterThan(out.lanes[0].y + out.lanes[0].height);
    const y = (id: string) => out.nodes.find((n) => n.id === id)!.y;
    expect(y("Service/payments")).toBeGreaterThanOrEqual(out.lanes[1].y);
    expect(y("Deployment/checkout")).toBeLessThan(out.lanes[1].y);
    // The call crosses; the routing inside a tier does not.
    expect(out.edges.find((e) => e.kind === "calls")!.crossesNamespace).toBe(true);
    expect(out.edges.filter((e) => e.kind === "routes").every((e) => !e.crossesNamespace)).toBe(true);
    // And a single namespace gets no lane at all — it is the heading.
    expect(layoutFlow(twoTiers()).lanes).toEqual([]);
  });

  it("puts a host outside the cluster in the lane of what calls it", () => {
    const call = { kind: "calls" as const, provenance: "declared" as const };
    const out = layoutFlow(
      graph(
        [
          node("Deployment/checkout", "workload", "checkout"),
          { ...node("Deployment/other", "workload", "other"), namespace: "payments" },
          { ...node("External//db", "external", "db"), namespace: "" },
        ],
        [
          edge("Deployment/checkout", "External//db", call),
          edge("Deployment/other", "External//db", call),
        ],
      ),
    );
    // Two namespaces call it; it goes with the first by name, and is drawn
    // once. Neither call is a cross-namespace one — the host is outside.
    const lane = out.lanes.find((l) => l.namespace === "checkout")!;
    const db = out.nodes.find((n) => n.id === "External//db")!;
    expect(db.y).toBeGreaterThanOrEqual(lane.y);
    expect(db.y).toBeLessThan(lane.y + lane.height);
    expect(out.lanes.some((l) => l.namespace === "")).toBe(false);
    expect(out.edges.every((e) => !e.crossesNamespace)).toBe(true);
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

describe("hubCounts", () => {
  it("names only what more than a handful of things call", () => {
    const into = (to: string, n: number) =>
      Array.from({ length: n }, (_, i) => edge(`c${i}`, to, { kind: "calls" }));
    const hubs = hubCounts([...into("busy", 9), ...into("quiet", 8), edge("a", "b")]);
    expect(hubs.get("busy")).toBe(9);
    expect(hubs.has("quiet")).toBe(false);
    // Routing is not calling: a Service with nine pods behind it is not a hub.
    expect(hubCounts(Array.from({ length: 9 }, (_, i) => edge("svc", `pod${i}`))).size).toBe(0);
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
