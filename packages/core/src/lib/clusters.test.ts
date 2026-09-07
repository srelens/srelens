import { describe, it, expect, vi } from "vitest";
import { listContexts, connectCluster, clusterFacts } from "./clusters";

describe("listContexts", () => {
  it("returns the contexts on success", async () => {
    const invoke = vi.fn().mockResolvedValue({
      contexts: [{ name: "kind-dev", cluster: "kind-dev", server: "https://x" }],
    });
    const outcome = await listContexts(["/tmp/extra"], invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listContexts", { paths: ["/tmp/extra"] });
    expect(outcome.error).toBeUndefined();
    expect(outcome.contexts).toHaveLength(1);
    expect(outcome.contexts?.[0].name).toBe("kind-dev");
  });

  it("passes through the context's declared namespace", async () => {
    const invoke = vi.fn().mockResolvedValue({
      contexts: [{ name: "team-a-ctx", cluster: "c", server: "https://x", namespace: "team-a" }],
    });
    const outcome = await listContexts(["/tmp/extra"], invoke);
    expect(outcome.contexts?.[0].namespace).toBe("team-a");
  });

  it("carries the source file and auth kind the backend reported", async () => {
    const invoke = vi.fn().mockResolvedValue({
      contexts: [{
        name: "prod-eu", stableId: "prod-eu", cluster: "prod", server: "https://prod:6443",
        namespace: "", isCurrent: true, isLocal: false,
        sourceFile: "/home/dana/.kube/config", authKind: "exec plugin · gcloud",
      }],
    });
    const out = await listContexts([], invoke);
    expect(out.contexts?.[0].sourceFile).toBe("/home/dana/.kube/config");
    expect(out.contexts?.[0].authKind).toBe("exec plugin · gcloud");
  });

  it("returns a normalised error on failure", async () => {
    const outcome = await listContexts([], () =>
      Promise.reject(new Error("read kubeconfig: not found")),
    );
    expect(outcome.contexts).toBeUndefined();
    expect(outcome.error).toContain("read kubeconfig: not found");
  });
});

describe("connectCluster", () => {
  it("passes the context through and returns the cluster info", async () => {
    const invoke = vi.fn().mockResolvedValue({
      context: "kind-dev",
      reachable: true,
      version: "v1.30.0",
    });
    const info = await connectCluster("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.clusterInfo", { context: "kind-dev" });
    expect(info.reachable).toBe(true);
    expect(info.version).toBe("v1.30.0");
  });

  it("normalises a transport failure into an unreachable result", async () => {
    const info = await connectCluster("prod", () =>
      Promise.reject(new Error("ipc unavailable")),
    );
    expect(info.reachable).toBe(false);
    expect(info.error).toContain("ipc unavailable");
  });
});

describe("clusterFacts", () => {
  it("passes the context through and returns the facts", async () => {
    const invoke = vi.fn().mockResolvedValue({
      context: "gke_acme_prod",
      provider: "GKE",
      region: "europe-west4",
      metricsServer: { state: "present", version: "v1beta1" },
    });
    const facts = await clusterFacts("gke_acme_prod", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.clusterFacts", { context: "gke_acme_prod" });
    expect(facts.provider).toBe("GKE");
    expect(facts.region).toBe("europe-west4");
    expect(facts.metricsServer.state).toBe("present");
    expect(facts.metricsServer.version).toBe("v1beta1");
    expect(facts.error).toBeUndefined();
  });

  it("carries a fact with nothing behind it as an empty value, never a placeholder", async () => {
    // The rail omits a row whose value is empty; "unknown" would look like an
    // answer and would render a row that says nothing.
    const invoke = vi.fn().mockResolvedValue({
      context: "kind-srelens-demo",
      provider: "kind",
      region: "",
      metricsServer: { state: "present", version: "v1beta1" },
    });
    const facts = await clusterFacts("kind-srelens-demo", invoke);
    expect(facts.region).toBe("");
  });

  it("keeps an absent metrics server distinct from one we could not ask about", async () => {
    const absent = await clusterFacts(
      "no-metrics",
      vi.fn().mockResolvedValue({
        context: "no-metrics",
        provider: "kind",
        region: "",
        metricsServer: { state: "absent", version: "" },
      }),
    );
    expect(absent.metricsServer.state).toBe("absent");
    expect(absent.error).toBeUndefined();

    const unreachable = await clusterFacts("prod", () =>
      Promise.reject(new Error("connection timed out")),
    );
    expect(unreachable.metricsServer.state).toBe("unknown");
    expect(unreachable.error).toContain("connection timed out");
  });

  it("normalises an unreachable cluster into empty facts plus a reason", async () => {
    const facts = await clusterFacts("prod", () => Promise.reject(new Error("ipc unavailable")));
    expect(facts.context).toBe("prod");
    expect(facts.provider).toBe("");
    expect(facts.region).toBe("");
    expect(facts.error).toContain("ipc unavailable");
  });
});
