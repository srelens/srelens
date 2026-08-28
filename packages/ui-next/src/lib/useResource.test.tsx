import { describe, it, expect, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useResource } from "./useResource";

describe("useResource", () => {
  it("goes loading → ready", async () => {
    const { result } = renderHook(() => useResource(() => Promise.resolve([1]), []));
    expect(result.current.status).toBe("loading");
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.data).toEqual([1]);
  });
  it("reports empty by the default predicate and by a custom one", async () => {
    const a = renderHook(() => useResource(() => Promise.resolve([]), []));
    await waitFor(() => expect(a.result.current.status).toBe("empty"));
    const b = renderHook(() => useResource(() => Promise.resolve({ n: 0 }), [], (v) => v.n === 0));
    await waitFor(() => expect(b.result.current.status).toBe("empty"));
  });
  it("reports an error with its message and retries through reload", async () => {
    const load = vi.fn().mockRejectedValueOnce(new Error("boom")).mockResolvedValueOnce("ok");
    const { result } = renderHook(() => useResource(load, []));
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error).toBe("boom");
    act(() => result.current.reload());
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(load).toHaveBeenCalledTimes(2);
  });
  it("ignores a result that lands after a newer load began", async () => {
    let resolveFirst!: (v: string) => void;
    const load = vi.fn()
      .mockImplementationOnce(() => new Promise<string>((r) => { resolveFirst = r; }))
      .mockResolvedValueOnce("second");
    const { result } = renderHook(() => useResource(load, []));
    act(() => result.current.reload());
    await waitFor(() => expect(result.current.data).toBe("second"));
    // The late resolution has to be flushed *inside* act: an update React
    // schedules outside act never reaches `result.current` before the
    // assertion runs, so the check would pass even with the guard removed.
    await act(async () => { resolveFirst("first"); await Promise.resolve(); });
    expect(result.current.data).toBe("second");
  });
  it("ignores a result after unmount", async () => {
    let resolve!: (v: string) => void;
    // `result.current` freezes at the last render once the hook is unmounted,
    // so it cannot witness a dropped result on its own. The predicate can:
    // useResource only consults isEmpty for a result it intends to keep.
    const isEmpty = vi.fn((v: string) => v === "");
    const { result, unmount } = renderHook(() => useResource(() => new Promise<string>((r) => { resolve = r; }), [], isEmpty));
    unmount();
    await act(async () => { resolve("late"); await Promise.resolve(); });
    expect(isEmpty).not.toHaveBeenCalled();
    expect(result.current.status).toBe("loading");
  });
});
