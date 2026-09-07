import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, render, renderHook, waitFor } from "@testing-library/react";
import {
  CACHE_TTL_MS,
  clearResourceCache,
  useCachedResource,
  __cacheSizeForTests,
} from "./cachedResource";

beforeEach(() => {
  clearResourceCache();
  vi.useRealTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useCachedResource — returning to a tab", () => {
  it("paints the last good answer on the next mount without loading again", async () => {
    const load = vi.fn().mockResolvedValue(["a", "b"]);
    const first = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(first.result.current.status).toBe("ready"));
    first.unmount();

    const second = renderHook(() => useCachedResource("nodes|prod", load));
    // No loading flash and no second call: this is what makes returning to
    // the tab instant rather than a spinner over data we already have.
    expect(second.result.current.status).toBe("ready");
    expect(second.result.current.data).toEqual(["a", "b"]);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("shows the cached rows while it refreshes them once the TTL has passed", async () => {
    const load = vi.fn().mockResolvedValueOnce(["old"]).mockResolvedValueOnce(["new"]);
    const first = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(first.result.current.data).toEqual(["old"]));
    first.unmount();

    const now = vi.spyOn(Date, "now");
    now.mockReturnValue(Date.now() + CACHE_TTL_MS + 1);
    const second = renderHook(() => useCachedResource("nodes|prod", load));
    // Stale-while-revalidate: rows first, spinner never.
    expect(second.result.current.status).toBe("ready");
    expect(second.result.current.data).toEqual(["old"]);
    await waitFor(() => expect(second.result.current.data).toEqual(["new"]));
    expect(load).toHaveBeenCalledTimes(2);
    now.mockRestore();
  });

  it("serves one load to every reader that asks for the same thing at once", async () => {
    const load = vi.fn().mockResolvedValue(["a"]);
    const a = renderHook(() => useCachedResource("nodes|prod", load));
    const b = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(a.result.current.status).toBe("ready"));
    await waitFor(() => expect(b.result.current.status).toBe("ready"));
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("keeps one cluster's answer out of another's", async () => {
    const load = vi.fn((key: string) => Promise.resolve([key]));
    const a = renderHook(() => useCachedResource("nodes|prod", () => load("prod")));
    await waitFor(() => expect(a.result.current.data).toEqual(["prod"]));
    const b = renderHook(() => useCachedResource("nodes|dev", () => load("dev")));
    await waitFor(() => expect(b.result.current.data).toEqual(["dev"]));
    expect(a.result.current.data).toEqual(["prod"]);
  });
});

describe("useCachedResource — stale data says it is stale", () => {
  it("keeps the rows when a refresh fails, and marks them no longer refreshing", async () => {
    const load = vi
      .fn()
      .mockResolvedValueOnce(["a"])
      .mockRejectedValueOnce(new Error("nodes is forbidden"));
    const first = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(first.result.current.data).toEqual(["a"]));
    first.unmount();

    const now = vi.spyOn(Date, "now");
    now.mockReturnValue(Date.now() + CACHE_TTL_MS + 1);
    const second = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(second.result.current.stale).toBe(true));

    // The rows the reader can see are still the last good ones — throwing
    // them away for an error state loses information nobody asked to lose.
    expect(second.result.current.data).toEqual(["a"]);
    // And the screen is told, so it can say so rather than pretending the
    // figures on it are current.
    expect(second.result.current.error).toContain("forbidden");
    now.mockRestore();
  });

  it("is an error, not stale data, when the first load fails with nothing cached", async () => {
    const load = vi.fn().mockRejectedValue(new Error("nodes is forbidden"));
    const { result } = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.stale).toBe(false);
    expect(result.current.data).toBeUndefined();
  });

  it("stops being stale once a refresh succeeds again", async () => {
    const load = vi
      .fn()
      .mockResolvedValueOnce(["a"])
      .mockRejectedValueOnce(new Error("boom"))
      .mockResolvedValueOnce(["b"]);
    const { result } = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(result.current.data).toEqual(["a"]));

    act(() => result.current.reload());
    await waitFor(() => expect(result.current.stale).toBe(true));

    act(() => result.current.reload());
    await waitFor(() => expect(result.current.data).toEqual(["b"]));
    expect(result.current.stale).toBe(false);
    expect(result.current.error).toBeUndefined();
  });
});

describe("useCachedResource — a late answer is never believed", () => {
  it("discards a response for a context the reader has left", async () => {
    let answerProd!: (v: string[]) => void;
    const load = vi.fn((key: string) =>
      key === "prod"
        ? new Promise<string[]>((resolve) => {
            answerProd = resolve;
          })
        : Promise.resolve(["dev-rows"]),
    );

    const { result, rerender } = renderHook(
      ({ ctx }: { ctx: string }) => useCachedResource(`nodes|${ctx}`, () => load(ctx)),
      { initialProps: { ctx: "prod" } },
    );
    rerender({ ctx: "dev" });
    await waitFor(() => expect(result.current.data).toEqual(["dev-rows"]));

    // Flushed inside `act`: an update React schedules outside it never
    // reaches `result.current` before the assertion, so this test would pass
    // with the guard removed.
    await act(async () => {
      answerProd(["prod-rows"]);
      await Promise.resolve();
    });
    expect(result.current.data).toEqual(["dev-rows"]);
  });

  it("cannot have a cleared cache repopulated by a load already in flight", async () => {
    let answer!: (v: string[]) => void;
    const load = vi.fn(
      () =>
        new Promise<string[]>((resolve) => {
          answer = resolve;
        }),
    );
    const { unmount } = renderHook(() => useCachedResource("nodes|prod", load));
    unmount();

    clearResourceCache();
    await act(async () => {
      answer(["late"]);
      await Promise.resolve();
    });

    // Deleting the pending promise cannot cancel its `.then`; only a
    // generation captured at the start can stop a reset being undone.
    expect(__cacheSizeForTests()).toBe(0);
    const load2 = vi.fn().mockResolvedValue(["fresh"]);
    const { result } = renderHook(() => useCachedResource("nodes|prod", load2));
    expect(result.current.status).toBe("loading");
    await waitFor(() => expect(result.current.data).toEqual(["fresh"]));
  });
});

