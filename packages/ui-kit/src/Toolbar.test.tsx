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
});
