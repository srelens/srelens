import { describe, it, expect, vi } from "vitest";
import {
  listNamespaces,
  listPods,
  podLogs,
  listDeployments,
  listServices,
  listReplicaSets,
  podsForSelector,
  podMetrics,
  deletePod,
  evictPod,
} from "./workloads";

describe("listNamespaces", () => {
  it("passes context and returns namespace names", async () => {
    const invoke = vi.fn().mockResolvedValue({ namespaces: ["default", "kube-system"] });
    const outcome = await listNamespaces("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listNamespaces", { context: "kind-dev" });
    expect(outcome.namespaces).toEqual(["default", "kube-system"]);
  });

  it("normalises errors", async () => {
    const outcome = await listNamespaces("x", () => Promise.reject(new Error("forbidden")));
    expect(outcome.namespaces).toBeUndefined();
    expect(outcome.error).toContain("forbidden");
  });
});

describe("listPods", () => {
  it("passes context+namespace and returns pods", async () => {
    const pod = {
      name: "web-1",
      namespace: "default",
      phase: "Running",
      ready: "1/1",
      restarts: 0,
      node: "node-a",
    };
    const invoke = vi.fn().mockResolvedValue({ pods: [pod] });
    const outcome = await listPods("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listPods", {
      context: "kind-dev",
      namespace: "default",
    });
    expect(outcome.pods).toHaveLength(1);
    expect(outcome.pods?.[0].ready).toBe("1/1");
  });

  it("normalises errors", async () => {
    const outcome = await listPods("x", "default", () => Promise.reject(new Error("timed out")));
    expect(outcome.pods).toBeUndefined();
    expect(outcome.error).toContain("timed out");
  });
});

describe("podLogs", () => {
  it("passes context+namespace+pod and returns logs", async () => {
    const invoke = vi.fn().mockResolvedValue({ logs: "line1\nline2" });
    const outcome = await podLogs("kind-dev", "default", "web-1", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.podLogs", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
    });
    expect(outcome.logs).toContain("line2");
  });

  it("threads container and log options through as snake_case keys", async () => {
    const invoke = vi.fn().mockResolvedValue({ logs: "" });
    await podLogs("kind-dev", "default", "web-1", invoke, {
      container: "app",
      previous: true,
      timestamps: true,
      sinceSeconds: 300,
      tailLines: 500,
    });
    expect(invoke).toHaveBeenCalledWith("k8s.podLogs", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: "app",
      previous: true,
      timestamps: true,
      since_seconds: 300,
      tail_lines: 500,
    });
  });

  it("omits options that are falsy or unset", async () => {
    const invoke = vi.fn().mockResolvedValue({ logs: "" });
    await podLogs("kind-dev", "default", "web-1", invoke, { container: "app", previous: false });
    expect(invoke).toHaveBeenCalledWith("k8s.podLogs", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: "app",
    });
  });

  it("normalises errors", async () => {
    const outcome = await podLogs("x", "default", "p", () =>
      Promise.reject(new Error("container not found")),
    );
    expect(outcome.logs).toBeUndefined();
    expect(outcome.error).toContain("container not found");
  });
});

describe("listDeployments", () => {
  it("passes context+namespace and returns deployments", async () => {
    const invoke = vi.fn().mockResolvedValue({
      deployments: [{ name: "web", namespace: "default", ready: "1/1", upToDate: 1, available: 1 }],
    });
    const outcome = await listDeployments("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listDeployments", {
      context: "kind-dev",
      namespace: "default",
    });
    expect(outcome.deployments?.[0].ready).toBe("1/1");
  });

  it("normalises errors", async () => {
    const outcome = await listDeployments("x", "default", () => Promise.reject(new Error("nope")));
    expect(outcome.error).toContain("nope");
  });
});

describe("listServices", () => {
  it("passes context+namespace and returns services", async () => {
    const invoke = vi.fn().mockResolvedValue({
      services: [
        { name: "api", namespace: "default", type: "ClusterIP", clusterIP: "10.0.0.1", ports: "80/TCP" },
      ],
    });
    const outcome = await listServices("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listServices", {
      context: "kind-dev",
      namespace: "default",
    });
    expect(outcome.services?.[0].ports).toBe("80/TCP");
  });

  it("normalises errors", async () => {
    const outcome = await listServices("x", "default", () => Promise.reject(new Error("boom")));
    expect(outcome.error).toContain("boom");
  });
});

describe("listReplicaSets", () => {
  it("passes context+namespace+owner and returns replicasets", async () => {
    const invoke = vi.fn().mockResolvedValue({ replicasets: [{ name: "web-abc123" }] });
    const outcome = await listReplicaSets("kind-dev", "default", "web", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listReplicaSets", {
      context: "kind-dev",
      namespace: "default",
      ownerName: "web",
    });
    expect(outcome.replicasets).toEqual([{ name: "web-abc123" }]);
  });

  it("normalises errors", async () => {
    const outcome = await listReplicaSets("x", "default", "web", () =>
      Promise.reject(new Error("forbidden")),
    );
    expect(outcome.replicasets).toBeUndefined();
    expect(outcome.error).toContain("forbidden");
  });
});

describe("podsForSelector", () => {
  it("passes context+namespace+selector and returns pods", async () => {
    const invoke = vi.fn().mockResolvedValue({ pods: [{ name: "web-1" }] });
    const outcome = await podsForSelector("kind-dev", "default", { app: "web" }, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.podsForSelector", {
      context: "kind-dev",
      namespace: "default",
      selector: { app: "web" },
    });
    expect(outcome.pods).toEqual([{ name: "web-1" }]);
  });

  it("normalises errors", async () => {
    const outcome = await podsForSelector("x", "default", {}, () =>
      Promise.reject(new Error("forbidden")),
    );
    expect(outcome.pods).toBeUndefined();
    expect(outcome.error).toContain("forbidden");
  });
});

describe("podMetrics", () => {
  it("passes context+namespace and returns metrics", async () => {
    const invoke = vi.fn().mockResolvedValue({ metrics: [{ name: "web-1", cpu: "5m" }] });
    const outcome = await podMetrics("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.podMetrics", {
      context: "kind-dev",
      namespace: "default",
    });
    expect(outcome.metrics).toEqual([{ name: "web-1", cpu: "5m" }]);
  });

  it("normalises errors", async () => {
    const outcome = await podMetrics("x", "default", () =>
      Promise.reject(new Error("metrics-server unavailable")),
    );
    expect(outcome.metrics).toBeUndefined();
    expect(outcome.error).toContain("metrics-server unavailable");
  });
});

describe("evictPod", () => {
  it("passes context+namespace+pod and returns ok", async () => {
    const invoke = vi.fn().mockResolvedValue({ ok: true });
    const outcome = await evictPod("kind-dev", "default", "web-1", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.evictPod", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
    });
    expect(outcome.ok).toBe(true);
  });

  it("normalises errors", async () => {
    const outcome = await evictPod("x", "default", "p", () =>
      Promise.reject(new Error("disruption budget")),
    );
    expect(outcome.ok).toBeUndefined();
    expect(outcome.error).toContain("disruption budget");
  });
});

describe("deletePod", () => {
  it("passes context+namespace+pod and returns deleted", async () => {
    const invoke = vi.fn().mockResolvedValue({ deleted: true });
    const outcome = await deletePod("kind-dev", "default", "web-1", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.deletePod", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
    });
    expect(outcome.deleted).toBe(true);
  });

  it("normalises errors", async () => {
    const outcome = await deletePod("x", "default", "p", () =>
      Promise.reject(new Error("forbidden")),
    );
    expect(outcome.error).toContain("forbidden");
  });
});
