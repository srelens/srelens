import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Gallery } from "./Gallery";
import * as kit from "../index";

describe("Gallery", () => {
  it("shows every component the kit exports", async () => {
    // Derived from the barrel rather than a hand-written list, which would
    // drift the first time someone was in a hurry. The catalogue is the only
    // visual review surface this design has.
    //
    // One name is excused, and it is named rather than pattern-matched so that
    // the next component someone finds inconvenient to draw cannot excuse
    // itself: `PortalScopeProvider` renders no DOM at all. It is a context
    // around a part of the window that owns the layers opened inside it, and a
    // catalogue entry for it would be an empty heading. (#357)
    const invisible = new Set(["PortalScopeProvider"]);
    const components = Object.keys(kit).filter((name) => /^[A-Z]/.test(name) && !invisible.has(name));
    expect(components.length).toBeGreaterThan(0);
    render(<Gallery />);
    for (const name of components) {
      expect(
        screen.getByRole("heading", { name, level: 2 }),
        `${name} is exported but not in the gallery`,
      ).toBeDefined();
    }
  });

  it("writes a header fact the way the kit says to write one", () => {
    // `InspectorFact.label` is not drawn — it is the `sr-only` term — so a
    // value that wants a word on screen has to carry its own. The catalogue
    // was passing `{ label: "Ready", value: "9/12" }` and rendering
    // `CrashLoopBackOff  9/12  17  6m`: bare figures with no words at all,
    // which is the mistake the component's own doc comment warns about. The
    // gallery is the only place anyone sees what a correct call looks like.
    render(<Gallery />);
    expect(screen.getByText("9/12 ready")).toBeDefined();
    expect(screen.getByText("17 restarts")).toBeDefined();
    expect(screen.getByText("6m old")).toBeDefined();
  });

  it("shows each component's states, not just its happy path", () => {
    render(<Gallery />);
    // The states that break on a real cluster.
    expect(screen.getByRole("meter", { name: "over limit" })).toBeDefined();
    expect(screen.getByRole("img", { name: "a single sample" })).toBeDefined();
    expect(screen.getByRole("img", { name: "no samples yet" })).toBeDefined();
  });
});
