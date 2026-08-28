import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { Sparkline } from "./Sparkline";

const pathsOf = (c: HTMLElement) => Array.from(c.querySelectorAll("path")).map((p) => p.getAttribute("d") ?? "");

describe("Sparkline", () => {
  it("draws a line through its samples, and an area under it", () => {
    const { container } = render(<Sparkline points={[1, 5, 2]} />);
    const [area, line] = pathsOf(container);
    expect(line).toMatch(/^M0,/);
    expect(area).toContain("Z");
  });

  it("omits the fill when asked", () => {
    const { container } = render(<Sparkline points={[1, 5, 2]} fill={false} />);
    expect(pathsOf(container)).toHaveLength(1);
  });

  it("never puts NaN in the path for a single sample", () => {
    // The version this came from divides by `points.length - 1`, so one sample
    // produces "MNaN,NaN" — which renders as nothing, with no error anywhere.
    const { container } = render(<Sparkline points={[7]} />);
    for (const d of pathsOf(container)) {
      expect(d, `path contains NaN: ${d}`).not.toContain("NaN");
    }
  });

  it("draws a lone sample as a flat line rather than a dot at the origin", () => {
    const { container } = render(<Sparkline points={[7]} fill={false} />);
    expect(pathsOf(container)[0]).toMatch(/^M0,[\d.]+ L100,[\d.]+$/);
  });

  it("renders an empty box for a series with no samples yet", () => {
    // The normal state of a chart that has just been opened, not an edge case.
    const { container } = render(<Sparkline points={[]} />);
    expect(pathsOf(container)).toHaveLength(0);
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("keeps its height so a column of sparklines stays aligned", () => {
    const { container } = render(<Sparkline points={[]} height={40} />);
    expect(container.querySelector("svg")?.style.height).toBe("40px");
  });

  it("is presentational unless given a label", () => {
    const { container: bare } = render(<Sparkline points={[1, 2]} />);
    expect(bare.querySelector("svg")?.getAttribute("role")).toBe("presentation");
    const { container: labelled } = render(<Sparkline points={[1, 2]} ariaLabel="CPU" />);
    expect(labelled.querySelector("svg")?.getAttribute("role")).toBe("img");
  });

  it("anchors to zero rather than to the samples' own range", () => {
    // Deliberate, and inherited from the mock: [90, 95] reads as high and
    // steady rather than as a climb, and two sparklines side by side share a
    // baseline. Asserted so a later reader does not "fix" it into range
    // scaling, which would change every chart in the product. (#317 review)
    const { container } = render(<Sparkline points={[90, 95]} fill={false} />);
    const d = container.querySelector("path")?.getAttribute("d") ?? "";
    const ys = [...d.matchAll(/[ML][\d.]+,([\d.]+)/g)].map((m) => Number(m[1]));
    // Both samples sit in the top third of a 34px box, close together.
    expect(Math.abs(ys[0] - ys[1])).toBeLessThan(3);
    expect(Math.max(...ys)).toBeLessThan(34 / 3);
  });
});
