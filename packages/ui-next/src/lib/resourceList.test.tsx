import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, renderHook, act, waitFor } from "@testing-library/react";

// `vi.hoisted` because `vi.mock` is hoisted above every declaration in the
// file — a plain `const watchResource = vi.fn(...)` below it would be read
// before it's initialized (see AppLog.test.tsx / Window.test.tsx for the
// same pattern elsewhere in this package).
const { stop, watchResource, mockState } = vi.hoisted(() => {
  const mockState: {
    emitRows: ((rows: unknown[]) => void) | null;
    emitStatus: ((s: string) => void) | null;
  } = { emitRows: null, emitStatus: null };
  const stop = vi.fn();
  const watchResource = vi.fn(
    async (
      _context: string,
      _namespace: string,
      _kind: string,
      onRows: (rows: unknown[]) => void,
      onStatus: (s: string) => void,
    ) => {
      mockState.emitRows = onRows;
      mockState.emitStatus = onStatus;
      return { stop };
    },
  );
  return { stop, watchResource, mockState };
});
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  watchResource,
}));

import { useResourceList, resetListCache } from "./resourceList";
import { rowKey, type KindDescriptor, type ListRow } from "./kinds/types";

// Typed via an explicit annotation, not `as const`: an `as const` object
// literal narrows its array properties to `readonly`, which then can't
// satisfy `KindDescriptor`'s mutable `columns: Column<Row>[]`. Annotating
// the variable instead gives every field its literal type (e.g. `"watch"`,
// not `string`) without that mismatch.
const watched: KindDescriptor<ListRow> = { k8sKind: "Pod", columns: [], source: "watch", scope: "namespaced", actions: {} };

