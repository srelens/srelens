import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Stat } from "./Stat";

/** New: the mock shipped this component with no tests at all. (#320) */
describe("Stat", () => {
  it("renders the label and the value", () => {
    render(<Stat label="Nodes" value={12} />);
    expect(screen.getByText("Nodes")).toBeDefined();
    expect(screen.getByText("12")).toBeDefined();
  });

  it("says the label in the eyebrow voice", () => {
    const { container } = render(<Stat label="Nodes" value={12} />);
    expect(container.querySelector(".eyebrow")?.textContent).toBe("Nodes");
    expect(container.querySelector(".stat-value")?.textContent).toBe("12");
  });

  it("renders the delta when given", () => {
    render(<Stat label="Opened" value="17" delta="+4 vs last week" />);
    expect(screen.getByText("+4 vs last week")).toBeDefined();
  });

  it("omits the delta element, not just its text", () => {
    // An empty line under the figure still takes its line box, so a stat with
    // no delta would stand taller than its neighbours in the same divided row.
    const { container } = render(<Stat label="Nodes" value={12} />);
    expect(container.querySelector(".num")).toBeNull();
  });

  it("treats a delta that resolved to false as absent", () => {
    const { container } = render(<Stat label="Nodes" value={12} delta={false} />);
    expect(container.querySelector(".num")).toBeNull();
  });

  it("colours the delta with the tone", () => {
    const { container } = render(<Stat label="Pods" value="1 284" delta="3 not ready" tone="sev" />);
    expect(container.querySelector<HTMLElement>(".num")?.style.color).toContain("--sev");
  });

  it("keeps the figure itself in the body colour, whatever the tone", () => {
    // Same reading as MetricTile: the number is what the eye came for, and the
    // tone is the context around it.
    const { container } = render(<Stat label="Pods" value="1 284" delta="3 not ready" tone="sev" />);
    expect(container.querySelector<HTMLElement>(".stat-value")?.style.color).toBe("");
    expect(container.querySelector<HTMLElement>(".eyebrow")?.getAttribute("style")).toBeNull();
  });

  it("defaults to the muted tone", () => {
    const { container } = render(<Stat label="Changes run" value="9" delta="all confirmed" />);
    expect(container.querySelector(".stat")?.getAttribute("data-tone")).toBe("muted");
    expect(container.querySelector<HTMLElement>(".num")?.style.color).toContain("--ink-muted");
  });

  it("puts the tone on the stat itself, delta or no delta", () => {
    // Otherwise a stat with nothing under the figure carries no trace of the
    // tone it was given, and neither a stylesheet nor a test can see it.
    const { container } = render(<Stat label="Age" value="84d" tone="ok" />);
    expect(container.querySelector(".stat")?.getAttribute("data-tone")).toBe("ok");
  });

  it("leaves its width to the row it sits in", () => {
    // The mock baked `flex-1` in. Two utilities that both set `flex` are
    // decided by stylesheet order, not attribute order, so a baked one cannot
    // be overridden through className — the caller sizes the row instead.
    const { container } = render(<Stat label="Nodes" value={12} />);
    expect(container.querySelector(".stat")?.className).not.toContain("flex-1");
  });

  it("forwards className onto the stat", () => {
    const { container } = render(<Stat label="Nodes" value={12} className="extra" />);
    expect(container.querySelector(".stat.extra")).not.toBeNull();
  });
});
