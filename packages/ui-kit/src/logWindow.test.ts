import { describe, it, expect } from "vitest";
import { computeLogWindow } from "./logWindow";

describe("computeLogWindow", () => {
  it("renders everything when wrap is on (rows are variable-height)", () => {
    const w = computeLogWindow({ total: 5000, scrollTop: 1000, viewportHeight: 400, rowHeight: 16, wrap: true });
    expect(w.virtualized).toBe(false);
    expect(w).toMatchObject({ start: 0, end: 5000, topPad: 0, bottomPad: 0 });
  });

  it("renders everything below the virtualize threshold", () => {
    const w = computeLogWindow({ total: 40, scrollTop: 0, viewportHeight: 400, rowHeight: 16, wrap: false });
    expect(w.virtualized).toBe(false);
    expect(w.end).toBe(40);
  });

  it("renders everything before a row height can be measured (jsdom / first paint)", () => {
    const w = computeLogWindow({ total: 5000, scrollTop: 0, viewportHeight: 400, rowHeight: 0, wrap: false });
    expect(w.virtualized).toBe(false);
    expect(w.end).toBe(5000);
  });

  it("windows to the scrolled region with spacers that preserve the full scroll height", () => {
    const w = computeLogWindow({
      total: 5000,
      scrollTop: 1600,
      viewportHeight: 320,
      rowHeight: 16,
      wrap: false,
      overscan: 10,
      threshold: 100,
    });
    expect(w.virtualized).toBe(true);
    // firstVisible = 1600/16 = 100; visibleCount = 320/16 = 20; overscan 10.
    expect(w.start).toBe(90);
    expect(w.end).toBe(130);
    expect(w.topPad).toBe(90 * 16);
    expect(w.bottomPad).toBe((5000 - 130) * 16);
    // Spacers + rendered rows reconstruct the whole virtual list height.
    expect(w.topPad + (w.end - w.start) * 16 + w.bottomPad).toBe(5000 * 16);
  });
});
