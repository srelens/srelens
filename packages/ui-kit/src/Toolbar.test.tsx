import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Toolbar } from "./Toolbar";

describe("Toolbar", () => {
  it("renders its children", () => {
    render(
      <Toolbar>
        <button>Refresh</button>
      </Toolbar>,
    );
    expect(screen.getByRole("button", { name: "Refresh" })).toBeDefined();
  });

  it("wears the design's toolbar chrome", () => {
    const { container } = render(<Toolbar>x</Toolbar>);
    expect(container.querySelector(".toolbar")).not.toBeNull();
  });

  it("forwards className onto the toolbar", () => {
    const { container } = render(<Toolbar className="extra">x</Toolbar>);
    expect(container.querySelector(".toolbar.extra")).not.toBeNull();
  });

  it("keeps its controls on one row by default", () => {
    // Screen's head is a Toolbar, and its title truncates rather than pushing
    // the actions onto a second row.
    const { container } = render(<Toolbar>x</Toolbar>);
    expect(container.querySelector(".toolbar")?.classList.contains("flex-wrap")).toBe(false);
  });

  it("wraps its controls onto further rows, with room between them, when asked", () => {
    const { container } = render(<Toolbar wrap>x</Toolbar>);
    const bar = container.querySelector(".toolbar");
    expect(bar?.classList.contains("flex-wrap")).toBe(true);
    // A second row needs a gap above it and an inset under it, or it sits on
    // the row above and on the rule below.
    expect(bar?.classList.contains("gap-y-1.5")).toBe(true);
    expect(bar?.classList.contains("py-1")).toBe(true);
  });
});
