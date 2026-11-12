import { describe, it, expect, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
  it("is named by its label, so a row of tiles can be told apart", () => {
    render(<MetricTile label="Pods" value={248} />);
    expect(screen.getByRole("article", { name: "Pods" })).toBeDefined();
  });

  it("opens what it counts from its label, when there is somewhere to go", async () => {
    // (#1001) An app's dashboard card opens the list behind its figure.
    const onOpen = vi.fn();
    render(<MetricTile label="Expiring" value={3} onOpen={onOpen} />);
    await userEvent.click(screen.getByRole("button", { name: "Open Expiring" }));
    expect(onOpen).toHaveBeenCalledOnce();
    // The tile keeps its own name; "Open" belongs to the button.
    expect(screen.getByRole("article", { name: "Expiring" })).toBeDefined();
  });

  it("keeps the label's case on the open button, as on every other tile's label", () => {
    // A button resets text-transform, so "PODS" beside it read "Expiring". Seen live.
    render(<MetricTile label="Expiring" value={3} onOpen={() => {}} />);
    expect(screen.getByRole("button", { name: "Open Expiring" }).className).toContain("[text-transform:inherit]");
  });

  it("draws its label as plain text when there is nowhere to go", () => {
    render(<MetricTile label="Expiring" value={3} />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("draws more under the figure, above the description", () => {
    // A breakdown by status, or the rows behind a count.
    render(
      <MetricTile label="Certificates" value={12} description="cert-manager">
        <ul aria-label="By status"><li>Ready 10</li></ul>
      </MetricTile>,
    );
    const tile = screen.getByRole("article", { name: "Certificates" });
    const list = within(tile).getByRole("list", { name: "By status" });
    const description = within(tile).getByText("cert-manager");
    expect(list.compareDocumentPosition(description) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("draws no figure while there is none, rather than an empty one", () => {
    // Loading, or a read that failed: a blank or a zero there would read as an answer.
    const { container } = render(<MetricTile label="Expiring">Loading…</MetricTile>);
    expect(container.querySelector(".stat-value")).toBeNull();
  });

  it("wraps a long label or description inside the tile rather than running out of it", () => {
    // An app names its own cards, and one unbroken word is enough to overflow.
    const { container } = render(
      <MetricTile label="Certificatesexpiringsoonacrossclusters" value={1} description="org.example.certificates" />,
    );
    expect(container.querySelector(".eyebrow")?.className).toContain("[overflow-wrap:anywhere]");
    expect(container.querySelector("p")?.className).toContain("[overflow-wrap:anywhere]");
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
