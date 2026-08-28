import { describe, it, expect, vi } from "vitest";
import {
  deleteResource,
  scaleResource,
  rolloutRestart,
  cronjobSetSuspend,
  cronjobTriggerNow,
  updateConfigData,
  debugPod,
  createNodeDebugPod,
} from "./actions";

describe("resource actions", () => {
  it("deleteResource passes kind/namespace/name", async () => {
    const invoke = vi.fn().mockResolvedValue({ ok: true });
    const out = await deleteResource("c", "ConfigMap", "default", "cm", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.deleteResource", {
      context: "c",
      kind: "ConfigMap",
      namespace: "default",
      name: "cm",
    });
    expect(out.ok).toBe(true);
  });

  it("scaleResource passes replicas", async () => {
    const invoke = vi.fn().mockResolvedValue({ ok: true });
    await scaleResource("c", "Deployment", "default", "web", 3, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.scale", {
      context: "c",
      kind: "Deployment",
      namespace: "default",
      name: "web",
      replicas: 3,
    });
  });

  it("rolloutRestart passes target", async () => {
    const invoke = vi.fn().mockResolvedValue({ ok: true });
    await rolloutRestart("c", "Deployment", "default", "web", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.rolloutRestart", {
      context: "c",
      kind: "Deployment",
      namespace: "default",
      name: "web",
    });
  });

  it("normalises errors", async () => {
    const out = await deleteResource("c", "Pod", "default", "p", () =>
      Promise.reject(new Error("forbidden")),
    );
    expect(out.error).toContain("forbidden");
  });

  it("cronjobSetSuspend passes the suspend flag", async () => {
    const invoke = vi.fn().mockResolvedValue({ ok: true });
    await cronjobSetSuspend("c", "ops", "nightly", true, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.cronjobSetSuspend", {
      context: "c",
      namespace: "ops",
      name: "nightly",
      suspend: true,
    });
  });

  it("updateConfigData passes kind/namespace/name and the edited data map", async () => {
    const invoke = vi.fn().mockResolvedValue({ ok: true });
    await updateConfigData("c", "ConfigMap", "default", "web-config", { "app.conf": "level=debug" }, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.updateConfigData", {
      context: "c",
      kind: "ConfigMap",
      namespace: "default",
      name: "web-config",
      data: { "app.conf": "level=debug" },
    });
  });

  it("cronjobTriggerNow sends a unique suffix and returns the job name", async () => {
    const invoke = vi.fn().mockResolvedValue({ jobName: "nightly-123", ok: true });
    const out = await cronjobTriggerNow("c", "ops", "nightly", invoke);
    const call = invoke.mock.calls[0];
    expect(call[0]).toBe("k8s.cronjobTriggerNow");
    expect(call[1]).toMatchObject({ context: "c", namespace: "ops", name: "nightly" });
    expect(typeof (call[1] as { suffix: string }).suffix).toBe("string");
    expect((call[1] as { suffix: string }).suffix.length).toBeGreaterThan(0);
    expect(out.jobName).toBe("nightly-123");
  });
});

describe("debug actions", () => {
  it("debugPod passes image + target and returns the container", async () => {
    const invoke = vi.fn().mockResolvedValue({ container: "debugger-1" });
    const out = await debugPod("c", "ns", "web", "busybox", "app", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.debugPod", {
      context: "c", namespace: "ns", pod: "web", image: "busybox", target_container: "app",
    });
    expect(out.container).toBe("debugger-1");
  });

  it("createNodeDebugPod defaults image/namespace to null and returns pod", async () => {
    const invoke = vi.fn().mockResolvedValue({ namespace: "default", pod: "srelens-node-debug-x" });
    const out = await createNodeDebugPod("c", "node-a", null, null, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.createNodeDebugPod", {
      context: "c", node: "node-a", image: null, namespace: null,
    });
    expect(out).toEqual({ namespace: "default", pod: "srelens-node-debug-x" });
  });

  it("maps a thrown error", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("forbidden"));
    expect((await debugPod("c", "ns", "web", "busybox", null, invoke)).error).toContain("forbidden");
  });
});
