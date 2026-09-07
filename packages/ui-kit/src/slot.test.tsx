import { describe, it, expect } from "vitest";
import { filled } from "./slot";

describe("filled", () => {
  it("accepts anything that renders", () => {
    expect(filled("text")).toBe(true);
    expect(filled(0)).toBe(true);
    expect(filled(<span />)).toBe(true);
  });

  it("rejects what renders nothing", () => {
    expect(filled(null)).toBe(false);
    expect(filled(undefined)).toBe(false);
    expect(filled("")).toBe(false);
  });

  it("rejects the boolean a conditional slot hands over", () => {
    // `action={canCreate && <Button />}` is the ordinary way to make a slot
    // conditional, and it passes `false`, not nothing. (#325 review)
    expect(filled(false)).toBe(false);
    expect(filled(true)).toBe(false);
  });

  it("keeps zero, which renders", () => {
    // A count of 0 is a real thing to show; dropping it would hide a figure
    // the caller meant to display.
    expect(filled(0)).toBe(true);
  });
  it("rejects an array that renders nothing", () => {
    // `actions={items.map(...)}` is the other ordinary conditional slot, and
    // an empty list hands over `[]`, not nothing. (#325 review)
    expect(filled([])).toBe(false);
    expect(filled([false, null, undefined])).toBe(false);
    expect(filled([[], [false]])).toBe(false);
  });

  it("accepts an array with anything renderable in it", () => {
    expect(filled([<span key="a" />])).toBe(true);
    expect(filled([false, <span key="a" />])).toBe(true);
    expect(filled([0])).toBe(true);
    expect(filled([[<span key="a" />]])).toBe(true);
  });
  it("rejects a fragment that renders nothing", () => {
    // `actions={<>{items.map(...)}</>}` groups conditional content, and an
    // empty result is still a valid element. (#325 review)
    expect(filled(<></>)).toBe(false);
    expect(filled(<>{[]}</>)).toBe(false);
    expect(filled(<>{false}</>)).toBe(false);
    expect(filled(<><>{[]}</></>)).toBe(false);
  });

  it("accepts a fragment with anything renderable in it", () => {
    expect(filled(<><span /></>)).toBe(true);
    expect(filled(<>{false}<span /></>)).toBe(true);
  });
});
