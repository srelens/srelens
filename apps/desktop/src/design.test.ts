import { describe, it, expect, beforeEach } from "vitest";
import { DESIGN_KEY, loadDesign, saveDesign } from "./design";

beforeEach(() => localStorage.clear());

describe("the design preference", () => {
  it("defaults to classic, so an untouched install is unchanged", () => {
    expect(loadDesign()).toBe("classic");
  });

  it("round-trips a choice", () => {
    saveDesign("next");
    expect(loadDesign()).toBe("next");
    saveDesign("classic");
    expect(loadDesign()).toBe("classic");
  });

  it("falls back to classic on a value it does not recognise", () => {
    // Written by a future version, or by hand. Never leave someone on a design
    // that does not exist — they would get a blank window with no way back.
    localStorage.setItem(DESIGN_KEY, "hologram");
    expect(loadDesign()).toBe("classic");
  });

  it("survives storage being unavailable", () => {
    // Storage throws in some privacy modes. A preference is not worth failing
    // to boot over.
    const original = Storage.prototype.getItem;
    Storage.prototype.getItem = () => {
      throw new Error("denied");
    };
    try {
      expect(loadDesign()).toBe("classic");
    } finally {
      Storage.prototype.getItem = original;
    }
  });

  it("does not throw when the choice cannot be saved", () => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = () => {
      throw new Error("quota");
    };
    try {
      expect(() => saveDesign("next")).not.toThrow();
    } finally {
      Storage.prototype.setItem = original;
    }
  });
});