describe("useCachedResource — the cache is bounded", () => {
  it("evicts the oldest entry rather than growing without end", async () => {
    for (let i = 0; i < 45; i++) {
      const { unmount } = renderHook(() =>
        useCachedResource(`nodes|c${i}`, () => Promise.resolve([i])),
      );
      await waitFor(() => expect(__cacheSizeForTests()).toBeGreaterThan(0));
      unmount();
    }
    expect(__cacheSizeForTests()).toBe(40);

    // The oldest key is gone and loads again; the newest is still served.
    const oldest = vi.fn().mockResolvedValue(["again"]);
    const a = renderHook(() => useCachedResource("nodes|c0", oldest));
    expect(a.result.current.status).toBe("loading");

    const newest = vi.fn().mockResolvedValue(["unused"]);
    const b = renderHook(() => useCachedResource("nodes|c44", newest));
    expect(b.result.current.status).toBe("ready");
    expect(newest).not.toHaveBeenCalled();
  });

  it("forces a refresh through reload even inside the TTL", async () => {
    const load = vi.fn().mockResolvedValueOnce(["a"]).mockResolvedValueOnce(["b"]);
    const { result } = renderHook(() => useCachedResource("nodes|prod", load));
    await waitFor(() => expect(result.current.data).toEqual(["a"]));

    act(() => result.current.reload());
    await waitFor(() => expect(result.current.data).toEqual(["b"]));
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("reports an empty answer as empty, by the caller's own rule", async () => {
    const a = renderHook(() => useCachedResource("k|1", () => Promise.resolve([])));
    await waitFor(() => expect(a.result.current.status).toBe("empty"));
    const b = renderHook(() =>
      useCachedResource("k|2", () => Promise.resolve({ n: 0 }), (v) => v.n === 0),
    );
    await waitFor(() => expect(b.result.current.status).toBe("empty"));
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
 * It is reachable on a MOUNTED hook, not only at mount: `Overview` and
 * `Resources` render their inner component with no `key`, and no screen
 * carries `key={name}` (see `lib/clusterMoved.tsx`), so switching cluster
 * changes the key on a hook that stays put.
 */
describe("useCachedResource — the render the key changes", () => {
  it("reports loading, never the previous key's data, on that render", async () => {
    const load = vi.fn((cluster: string) => Promise.resolve([cluster]));

    const seen: Array<{
      asked: string;
      status: string;
      data?: string[];
      stale: boolean;
      updatedAt?: number;
    }> = [];
    function Probe({ cluster }: { cluster: string }) {
      const r = useCachedResource(`nodes|${cluster}`, () => load(cluster));
      seen.push({ asked: cluster, status: r.status, data: r.data, stale: r.stale, updatedAt: r.updatedAt });
      return null;
    }

    const { rerender } = render(<Probe cluster="prod" />);
    await waitFor(() => expect(seen.at(-1)?.data).toEqual(["prod"]));
    seen.length = 0;

    rerender(<Probe cluster="dev" />);

    // The FIRST render at the new key is the frame a browser would paint. It
    // must not carry prod's nodes — nor prod's `updatedAt`, which is the
    // figure the screen prints next to them — under a caller now asking
    // about dev.
    expect(seen[0]).toEqual({
      asked: "dev",
      status: "loading",
      data: undefined,
      stale: false,
      updatedAt: undefined,
    });
    // No render in between may leak them either.
    expect(seen.filter((s) => s.asked === "dev" && s.data?.[0] === "prod")).toEqual([]);

    await waitFor(() => expect(seen.at(-1)?.data).toEqual(["dev"]));
  });

  it("keeps serving the data across a reload of the same key", async () => {
    // The gate compares keys, not fetch generations: `reload` does not change
    // the key, so it must not blank the screen on its way to a refetch — that
    // would flash a spinner over figures that are still correct, which is the
    // whole thing this hook exists to remove.
    const load = vi.fn().mockResolvedValue(["a"]);

    const seen: string[] = [];
    let reload!: () => void;
    function Probe() {
      const r = useCachedResource("nodes|prod", load);
      reload = r.reload;
      seen.push(r.status);
      return null;
    }

    render(<Probe />);
    await waitFor(() => expect(seen.at(-1)).toBe("ready"));
    seen.length = 0;

    act(() => reload());
    expect(seen[0]).toBe("ready");
  });
});
