import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Gallery } from "./Gallery";
import * as kit from "../index";

describe("Gallery", () => {
  it("shows every component the kit exports", async () => {
    // Derived from the barrel rather than a hand-written list, which would
    // drift the first time someone was in a hurry. The catalogue is the only
    // visual review surface this design has.
    const components = Object.keys(kit).filter((name) => /^[A-Z]/.test(name));
    expect(components.length).toBeGreaterThan(0);
    render(<Gallery />);
    for (const name of components) {
      expect(
        screen.getByRole("heading", { name, level: 2 }),
        `${name} is exported but not in the gallery`,
      ).toBeDefined();
    }
  });

  it("shows each component's states, not just its happy path", () => {
    render(<Gallery />);
    // The states that break on a real cluster.
    expect(screen.getByRole("meter", { name: "over limit" })).toBeDefined();
    expect(screen.getByRole("img", { name: "a single sample" })).toBeDefined();
    expect(screen.getByRole("img", { name: "no samples yet" })).toBeDefined();
  });
});
