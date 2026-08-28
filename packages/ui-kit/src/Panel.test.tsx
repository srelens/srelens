import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Panel } from "./Panel";

/** The classic component's tests, carried over. (#318) */
describe("Panel", () => {
  it("renders the title and children", () => {
    render(<Panel title="Cluster">body content</Panel>);
    expect(screen.getByText("Cluster")).toBeDefined();
    expect(screen.getByText("body content")).toBeDefined();
  });

  it("omits the title when none is given", () => {
    render(<Panel>only body</Panel>);
    expect(screen.queryByText("Cluster")).toBeNull();
    expect(screen.getByText("only body")).toBeDefined();
  });

  it("omits the header entirely, not just its text", () => {
    // An empty ruled header is a visible artefact, not a no-op.
    const { container } = render(<Panel>only body</Panel>);
    expect(container.querySelector(".card-head")).toBeNull();
  });

  it("forwards className onto the card", () => {
    const { container } = render(<Panel className="extra">x</Panel>);
    expect(container.querySelector(".card.extra")).not.toBeNull();
  });
});
