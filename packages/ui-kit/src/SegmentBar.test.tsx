import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { SegmentBar } from "./SegmentBar";

const runs = [
  { value: 18, tone: "ok" as const, label: "Running" },
  { value: 3, tone: "warn" as const, label: "Pending" },
  { value: 1, tone: "sev" as const, label: "Failed" },
];

/** The bar's own children, in document order. */
function widths(bar: HTMLElement): string[] {
  return Array.from(bar.children).map((child) => (child as HTMLElement).style.width);
}

describe("SegmentBar", () => {
  it("renders the segments in order, sized in proportion to their values", () => {
    const bar = render(<SegmentBar segments={runs} ariaLabel="Pod distribution" />).container
      .firstElementChild as HTMLElement;
    expect(bar.children).toHaveLength(3);
    expect(widths(bar)).toEqual(["81.81818181818183%", "13.636363636363635%", "4.545454545454546%"]);
  });

  it("labels each run with its name and count", () => {
    const bar = render(<SegmentBar segments={runs} ariaLabel="Pod distribution" />).container
      .firstElementChild as HTMLElement;
    expect(Array.from(bar.children).map((c) => c.getAttribute("title"))).toEqual([
      "Running: 18",
      "Pending: 3",
      "Failed: 1",
    ]);
  });

  it("carries each segment's tone colour", () => {
    const bar = render(<SegmentBar segments={runs} ariaLabel="Pod distribution" />).container
      .firstElementChild as HTMLElement;
    const backgrounds = Array.from(bar.children).map((c) => (c as HTMLElement).style.background);
    expect(backgrounds[0]).toContain("--ok");
    expect(backgrounds[1]).toContain("--warn");
    expect(backgrounds[2]).toContain("--sev");
  });

  it("is named by the caller, not by its own shape", () => {
    // The classic version hardcoded "Segmented status bar" — a name that
    // describes the widget rather than what it counts, and that is identical
    // for every bar on the page. (#318)
    render(<SegmentBar segments={runs} ariaLabel="Pod distribution" />);
    expect(screen.getByRole("img", { name: "Pod distribution" })).toBeDefined();
    expect(screen.queryByLabelText("Segmented status bar")).toBeNull();
  });

  it("does not divide by zero when every count is zero", () => {
    // A cluster with nothing scheduled yet is the normal first render, not an
    // edge case, and a NaN width silently collapses the run to nothing.
    const bar = render(
      <SegmentBar
        segments={[
          { value: 0, tone: "ok", label: "Running" },
          { value: 0, tone: "sev", label: "Failed" },
        ]}
        ariaLabel="Pods"
      />,
    ).container.firstElementChild as HTMLElement;
    expect(widths(bar)).toEqual(["0%", "0%"]);
    expect(bar.innerHTML).not.toContain("NaN");
  });

  it("renders an empty bar rather than crashing on no segments at all", () => {
    const bar = render(<SegmentBar segments={[]} ariaLabel="Pods" />).container
      .firstElementChild as HTMLElement;
    expect(bar).not.toBeNull();
    expect(bar.children).toHaveLength(0);
  });

  it("never gives a segment a negative width", () => {
    // The classic version clamped negatives when summing the total but then
    // took each width from the raw value, so a negative count drew backwards
    // against a total that never counted it. Clamped in both places now. (#318)
    const bar = render(
      <SegmentBar
        segments={[
          { value: -5, tone: "sev", label: "Failed" },
          { value: 5, tone: "ok", label: "Running" },
        ]}
        ariaLabel="Pods"
      />,
    ).container.firstElementChild as HTMLElement;
    expect(widths(bar)).toEqual(["0%", "100%"]);
  });

  it("still reports the real count when a value is negative", () => {
    // Only the geometry is clamped; hiding the figure would hide the fault.
    const bar = render(
      <SegmentBar segments={[{ value: -5, tone: "sev", label: "Failed" }]} ariaLabel="Pods" />,
    ).container.firstElementChild as HTMLElement;
    expect(bar.children[0].getAttribute("title")).toBe("Failed: -5");
  });

  it("forwards className onto the bar", () => {
    const { container } = render(
      <SegmentBar segments={runs} ariaLabel="Pods" className="extra" />,
    );
    expect(container.querySelector(".extra")).toBe(container.firstElementChild);
  });
});
