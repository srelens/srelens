import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import {
  canI,
  clearAccessCache,
  rbac,
  useAccess,
  isForbidden,
  reportActionError,
  denyReason,
  type AccessResult,
  type AccessCheck,
} from "./access";
import { describeError } from "./errors";

const { notifyMock } = vi.hoisted(() => ({
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("./notify", () => ({ notify: notifyMock }));

describe("canI", () => {
  it("posts checks and returns results", async () => {
    const invoke = vi.fn().mockResolvedValue({ results: [{ allowed: true, denied: false, reason: "", error: false }] });
    const checks = [{ verb: "delete", resource: "pods", namespace: "prod" }];
    const out = await canI("ctx", checks, invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.canI", { context: "ctx", checks });
    expect(out.results?.[0].allowed).toBe(true);
  });
  it("returns error on failure", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("boom"));
    expect((await canI("ctx", [], invoke)).error).toContain("boom");
  });
});

describe("rbac builders", () => {
  it("maps actions to (verb, resource, subresource)", () => {
    expect(rbac.deletePod("prod")).toEqual({ verb: "delete", resource: "pods", namespace: "prod" });
    expect(rbac.evictPod("prod")).toEqual({ verb: "create", resource: "pods", subresource: "eviction", namespace: "prod" });
    expect(rbac.scale("apps", "deployments", "prod")).toEqual({ verb: "patch", group: "apps", resource: "deployments", subresource: "scale", namespace: "prod" });
    expect(rbac.cordon()).toEqual({ verb: "patch", resource: "nodes" });
    expect(rbac.cronjobTrigger("prod")).toEqual({ verb: "create", group: "batch", resource: "jobs", namespace: "prod" });
  });
});

describe("cache", () => {
  it("clearAccessCache(context) does not throw", () => {
    expect(() => clearAccessCache("ctx")).not.toThrow();
    expect(() => clearAccessCache()).not.toThrow();
  });
});

describe("isForbidden", () => {
  it("is true for a 403 / Forbidden string, false otherwise", () => {
    expect(isForbidden("pods is forbidden: User cannot delete")).toBe(true);
    expect(isForbidden("the server responded with 403")).toBe(true);
    expect(isForbidden("Forbidden")).toBe(true);
    expect(isForbidden("connection refused")).toBe(false);
    expect(isForbidden("4030 things happened")).toBe(false);
    expect(isForbidden("")).toBe(false);
  });
});

describe("reportActionError", () => {
  beforeEach(() => {
    clearAccessCache();
    notifyMock.error.mockReset();
  });

  it("toasts the friendly detail (not the raw string)", () => {
    const raw = "pods is forbidden: User cannot delete resource \"pods\" in the namespace \"prod\"";
    reportActionError("ctx", "Failed to delete web-1", raw);
    expect(notifyMock.error).toHaveBeenCalledWith("Failed to delete web-1", describeError(raw).detail);
  });

  it("clears the context's access cache on a 403 so the control re-gates", async () => {
    const check: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
    const invoke = vi.fn().mockResolvedValue({ results: [{ allowed: true, denied: false, reason: "", error: false }] });
    const { result } = renderHook(() => useAccess("ctx", [check], invoke));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.known(check)).toBe(true);

    act(() => reportActionError("ctx", "Failed", "pods is forbidden: cannot delete"));

    // Cache invalidated → the check is unknown again.
    expect(result.current.known(check)).toBe(false);
  });

  it("keeps the cache when the error is not a 403", async () => {
    const check: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
    const invoke = vi.fn().mockResolvedValue({ results: [{ allowed: true, denied: false, reason: "", error: false }] });
    const { result } = renderHook(() => useAccess("ctx", [check], invoke));
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() => reportActionError("ctx", "Failed", "connection refused"));

    expect(result.current.known(check)).toBe(true);
  });
});

describe("denyReason", () => {
  const access = (allowed: boolean, known: boolean) => ({
    allowed: () => allowed,
    known: () => known,
  });

  it("returns the sentence only when the check resolved denied", () => {
    const check: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
    expect(denyReason(access(false, true), check)).toBe(
      "You don't have permission to delete pods in prod",
    );
  });

  it("omits the namespace clause for a cluster-scoped check", () => {
    const check: AccessCheck = { verb: "patch", resource: "nodes" };
    expect(denyReason(access(false, true), check)).toBe("You don't have permission to patch nodes");
  });

  it("includes the subresource so Evict/Scale name the real permission", () => {
    const evict: AccessCheck = { verb: "create", resource: "pods", subresource: "eviction", namespace: "prod" };
    expect(denyReason(access(false, true), evict)).toBe(
      "You don't have permission to create pods/eviction in prod",
    );
    const scale: AccessCheck = { verb: "patch", group: "apps", resource: "deployments", subresource: "scale", namespace: "prod" };
    expect(denyReason(access(false, true), scale)).toBe(
      "You don't have permission to patch deployments/scale in prod",
    );
  });

  it("returns undefined when allowed or still unknown", () => {
    const check: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
    expect(denyReason(access(true, true), check)).toBeUndefined();
    expect(denyReason(access(false, false), check)).toBeUndefined();
  });
});

