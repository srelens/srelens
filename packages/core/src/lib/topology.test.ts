import { describe, expect, it, vi } from "vitest";
import { prometheusDiscover, topologyGraph } from "./topology";

describe("topologyGraph", () => {
  it("passes the namespaces and returns the graph", async () => {
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
          provenance: "topology",
          detail: "",
          health: "degraded",
        },
      ],
    });

    const out = await topologyGraph("kind-dev", ["checkout", "payments"], undefined, invoke);

    expect(invoke).toHaveBeenCalledWith("k8s.topologyGraph", {
      context: "kind-dev",
      namespaces: ["checkout", "payments"],
      prometheus: undefined,
    });
    expect(out.graph?.nodes[0].health).toBe("degraded");
    expect(out.graph?.edges[0].kind).toBe("routes");
  });

  it("carries a metrics source through when there is one", async () => {
    // Without it the graph is still built, from the API alone — telemetry only
    // ever adds observed edges and rates on top.
    const invoke = vi.fn().mockResolvedValue({ nodes: [], edges: [] });
    const prometheus = { namespace: "monitoring", service: "prometheus", port: 9090 };
    await topologyGraph("c", ["checkout"], prometheus, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.topologyGraph", {
      context: "c",
      namespaces: ["checkout"],
      prometheus,
    });
  });

  it("normalises a rejection into an error rather than throwing", async () => {
    // Every other reader in core answers this shape, and the screen renders
    // `error` as its own state — a throw here would take the tab down instead.
    const out = await topologyGraph("c", ["checkout"], undefined, () =>
      Promise.reject(new Error("no such namespace")),
    );
    expect(out.graph).toBeUndefined();
    expect(out.error).toContain("no such namespace");
  });

  it("reads an empty namespace as an empty graph, not as nothing to draw", async () => {
    // A namespace with no workloads answers with empty lists. That is a real
    // answer — the screen says the namespace is empty rather than staying on
    // its loading state forever.
    const invoke = vi.fn().mockResolvedValue({ nodes: [], edges: [] });
    const out = await topologyGraph("c", ["empty"], undefined, invoke);
    expect(out.error).toBeUndefined();
    expect(out.graph).toEqual({ nodes: [], edges: [] });
  });
});

describe("prometheusDiscover", () => {
  it("returns the candidates the cluster already runs", async () => {
    const invoke = vi.fn().mockResolvedValue({
      candidates: [
        { namespace: "monitoring", service: "prometheus", port: 9090, flavour: "prometheus" },
      ],
    });
    const out = await prometheusDiscover("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.prometheusDiscover", { context: "kind-dev" });
    expect(out.candidates?.[0].port).toBe(9090);
  });

  it("treats no metrics backend as an ordinary answer", async () => {
    // Most clusters run none, and every caller works without one — this must
    // not read as a failure.
    const invoke = vi.fn().mockResolvedValue({ candidates: [] });
    const out = await prometheusDiscover("c", invoke);
    expect(out.error).toBeUndefined();
    expect(out.candidates).toEqual([]);
  });

  it("normalises a refusal rather than throwing", async () => {
    // Listing Services cluster-wide is a permission a reader may not have, and
    // losing it must cost the rates, not the screen.
    const out = await prometheusDiscover("c", () => Promise.reject(new Error("forbidden")));
    expect(out.candidates).toBeUndefined();
    expect(out.error).toContain("forbidden");
  });
});
