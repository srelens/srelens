import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { probeCluster, getInfo, useInfo, useInfos, getProbe, useProbe, useProbes, resetProbes } from "./probe";
import { getView, resetView } from "./workspace";

const ctx = {
  name: "prod-eu", stableId: "prod", cluster: "c", server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
};

beforeEach(() => { resetView(); resetProbes(); });

describe("probeCluster", () => {
  it("marks connecting, then connected with the version", async () => {
    let resolve!: (v: unknown) => void;
    const connect = vi.fn(() => new Promise<never>((r) => { resolve = r as never; }));
    const p = probeCluster(ctx, connect as never);
    expect(getView().links.prod).toEqual({ state: "connecting" });
    resolve({ context: "prod-eu", reachable: true, version: "v1.29.0" });
    await p;
    expect(getView().links.prod).toEqual({ state: "connected" });
    expect(getInfo("prod")?.version).toBe("v1.29.0");
  });
  it("marks error with the message when unreachable with one, disconnected without", async () => {
    await probeCluster(ctx, vi.fn().mockResolvedValue({ context: "prod-eu", reachable: false, error: "timeout" }) as never);
    expect(getView().links.prod).toEqual({ state: "error", error: "timeout" });
    await probeCluster(ctx, vi.fn().mockResolvedValue({ context: "prod-eu", reachable: false }) as never);
    expect(getView().links.prod).toEqual({ state: "disconnected" });
  });
  it("calls connectCluster with the context NAME, never the stableId", async () => {
    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true });
    await probeCluster(ctx, connect as never);
    expect(connect).toHaveBeenCalledWith("prod-eu");
  });
});

describe("useInfo", () => {
  it("re-renders with the info once the probe lands, and forgets it on reset", async () => {
    const { result } = renderHook(() => useInfo("prod"));
    expect(result.current).toBeUndefined();
    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true, version: "v1.30.1" });
    await act(async () => { await probeCluster(ctx, connect as never); });
    expect(result.current?.version).toBe("v1.30.1");
    act(() => resetProbes());
    expect(result.current).toBeUndefined();
  });
});

describe("useInfos", () => {
  it("hands back the whole store, with a new identity only when a probe lands", async () => {
    const { result, rerender } = renderHook(() => useInfos());
    expect(result.current).toEqual({});
    const before = result.current;
    // The snapshot has to survive a render that changed nothing, or
    // `useSyncExternalStore` re-renders forever.
    rerender();
    expect(result.current).toBe(before);

    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true, version: "v1.31.0" });
    await act(async () => { await probeCluster(ctx, connect as never); });
    expect(result.current.prod?.version).toBe("v1.31.0");
    expect(result.current).not.toBe(before);
  });
});

describe("getProbe", () => {
  it("is unread before anything has probed", () => {
    expect(getProbe("prod")).toEqual({ state: "unread" });
  });

  it("times the round trip and reports it once reachable", async () => {
    let t = 1000;
    const now = () => (t += 42);
    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true, version: "v1.29.4" });
    await probeCluster(ctx, connect as never, now);
    expect(getProbe("prod")).toEqual({ state: "reachable", latencyMs: 42, version: "v1.29.4" });
  });

  it("reads the clock exactly twice: once before the call, once after", async () => {
    const now = vi.fn().mockReturnValueOnce(1000).mockReturnValueOnce(1100);
    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true });
    await probeCluster(ctx, connect as never, now);
    expect(now).toHaveBeenCalledTimes(2);
    expect(getProbe("prod").latencyMs).toBe(100);
  });

  it("has no latency at all when the cluster does not answer, never zero", async () => {
    let t = 1000;
    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: false, error: "dial tcp: timeout" });
    await probeCluster(ctx, connect as never, () => (t += 42));
    const probe = getProbe("prod");
    expect(probe.state).toBe("unreachable");
    expect("latencyMs" in probe).toBe(false);
    expect(probe.error).toBeTruthy();
    expect(probe.error).not.toContain("dial tcp");
  });

  it("has no error at all when unreachable without one, matching the disconnected link state", async () => {
    const connect = vi.fn().mockResolvedValue({ context: "prod-eu", reachable: false });
    await probeCluster(ctx, connect as never);
    expect(getProbe("prod")).toEqual({ state: "unreachable" });
  });

  it("reports a rejected probe as unreachable rather than an unhandled rejection", async () => {
    const connect = vi.fn().mockRejectedValue(new Error("no route to host"));
    await probeCluster(ctx, connect as never);
    const probe = getProbe("prod");
    expect(probe.state).toBe("unreachable");
    expect("latencyMs" in probe).toBe(false);
    // The rejection still reaches the link state, same as a resolved failure.
    expect(getView().links.prod.state).toBe("error");
  });

  it("clears a stale latency reading once a later probe comes back unreachable", async () => {
    let t = 0;
    const now = () => (t += 10);
    await probeCluster(ctx, vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true }) as never, now);
    expect(getProbe("prod").latencyMs).toBe(10);
    await probeCluster(
      ctx,
      vi.fn().mockResolvedValue({ context: "prod-eu", reachable: false, error: "timeout" }) as never,
      now,
    );
    expect("latencyMs" in getProbe("prod")).toBe(false);
  });

  it("forgets everything on reset", async () => {
    await probeCluster(ctx, vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true }) as never, () => 5);
    resetProbes();
    expect(getProbe("prod")).toEqual({ state: "unread" });
  });
});

describe("useProbe", () => {
  it("re-renders with the probe once it lands, and forgets it on reset", async () => {
    const { result } = renderHook(() => useProbe("prod"));
    expect(result.current).toEqual({ state: "unread" });
    let t = 0;
    const now = () => (t += 7);
    await act(async () => {
      await probeCluster(ctx, vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true }) as never, now);
    });
    expect(result.current).toEqual({ state: "reachable", latencyMs: 7 });
    act(() => resetProbes());
    expect(result.current).toEqual({ state: "unread" });
  });
});

describe("useProbes", () => {
  it("hands back the whole store, with a new identity only when a probe lands", async () => {
    const { result, rerender } = renderHook(() => useProbes());
    expect(result.current).toEqual({});
    const before = result.current;
    rerender();
    expect(result.current).toBe(before);

    let t = 0;
    await act(async () => {
      await probeCluster(ctx, vi.fn().mockResolvedValue({ context: "prod-eu", reachable: true }) as never, () => (t += 5));
    });
    expect(result.current.prod).toEqual({ state: "reachable", latencyMs: 5 });
    expect(result.current).not.toBe(before);
  });
});
