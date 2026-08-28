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

  it("renders the description under the title, inside the header", () => {
    const { container } = render(
      <Panel title="Cluster" description="Every node in the current context">
        body content
      </Panel>,
    );
    const head = container.querySelector(".card-head");
    expect(head?.textContent).toContain("Cluster");
    expect(head?.textContent).toContain("Every node in the current context");
  });

  it("still shows the header for a description with no title", () => {
    // The header is omitted only when there is nothing to put in it; a
    // description alone is something. (#318)
    const { container } = render(<Panel description="No title, still a header">only body</Panel>);
    expect(container.querySelector(".card-head")).not.toBeNull();
    expect(screen.getByText("No title, still a header")).toBeDefined();
  });

  it("forwards className onto the card", () => {
    const { container } = render(<Panel className="extra">x</Panel>);
    expect(container.querySelector(".card.extra")).not.toBeNull();
  });
  it("omits the header when both slots resolved to false", () => {
    // (#325 review)
    const { container } = render(
      <Panel title={false} description={false}>
        only body
      </Panel>,
    );
    expect(container.querySelector(".card-head")).toBeNull();
  });
  it("renders the title as a section heading", () => {
    // The classic SectionPanel this replaces rendered its title as an h2, and
    // its call sites are real sections of a page — ClusterOverview's among
    // them. A div would drop every one of them out of the document outline,
    // which is the same loss Screen's h1 exists to avoid. No level prop: no
    // SectionPanel in the app nests inside another. (#325 review)
    render(<Panel title="Cluster">body</Panel>);
    expect(screen.getByRole("heading", { level: 2, name: "Cluster" })).toBeDefined();
  });
});
