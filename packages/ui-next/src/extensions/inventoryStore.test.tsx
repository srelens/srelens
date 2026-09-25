import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({
  listExtensions: vi.fn(),
  onExtensionInventoryChanged: vi.fn(),
  tauri: true,
}));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  listExtensions: core.listExtensions,
  onExtensionInventoryChanged: core.onExtensionInventoryChanged,
  isTauri: () => core.tauri,
}));

import { useExtensions } from "./inventoryStore";

const inventory = (nextRevision: number) => ({ schemaVersion: 1, nextRevision, plugins: [] });
let announce: () => void = () => {};

beforeEach(() => {
  vi.useFakeTimers();
  core.tauri = true;
  core.listExtensions.mockReset();
  core.onExtensionInventoryChanged.mockReset();
  let revision = 1;
  core.listExtensions.mockImplementation(async () => inventory(revision++));
  core.onExtensionInventoryChanged.mockImplementation(async (listener: () => void) => {
    announce = listener;
    return () => { announce = () => {}; };
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function settle() {
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
}

describe("the app inventory store (#566)", () => {
  it("reads again when the host announces a write, and never polls", async () => {
    const { result } = renderHook(() => useExtensions());
    await settle();
    expect(result.current.status).toBe("ready");
    expect(result.current.updates).toEqual({ mode: "live" });
    expect(core.listExtensions).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(core.listExtensions).toHaveBeenCalledTimes(1);
    act(() => announce());
    await settle();
    expect(core.listExtensions).toHaveBeenCalledTimes(2);
    expect(result.current.data?.nextRevision).toBe(2);
  });

  it("stops listening when the last consumer leaves", async () => {
    const { unmount } = renderHook(() => useExtensions());
    await settle();
    const stop = await core.onExtensionInventoryChanged.mock.results[0].value;
    expect(typeof stop).toBe("function");
    unmount();
    act(() => announce());
    await settle();
    expect(core.listExtensions).toHaveBeenCalledTimes(1);
  });

  it("falls back to reading every five seconds when it cannot listen, and says so", async () => {
    core.onExtensionInventoryChanged.mockRejectedValue(new Error("event channel unavailable"));
    const { result } = renderHook(() => useExtensions());
    await settle();
    expect(result.current.updates).toEqual({ mode: "polling", reason: "event channel unavailable" });
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(core.listExtensions).toHaveBeenCalledTimes(3);
  });

  /**
   * The web server keeps each user's own inventory (#515), so the list is read
   * from it. It announces nothing, so there is nothing to listen to and the
   * store does not claim to be live, nor poll: it reads again on focus, where
   * a change made in another tab shows.
   */
  it("lists the user's own apps on the web, and reads again on focus rather than listening", async () => {
    core.tauri = false;
    const { result } = renderHook(() => useExtensions());
    await settle();
    expect(result.current.status).toBe("ready");
    expect(result.current.data?.nextRevision).toBe(1);
    expect(result.current.updates).toEqual({ mode: "none" });
    expect(core.onExtensionInventoryChanged).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(core.listExtensions).toHaveBeenCalledTimes(1);
    act(() => { window.dispatchEvent(new Event("focus")); });
    await settle();
    expect(core.listExtensions).toHaveBeenCalledTimes(2);
    expect(result.current.data?.nextRevision).toBe(2);
  });
});