describe("useResourceList", () => {
  beforeEach(() => {
    resetListCache();
    vi.clearAllMocks();
    mockState.emitRows = null;
    mockState.emitStatus = null;
  });

  it("starts on loading and settles on the first snapshot", async () => {
    const { result } = renderHook(() => useResourceList("prod", "pods", watched, "default", []));
    expect(result.current.status).toBe("loading");
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([{ name: "a" }]));
    expect(result.current.status).toBe("ready");
    expect(result.current.rows).toHaveLength(1);
  });

  it("settles on error, not stuck loading, when the watch fails to start", async () => {
    watchResource.mockRejectedValueOnce(new Error("rbac denied"));
    const { result } = renderHook(() => useResourceList("prod", "pods", watched, "default", []));
    expect(result.current.status).toBe("loading");
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error).toBe("rbac denied");
  });

  it("says empty, not ready, when the kind has none — the states differ to a reader", async () => {
    const { result } = renderHook(() => useResourceList("prod", "pods", watched, "default", []));
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([]));
    expect(result.current.status).toBe("empty");
  });

  it("reports a reconnecting watch without emptying the table", async () => {
    const { result } = renderHook(() => useResourceList("prod", "pods", watched, "default", []));
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([{ name: "a" }]));
    act(() => mockState.emitStatus!("reconnecting"));
    expect(result.current.watch).toBe("reconnecting");
    expect(result.current.rows).toHaveLength(1);
  });

  it("stops the old watch before the new view starts one", async () => {
    const { rerender } = renderHook((p: { ns: string }) => useResourceList("prod", "pods", watched, p.ns, []), {
      initialProps: { ns: "default" },
    });
    await waitFor(() => expect(watchResource).toHaveBeenCalledTimes(1));
    rerender({ ns: "kube-system" });
    await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
    expect(watchResource).toHaveBeenCalledTimes(2);
  });

  it("drops a snapshot that arrives after the view changed", async () => {
    const { result, rerender } = renderHook((p: { ns: string }) => useResourceList("prod", "pods", watched, p.ns, []), {
      initialProps: { ns: "default" },
    });
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    const stale = mockState.emitRows!;
    rerender({ ns: "kube-system" });
    act(() => stale([{ name: "from-the-old-namespace" }]));
    expect(result.current.rows).toHaveLength(0);
  });

  it("paints the cached rows on a remount instead of flashing empty", async () => {
    const first = renderHook(() => useResourceList("prod", "pods", watched, "default", []));
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([{ name: "a" }]));
    first.unmount();
    const second = renderHook(() => useResourceList("prod", "pods", watched, "default", []));
    expect(second.result.current.rows).toHaveLength(1);
  });

  it("keeps the cached rows when a poll fails, and says why above them", async () => {
    const load = vi.fn()
      .mockResolvedValueOnce({ rows: [{ name: "a" }] })
      .mockResolvedValueOnce({ error: "connection refused" });
    const polled: KindDescriptor<ListRow> = { ...watched, source: "poll", load };
    const { result } = renderHook(() => useResourceList("prod", "leases", polled, "default", []));
    await waitFor(() => expect(result.current.rows).toHaveLength(1));
    act(() => result.current.reload());
    await waitFor(() => expect(result.current.error).toBe("connection refused"));
    expect(result.current.rows).toHaveLength(1);
  });

  it("evicts the oldest view key at the 40-entry cap, but spares one just refreshed", async () => {
    // Seed a view's cache entry by mounting it, waiting for its first
    // snapshot, and emitting rows for it — this both inserts (a new key)
    // and refreshes (an existing key, moved to most-recently-written) the
    // same way, since cacheSet always re-inserts on write.
    const seed = async (ctx: string, name: string) => {
      const { unmount } = renderHook(() => useResourceList(ctx, "pods", watched, "default", []));
      await waitFor(() => expect(mockState.emitRows).not.toBeNull());
      act(() => mockState.emitRows!([{ name }]));
      unmount();
    };

    for (let i = 0; i < 40; i++) {
      await seed(`ctx${i}`, `row${i}`);
    }
    // Refresh ctx0 last, so it is no longer the oldest entry.
    await seed("ctx0", "row0-refreshed");
    // A 41st distinct key pushes the cache over its cap: the true oldest
    // entry (ctx1, not the just-refreshed ctx0) must be the one evicted.
    await seed("ctx40", "row40");

    const refreshed = renderHook(() => useResourceList("ctx0", "pods", watched, "default", []));
    expect(refreshed.result.current.rows).toHaveLength(1);

    const evicted = renderHook(() => useResourceList("ctx1", "pods", watched, "default", []));
    expect(evicted.result.current.status).toBe("loading");
    expect(evicted.result.current.rows).toHaveLength(0);
  }, 20000);

  it("merges metrics into the rows by their identity, without waiting for them", async () => {
    const enrich = vi.fn().mockResolvedValue(new Map([[rowKey({ name: "a", namespace: "shop" }), { cpu: 12 }]]));
    const d = { ...watched, enrich, enrichMs: 10000 } as const;
    const { result } = renderHook(() => useResourceList("prod", "pods", d, "shop", []));
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([{ name: "a", namespace: "shop" }, { name: "b", namespace: "shop" }]));
    expect(result.current.rows[0]).toMatchObject({ name: "a" }); // rows are on screen at once
    await waitFor(() => expect(result.current.rows[0]).toMatchObject({ name: "a", cpu: 12 }));
    expect(result.current.rows[1]).not.toHaveProperty("cpu");
  });

  /**
   * All-namespaces mode: `podMetrics` answers for every namespace, so two
   * namespaces each running `api-0` are two readings. Keyed by name alone,
   * one pod's CPU landed on the other's row — displayed, and sorted on.
   */
  it("gives each of two namespaces' api-0 its own reading, never the other's", async () => {
    const enrich = vi.fn().mockResolvedValue(
      new Map([
        [rowKey({ name: "api-0", namespace: "shop" }), { cpu: 10 }],
        [rowKey({ name: "api-0", namespace: "billing" }), { cpu: 20 }],
      ]),
    );
    const d = { ...watched, enrich, enrichMs: 10000 } as const;
    const { result } = renderHook(() => useResourceList("prod", "pods", d, "", []));
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() =>
      mockState.emitRows!([
        { name: "api-0", namespace: "shop" },
        { name: "api-0", namespace: "billing" },
      ]),
    );
    await waitFor(() => expect(result.current.rows[0]).toMatchObject({ namespace: "shop", cpu: 10 }));
    expect(result.current.rows[1]).toMatchObject({ namespace: "billing", cpu: 20 });
  });

  it("lists the pods anyway when there is no metrics-server", async () => {
    const enrich = vi.fn().mockRejectedValue(new Error("metrics API not available"));
    const d = { ...watched, enrich, enrichMs: 10000 } as const;
    const { result } = renderHook(() => useResourceList("prod", "pods", d, "default", []));
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([{ name: "a" }]));
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.error).toBeUndefined();
  });
});

