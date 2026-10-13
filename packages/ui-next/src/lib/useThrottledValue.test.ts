import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useThrottledValue } from "./useThrottledValue";

describe("useThrottledValue", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  /** Every value a render handed back, in order. */
  let rendered: number[] = [];
  const hook = (ms: number) => {
    rendered = [];
    return renderHook(
      ({ value, wait }) => {
        const shown = useThrottledValue(value, wait);
        rendered.push(shown);
        return shown;
      },
      { initialProps: { value: 0, wait: ms } },
    );
  };

  it("lands a change after a quiet spell at once, and the last change inside the spell when it ends", () => {
    const { result, rerender } = hook(250);
    rerender({ value: 1, wait: 250 });
    expect(result.current).toBe(1);
    act(() => vi.advanceTimersByTime(100));
    rerender({ value: 2, wait: 250 });
    rerender({ value: 3, wait: 250 });
    expect(result.current).toBe(1);
    act(() => vi.advanceTimersByTime(149));
    expect(result.current).toBe(1);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toBe(3);
  });

  it("hands the value straight through with no wait, and does not go back to an older one when the wait returns", () => {
    const { result, rerender } = hook(250);
    rerender({ value: 1, wait: 250 });
    rendered = [];
    rerender({ value: 2, wait: 0 });
    rerender({ value: 3, wait: 0 });
    expect(result.current).toBe(3);
    // Not one render behind, either.
    expect(rendered.filter((v) => v < 2)).toEqual([]);
    rerender({ value: 3, wait: 250 });
    expect(result.current).toBe(3);
  });
});
