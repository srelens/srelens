import { act, renderHook } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useTabStripScroll } from "./useTabStripScroll";

it("measures edges, scrolls by a page, and follows resize and scroll events", () => {
  const node = document.createElement("div");
  let width = 100;
  Object.defineProperties(node, { clientWidth: { get: () => width }, scrollWidth: { get: () => 500 } });
  node.scrollBy = vi.fn();
  const { result, unmount } = renderHook(() => useTabStripScroll({ current: node }, "a|b"));
  expect(result.current.left).toBe(false); expect(result.current.right).toBe(true);
  act(() => result.current.click(1));
  expect(node.scrollBy).toHaveBeenCalledWith({ left: 80, behavior: "smooth" });
  act(() => { node.scrollLeft = 400; node.dispatchEvent(new Event("scroll")); });
  expect(result.current.left).toBe(true); expect(result.current.right).toBe(false);
  act(() => { width = 500; node.scrollLeft = 0; window.dispatchEvent(new Event("resize")); });
  expect(result.current.left).toBe(false); expect(result.current.right).toBe(false);
  unmount();
});

it("repeats during a hold, stops on release/unmount, and respects reduced motion", () => {
  vi.useFakeTimers();
  const original = window.matchMedia;
  window.matchMedia = vi.fn().mockReturnValue({ matches: true });
  try {
    const node = document.createElement("div");
    Object.defineProperties(node, { clientWidth: { value: 100 }, scrollWidth: { value: 500 } });
    node.scrollBy = vi.fn();
    const ref = { current: node };
    const { result, unmount } = renderHook(() => useTabStripScroll(ref, "a|b"));
    act(() => result.current.click(1));
    expect(node.scrollBy).toHaveBeenCalledWith({ left: 80, behavior: "auto" });
    vi.mocked(node.scrollBy).mockClear();
    act(() => { result.current.start(1); vi.advanceTimersByTime(550); });
    expect(node.scrollBy).toHaveBeenCalledTimes(3);
    act(() => { result.current.stop(); result.current.click(1); vi.advanceTimersByTime(500); });
    expect(node.scrollBy).toHaveBeenCalledTimes(3);
    act(() => result.current.start(-1));
    unmount(); vi.advanceTimersByTime(1000);
    expect(node.scrollBy).toHaveBeenCalledTimes(3);
  } finally { window.matchMedia = original; vi.useRealTimers(); }
});
