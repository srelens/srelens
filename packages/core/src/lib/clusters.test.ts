import { describe, it, expect, vi } from "vitest";
import { listContexts, connectCluster } from "./clusters";

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
