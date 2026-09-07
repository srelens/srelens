import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MetricTile } from "./MetricTile";

/** The classic component's tests, carried over and re-toned. (#318) */
describe("MetricTile", () => {
  it("renders the label and the value", () => {
    render(<MetricTile label="Pods" value={248} />);
    expect(screen.getByText("Pods")).toBeDefined();
    expect(screen.getByText("248")).toBeDefined();
  });

  it("renders the description when given", () => {
    render(<MetricTile label="Pods" value={248} description="12 pending" />);
    expect(screen.getByText("12 pending")).toBeDefined();
  });

  it("omits the description element, not just its text", () => {
    // An empty paragraph still takes its line box, so a tile without a
    // description would sit taller than its neighbours in the same row.
    const { container } = render(<MetricTile label="Pods" value={248} />);
    expect(container.querySelector("p")).toBeNull();
  });

  it("renders the action beside the figure", () => {
    const { container } = render(
      <MetricTile label="Pods" value={248} action={<button>Refresh</button>} />,
    );
    expect(screen.getByRole("button", { name: "Refresh" })).toBeDefined();
    expect(container.querySelectorAll("article > *").length).toBe(2);
  });

  it("omits the action's slot entirely when there is none", () => {
    // A bare flex child with nothing in it still consumes the gap.
    const { container } = render(<MetricTile label="Pods" value={248} />);
    expect(container.querySelectorAll("article > *").length).toBe(1);
  });

  it("tints the tile with its tone", () => {
    const { container } = render(<MetricTile label="Restarts" value={9} tone="sev" />);
    const tile = container.querySelector<HTMLElement>(".stat");
    expect(tile?.getAttribute("data-tone")).toBe("sev");
    expect(tile?.style.background).toContain("--sev-wash");
    expect(container.querySelector<HTMLElement>(".eyebrow")?.style.color).toContain("--sev");
  });

  it("keeps the figure itself in the body colour, whatever the tone", () => {
    // The number is the thing being read; the tone is context around it.
    const { container } = render(<MetricTile label="Restarts" value={9} tone="sev" />);
    expect(container.querySelector<HTMLElement>(".stat-value")?.style.color).toBe("");
  });

  it("defaults to the muted tone, which lays no tint over the surface", () => {
    const { container } = render(<MetricTile label="Nodes" value={12} />);
    const tile = container.querySelector<HTMLElement>(".stat");
    expect(tile?.getAttribute("data-tone")).toBe("muted");
    expect(tile?.style.background).toBe("transparent");
  });

  it("forwards className onto the tile", () => {
    const { container } = render(<MetricTile label="Nodes" value={12} className="extra" />);
    expect(container.querySelector(".stat.extra")).not.toBeNull();
  });
  it("treats conditional slots that resolved to false as absent", () => {
    // (#325 review)
    const { container } = render(
      <MetricTile label="Pods" value={248} description={false} action={false} />,
    );
    expect(container.querySelector("p")).toBeNull();
    expect(container.querySelectorAll("article > *").length).toBe(1);
  });
});
