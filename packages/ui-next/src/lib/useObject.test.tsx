import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, renderHook, waitFor } from "@testing-library/react";
import type { K8sObject } from "@srelens/core";
import { useObject } from "./useObject";

vi.mock("@srelens/core", () => ({
  getObject: vi.fn(),
}));

import { getObject } from "@srelens/core";

const mockedGetObject = vi.mocked(getObject);

describe("useObject", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("settles on ready with the object", async () => {
    const object: K8sObject = { kind: "Pod", metadata: { name: "web-1" } };
    mockedGetObject.mockResolvedValueOnce({ object });
    const { result } = renderHook(() => useObject("ctx", "Pod", "default", "web-1"));
    expect(result.current.status).toBe("loading");
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.object).toEqual(object);
    expect(result.current.error).toBeUndefined();
  });

  it("settles on error with the message when getObject resolves with an error field", async () => {
    mockedGetObject.mockResolvedValueOnce({ error: "not found" });
    const { result } = renderHook(() => useObject("ctx", "Pod", "default", "missing"));
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error).toBe("not found");
    expect(result.current.object).toBeUndefined();
  });

  it("settles on error when getObject's promise rejects rather than resolving with an error field", async () => {
    mockedGetObject.mockRejectedValueOnce(new Error("boom"));
    const { result } = renderHook(() => useObject("ctx", "Pod", "default", "web-1"));
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error).toBe("boom");
  });

  it("drops a result that arrives after the target changed", async () => {
    let resolveFirst!: (v: { object?: K8sObject; error?: string }) => void;
    mockedGetObject.mockImplementationOnce(
      () => new Promise((r) => { resolveFirst = r; }),
    );
    const second: K8sObject = { kind: "Pod", metadata: { name: "web-2" } };
    mockedGetObject.mockResolvedValueOnce({ object: second });

    const { result, rerender } = renderHook(
      ({ name }: { name: string }) => useObject("ctx", "Pod", "default", name),
      { initialProps: { name: "web-1" } },
    );

    rerender({ name: "web-2" });
    await waitFor(() => expect(result.current.object).toEqual(second));

    // The stale first-target promise resolves after the rerender switched
    // targets; it must be dropped, not clobber the now-current object.
    const stale: K8sObject = { kind: "Pod", metadata: { name: "web-1" } };
    await act(async () => {
      resolveFirst({ object: stale });
      await Promise.resolve();
    });
    expect(result.current.object).toEqual(second);
    expect(result.current.status).toBe("ready");
  });

  it("reload() re-fetches", async () => {
    const first: K8sObject = { kind: "Pod", metadata: { name: "web-1", labels: { v: "1" } } };
    const second: K8sObject = { kind: "Pod", metadata: { name: "web-1", labels: { v: "2" } } };
    mockedGetObject.mockResolvedValueOnce({ object: first }).mockResolvedValueOnce({ object: second });

    const { result } = renderHook(() => useObject("ctx", "Pod", "default", "web-1"));
    await waitFor(() => expect(result.current.object).toEqual(first));

    act(() => result.current.reload());
    await waitFor(() => expect(result.current.object).toEqual(second));
    expect(mockedGetObject).toHaveBeenCalledTimes(2);
  });
  // The render-time gate. Everything above settles first and then asserts,
  // which is blind to this: RTL flushes passive effects synchronously, so by
  // the time a `waitFor` resolves the effect has already reset the state to
  // "loading" and any bad frame has been overwritten. A real browser paints
  // whatever was committed, and what gets committed is decided during render,
  // before any effect runs. So this observes the hook's return value AT RENDER
  // TIME instead.
  it("reports loading, never the previous target's object, on the render the target changes", async () => {
    const first: K8sObject = { kind: "Pod", metadata: { name: "web-1" } };
    const second: K8sObject = { kind: "Pod", metadata: { name: "web-2" } };
    mockedGetObject.mockResolvedValueOnce({ object: first }).mockResolvedValueOnce({ object: second });

    const seen: Array<{ asked: string; status: string; object?: K8sObject }> = [];
    function Probe({ name }: { name: string }) {
      const resource = useObject("ctx", "Pod", "default", name);
      seen.push({ asked: name, status: resource.status, object: resource.object });
      return null;
    }

    const { rerender } = render(<Probe name="web-1" />);
    await waitFor(() => expect(seen.at(-1)?.object).toEqual(first));
    seen.length = 0;

    // The subject changes on a hook that stays MOUNTED — the peek pane's
    // ordinary case, and the only one that can commit a mismatched frame.
    rerender(<Probe name="web-2" />);

    // The FIRST render at the new target is the frame a browser would paint.
    // It must not carry web-1's object under a caller that is now rendering
    // web-2's name.
    expect(seen[0]).toEqual({ asked: "web-2", status: "loading", object: undefined });
    // No render in between may leak it either.
    expect(seen.filter((s) => s.asked === "web-2" && s.object === first)).toEqual([]);

    await waitFor(() => expect(seen.at(-1)?.object).toEqual(second));
  });

  it("keeps serving the object across a reload of the same target", async () => {
    // The gate compares targets, not fetch generations: `reload` does not
    // change the target, so it must not blank the pane on its way to a
    // refetch — that would flash a loading state over data that is still
    // correct. (The refetch itself is asserted above.)
    const object: K8sObject = { kind: "Pod", metadata: { name: "web-1" } };
    mockedGetObject.mockResolvedValue({ object });

    const seen: string[] = [];
    let reload!: () => void;
    function Probe() {
      const resource = useObject("ctx", "Pod", "default", "web-1");
      reload = resource.reload;
      seen.push(resource.status);
      return null;
    }

    render(<Probe />);
    await waitFor(() => expect(seen.at(-1)).toBe("ready"));
    seen.length = 0;

    act(() => reload());
    // The render `reload()` schedules still holds the same target, so the
    // gate lets the settled object through; the effect's own reset is what
    // moves it to loading afterwards.
    expect(seen[0]).toBe("ready");
  });
});