/**
 * The render-time key gate — `useObject`'s, for the same reason and in the
 * same shape.
 *
 * Every test above settles first and then asserts, which is blind to this:
 * RTL flushes passive effects synchronously, so by the time a `waitFor`
 * resolves the effect has already reset the state and any bad frame has been
 * overwritten. A real browser paints whatever was committed, and what gets
 * committed is decided during render, before any effect runs. So these
 * observe the hook's return value AT RENDER TIME instead.
 *
 * It is reachable on a MOUNTED hook, not only at mount: `Resources` renders
 * its inner component with no `key`, and no screen carries `key={name}` (see
 * `lib/clusterMoved.tsx`), so switching cluster or namespace changes the view
 * on a hook that stays put.
 */
describe("useResourceList — the render the view changes", () => {
  beforeEach(() => {
    resetListCache();
    vi.clearAllMocks();
    mockState.emitRows = null;
    mockState.emitStatus = null;
  });

  it("reports loading, never the previous view's rows, on that render", async () => {
    const seen: Array<{ asked: string; status: string; rows: ListRow[] }> = [];
    function Probe({ ns }: { ns: string }) {
      const list = useResourceList("prod", "pods", watched, ns, []);
      seen.push({ asked: ns, status: list.status, rows: list.rows });
      return null;
    }

    const { rerender } = render(<Probe ns="default" />);
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([{ name: "a" }]));
    await waitFor(() => expect(seen.at(-1)?.status).toBe("ready"));
    seen.length = 0;

    rerender(<Probe ns="kube-system" />);

    // The FIRST render at the new view is the frame a browser would paint. It
    // must not put `default`'s pods under a table now headed kube-system —
    // where every row action would run against the namespace the reader left.
    expect(seen[0]).toEqual({ asked: "kube-system", status: "loading", rows: [] });
    // No render in between may leak them either.
    expect(seen.filter((s) => s.asked === "kube-system" && s.rows.length > 0)).toEqual([]);
  });

  it("pairs no view's metrics with another view's rows on that render", async () => {
    // Enrichment is held apart from the rows and merged at render, so the
    // gate has to cover it too: a readings map from the namespace the reader
    // left, merged into rows by identity, is how one pod's CPU lands on
    // another's row — displayed, and sorted on.
    const enrich = vi.fn().mockResolvedValue(new Map([[rowKey({ name: "api-0", namespace: "shop" }), { cpu: 10 }]]));
    const d: KindDescriptor<ListRow> = { ...watched, enrich, enrichMs: 10000 };

    const seen: Array<{ asked: string; rows: ListRow[] }> = [];
    function Probe({ ns }: { ns: string }) {
      const list = useResourceList("prod", "pods", d, ns, []);
      seen.push({ asked: ns, rows: list.rows });
      return null;
    }

    const { rerender } = render(<Probe ns="shop" />);
    await waitFor(() => expect(mockState.emitRows).not.toBeNull());
    act(() => mockState.emitRows!([{ name: "api-0", namespace: "shop" }]));
    await waitFor(() => expect(seen.at(-1)?.rows[0]).toMatchObject({ cpu: 10 }));
    seen.length = 0;

    rerender(<Probe ns="billing" />);
    expect(seen[0]).toEqual({ asked: "billing", rows: [] });
  });

  it("keeps serving the rows across a reload of the same view", async () => {
    // The gate compares views, not fetch generations: `reload` does not change
    // the view, so it must not empty the table on its way to a refetch.
    const load = vi.fn().mockResolvedValue({ rows: [{ name: "a" }] });
    const polled: KindDescriptor<ListRow> = { ...watched, source: "poll", load };

    const seen: string[] = [];
    let reload!: () => void;
    function Probe() {
      const list = useResourceList("prod", "leases", polled, "default", []);
      reload = list.reload;
      seen.push(list.status);
      return null;
    }

    render(<Probe />);
    await waitFor(() => expect(seen.at(-1)).toBe("ready"));
    seen.length = 0;

    act(() => reload());
    expect(seen[0]).toBe("ready");
  });
});
