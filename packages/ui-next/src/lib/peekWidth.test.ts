import { describe, it, expect, beforeEach } from "vitest";
import {
  DEFAULT_PEEK_WIDTH,
  MAX_PEEK_WIDTH,
  MIN_LIST_WIDTH,
  MIN_PEEK_WIDTH,
  PEEK_WIDTH_KEY,
  clampPeekWidth,
  loadPeekWidth,
  peekBounds,
  peekWidth,
  savePeekWidth,
  setPeekWidth,
} from "./peekWidth";

describe("the peek's width", () => {
  beforeEach(() => {
    localStorage.clear();
    loadPeekWidth();
  });

  it("opens at the width the pane shipped as", () => {
    expect(peekWidth()).toBe(DEFAULT_PEEK_WIDTH);
  });

  it("survives a reload", () => {
    savePeekWidth(480);
    loadPeekWidth();
    expect(peekWidth()).toBe(480);
  });

  it("is a snapshot that cannot tear", () => {
    // A number, so `useSyncExternalStore` compares it by value and a reader
    // cannot be re-rendered forever by a freshly composed object.
    expect(typeof peekWidth()).toBe("number");
    setPeekWidth(400);
    expect(peekWidth()).toBe(peekWidth());
  });

  it("refuses a width the pane could never honour", () => {
    savePeekWidth(MAX_PEEK_WIDTH + 400);
    expect(peekWidth()).toBe(MAX_PEEK_WIDTH);
    savePeekWidth(20);
    expect(peekWidth()).toBe(MIN_PEEK_WIDTH);
  });

  it("keeps the width the reader chose, whatever the room happens to be", () => {
    // The layout clamp is applied on the way out, by whoever knows the room.
    // Writing it back would mean a pane squeezed once by a narrow window had
    // permanently forgotten what it was dragged to.
    savePeekWidth(MAX_PEEK_WIDTH);
    expect(clampPeekWidth(peekWidth(), peekBounds(MIN_LIST_WIDTH + 300))).toBe(300);
    expect(peekWidth()).toBe(MAX_PEEK_WIDTH);
    expect(JSON.parse(localStorage.getItem(PEEK_WIDTH_KEY)!)).toBe(MAX_PEEK_WIDTH);
  });

  it("reads nonsense as no stored width at all", () => {
    for (const raw of ['"wide"', "null", "{}", "0", "-40", "not json"]) {
      localStorage.setItem(PEEK_WIDTH_KEY, raw);
      loadPeekWidth();
      expect(peekWidth(), raw).toBe(DEFAULT_PEEK_WIDTH);
    }
  });

  it("costs the width and nothing else when storage refuses", () => {
    const throwing = {
      getItem() {
        throw new DOMException("denied");
      },
      setItem() {
        throw new DOMException("denied");
      },
      removeItem() {
        throw new DOMException("denied");
      },
    };
    expect(() => loadPeekWidth(throwing)).not.toThrow();
    expect(peekWidth()).toBe(DEFAULT_PEEK_WIDTH);
    expect(() => savePeekWidth(400, throwing)).not.toThrow();
    // The drag still worked; only the memory of it was lost.
    expect(peekWidth()).toBe(400);
  });

  it("only writes when the resize settles", () => {
    setPeekWidth(400);
    expect(peekWidth()).toBe(400);
    // Every pixel of a drag comes through `setPeekWidth`; a write per pixel is
    // what `ResizeHandle`'s two callbacks exist to avoid.
    expect(localStorage.getItem(PEEK_WIDTH_KEY)).toBeNull();
    savePeekWidth(400);
    expect(localStorage.getItem(PEEK_WIDTH_KEY)).toBe("400");
  });
});

/**
 * The bounds, as a pure function of the room the list and the peek share.
 *
 * A function of that row and not of the window, which is the fix this round
 * is about: the cluster rail and the navigation sidebar are outside the list,
 * and the sidebar is itself resizable, so the window is space the list has no
 * claim on.
 */
describe("the peek's bounds", () => {
  it("leaves the list its floor", () => {
    // 900px between them, so the peek stops at 540 and the list keeps 360 —
    // well inside the absolute ceiling, which is not what is biting here.
    expect(peekBounds(900)).toEqual({ minWidth: MIN_PEEK_WIDTH, maxWidth: 900 - MIN_LIST_WIDTH });
    // Whatever the row, and whatever the peek is dragged to inside it, the
    // list keeps a usable table — the property the floor constant is for.
    for (const available of [700, 900, 1100, 1400, 2600]) {
      const widest = clampPeekWidth(Infinity, peekBounds(available));
      expect(available - widest, `${available}px row`).toBeGreaterThanOrEqual(MIN_LIST_WIDTH);
    }
  });

  it("stops at its own ceiling in a row with room to spare", () => {
    expect(peekBounds(4000).maxWidth).toBe(MAX_PEEK_WIDTH);
  });

  it("stays legible in a row too narrow for both", () => {
    // The one case the floor gives way: a pane below its minimum cannot show
    // what it holds, so the minimum wins and the table scrolls inside itself.
    expect(peekBounds(MIN_LIST_WIDTH + 100).maxWidth).toBe(MIN_PEEK_WIDTH);
    expect(peekBounds(100).maxWidth).toBe(MIN_PEEK_WIDTH);
  });

  it("treats an unmeasured row as no constraint rather than as no room", () => {
    // A first render, or a host that does no layout. React runs the measuring
    // layout effect before paint, so this is never a frame the reader sees —
    // but reading it as zero room would pin the pane to its minimum.
    expect(peekBounds(0).maxWidth).toBe(MAX_PEEK_WIDTH);
  });
});
