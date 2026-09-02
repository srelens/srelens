import { describe, expect, it, vi } from "vitest";
import { topologyGraph } from "./topology";

describe("topologyGraph", () => {
  it("passes context and namespace, and returns the graph", async () => {
    const invoke = vi.fn().mockResolvedValue({
      nodes: [
        {
          id: "Deployment/checkout/checkout-api",
          kind: "Deployment",
          name: "checkout-api",
          namespace: "checkout",
          lane: "workload",
          detail: "9/12",
          ready: 9,
          desired: 12,
          health: "degraded",
        },
      ],
      edges: [
        {
          from: "Service/checkout/checkout-api",
          to: "Deployment/checkout/checkout-api",
          kind: "routes",
          health: "degraded",
        },
      ],
    });

    const out = await topologyGraph("kind-dev", ["checkout", "payments"], invoke);

    expect(invoke).toHaveBeenCalledWith("k8s.topologyGraph", {
      context: "kind-dev",
      namespaces: ["checkout", "payments"],
    });
    expect(out.graph?.nodes[0].health).toBe("degraded");
    expect(out.graph?.edges[0].kind).toBe("routes");
  });

  it("normalises a rejection into an error rather than throwing", async () => {
    // Every other reader in core answers this shape, and the screen renders
    // `error` as its own state — a throw here would take the tab down instead.
    const out = await topologyGraph("c", ["checkout"], () => Promise.reject(new Error("no such namespace")));
    expect(out.graph).toBeUndefined();
    expect(out.error).toContain("no such namespace");
  });

  it("reads an empty namespace as an empty graph, not as nothing to draw", async () => {
    // A namespace with no workloads answers with empty lists. That is a real
    // answer — the screen says the namespace is empty rather than staying on
    // its loading state forever.
    const invoke = vi.fn().mockResolvedValue({ nodes: [], edges: [] });
    const out = await topologyGraph("c", ["empty"], invoke);
    expect(out.error).toBeUndefined();
    expect(out.graph).toEqual({ nodes: [], edges: [] });
  });
});