describe("useAccess", () => {
  beforeEach(() => {
    clearAccessCache();
  });

  it("batches only uncached checks into one canI call, and allowed() reflects the result once resolved", async () => {
    const checkA: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
    const checkB: AccessCheck = { verb: "get", resource: "pods", namespace: "prod" };
    const resultA: AccessResult = { allowed: true, denied: false, reason: "", error: false };
    const resultB: AccessResult = { allowed: false, denied: true, reason: "forbidden", error: false };

    const invoke = vi
      .fn()
      .mockResolvedValueOnce({ results: [resultA] })
      .mockResolvedValueOnce({ results: [resultB] });

    const { result, rerender } = renderHook(({ checks }: { checks: AccessCheck[] }) => useAccess("ctx", checks, invoke), {
      initialProps: { checks: [checkA] },
    });

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenNthCalledWith(1, "k8s.canI", { context: "ctx", checks: [checkA] });
    expect(result.current.allowed(checkA)).toBe(true);

    // Rerender with an additional, previously-uncached check. The already
    // cached checkA must NOT be resent — only the new checkB is batched.
    rerender({ checks: [checkA, checkB] });

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenNthCalledWith(2, "k8s.canI", { context: "ctx", checks: [checkB] });
    expect(result.current.allowed(checkA)).toBe(true);
    expect(result.current.allowed(checkB)).toBe(false);
  });

  it("treats unresolved checks as unknown/not-allowed while loading, then flips once resolved", async () => {
    const check: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
    let resolve!: (v: { results: AccessResult[] }) => void;
    const pending = new Promise<{ results: AccessResult[] }>((r) => {
      resolve = r;
    });
    const invoke = vi.fn().mockReturnValue(pending);

    const { result } = renderHook(() => useAccess("ctx", [check], invoke));

    // Before the promise resolves: unknown, not allowed, loading.
    expect(result.current.loading).toBe(true);
    expect(result.current.known(check)).toBe(false);
    expect(result.current.allowed(check)).toBe(false);

    resolve({ results: [{ allowed: true, denied: false, reason: "", error: false }] });

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.known(check)).toBe(true);
    expect(result.current.allowed(check)).toBe(true);
  });

  it("does NOT cache a failed check (error:true) and retries until it resolves", async () => {
    vi.useFakeTimers();
    try {
      const check: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
      const invoke = vi
        .fn()
        // First attempt FAILS (timeout/call error, not a real RBAC denial).
        .mockResolvedValueOnce({ results: [{ allowed: false, denied: false, reason: "", error: true }] })
        // Retry succeeds with a definitive answer.
        .mockResolvedValue({ results: [{ allowed: true, denied: false, reason: "", error: false }] });

      const { result } = renderHook(() => useAccess("ctx", [check], invoke));

      // Flush the first (errored) response.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(invoke).toHaveBeenCalledTimes(1);
      // A failed check is NOT cached: known() stays false so denyReason renders
      // no false "no permission" tooltip and the control re-checks.
      expect(result.current.known(check)).toBe(false);

      // The bounded retry timer (1500ms * attempt) fires and re-checks.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1500);
      });
      expect(invoke).toHaveBeenCalledTimes(2);
      expect(result.current.known(check)).toBe(true);
      expect(result.current.allowed(check)).toBe(true);
    } finally {
      vi.useRealTimers();
    }
  });

  it("caches a definitive denial (error:false, allowed:false) as known — no retry", async () => {
    const check: AccessCheck = { verb: "delete", resource: "pods", namespace: "prod" };
    const invoke = vi
      .fn()
      .mockResolvedValue({ results: [{ allowed: false, denied: true, reason: "forbidden", error: false }] });

    const { result } = renderHook(() => useAccess("ctx", [check], invoke));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.known(check)).toBe(true);
    expect(result.current.allowed(check)).toBe(false);
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("does not collide checks that differ only by `name` (resourceNames-scoped RBAC)", async () => {
    const checkPodA: AccessCheck = { verb: "get", resource: "pods", namespace: "ns", name: "pod-a" };
    const checkPodB: AccessCheck = { verb: "get", resource: "pods", namespace: "ns", name: "pod-b" };
    const invoke = vi.fn().mockResolvedValue({
      results: [
        { allowed: true, denied: false, reason: "", error: false },
        { allowed: false, denied: true, reason: "forbidden", error: false },
      ],
    });

    const { result } = renderHook(() => useAccess("ctx", [checkPodA, checkPodB], invoke));

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.allowed(checkPodA)).toBe(true);
    expect(result.current.allowed(checkPodB)).toBe(false);
  });
});
