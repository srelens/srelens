import { describe, it, expect, vi } from "vitest";
import {
  listStatefulSets,
  listDaemonSets,
  listJobs,
  listCronJobs,
  listConfigMaps,
  listSecrets,
  listResourceQuotas,
  listLimitRanges,
} from "./controllers";

describe("config resource lists", () => {
  it("listConfigMaps returns key counts", async () => {
    const invoke = vi.fn().mockResolvedValue({
      configmaps: [{ name: "web", namespace: "default", keys: 3, age: "2d" }],
    });
    const out = await listConfigMaps("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listConfigMaps", { context: "kind-dev", namespace: "default" });
    expect(out.configmaps?.[0].keys).toBe(3);
  });

  it("listSecrets returns type + key count and never any values", async () => {
    const invoke = vi.fn().mockResolvedValue({
      secrets: [{ name: "tls", namespace: "default", type: "kubernetes.io/tls", keys: 2, age: "1d" }],
    });
    const out = await listSecrets("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listSecrets", { context: "kind-dev", namespace: "default" });
    expect(out.secrets?.[0].type).toBe("kubernetes.io/tls");
    expect(out.secrets?.[0].keys).toBe(2);
    expect(Object.keys(out.secrets![0])).not.toContain("data");
  });

  it("listResourceQuotas returns constrained-resource counts", async () => {
    const invoke = vi.fn().mockResolvedValue({
      resourcequotas: [{ name: "q", namespace: "team", resources: 3, age: "5d" }],
    });
    const out = await listResourceQuotas("kind-dev", "team", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listResourceQuotas", { context: "kind-dev", namespace: "team" });
    expect(out.resourcequotas?.[0].resources).toBe(3);
  });

  it("listLimitRanges returns limit-entry counts", async () => {
    const invoke = vi.fn().mockResolvedValue({
      limitranges: [{ name: "lr", namespace: "default", limits: 2, age: "3d" }],
    });
    const out = await listLimitRanges("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listLimitRanges", { context: "kind-dev", namespace: "default" });
    expect(out.limitranges?.[0].limits).toBe(2);
  });
});

describe("listStatefulSets", () => {
  it("passes context+namespace and returns rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      statefulsets: [{ name: "pg", namespace: "data", ready: "2/3", updated: 3, service: "pg-headless", age: "5d" }],
    });
    const out = await listStatefulSets("kind-dev", "data", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listStatefulSets", { context: "kind-dev", namespace: "data" });
    expect(out.statefulsets?.[0].service).toBe("pg-headless");
  });

  it("normalises errors", async () => {
    const out = await listStatefulSets("x", "y", () => Promise.reject(new Error("forbidden")));
    expect(out.error).toContain("forbidden");
    expect(out.statefulsets).toBeUndefined();
  });
});

describe("listDaemonSets", () => {
  it("returns node-coverage rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      daemonsets: [{ name: "fluentd", namespace: "logging", desired: 5, current: 5, ready: 4, upToDate: 5, available: 4, age: "1d" }],
    });
    const out = await listDaemonSets("kind-dev", "logging", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listDaemonSets", { context: "kind-dev", namespace: "logging" });
    expect(out.daemonsets?.[0].ready).toBe(4);
  });
});

describe("listJobs", () => {
  it("returns completion+owner rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      jobs: [{ name: "backup-1", namespace: "ops", completions: "1/1", active: 0, failed: 0, duration: "2m", owner: "backup", age: "3h" }],
    });
    const out = await listJobs("kind-dev", "ops", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listJobs", { context: "kind-dev", namespace: "ops" });
    expect(out.jobs?.[0].owner).toBe("backup");
  });
});

describe("listCronJobs", () => {
  it("returns schedule+suspend rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      cronjobs: [{ name: "nightly", namespace: "ops", schedule: "0 2 * * *", suspended: true, active: 0, lastSchedule: "2h", age: "9d" }],
    });
    const out = await listCronJobs("kind-dev", "ops", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listCronJobs", { context: "kind-dev", namespace: "ops" });
    expect(out.cronjobs?.[0].suspended).toBe(true);
  });
});
