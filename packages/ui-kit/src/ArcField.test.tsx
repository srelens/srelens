import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { ArcField } from "./ArcField";

/**
 * Nothing here is interactive, so the whole of the test is the one thing an SVG
 * background can get wrong: the document-wide id it hangs its pattern on.
 * (#320)
 */
describe("ArcField", () => {
  const patternId = (svg: Element) => svg.querySelector("pattern")?.getAttribute("id") ?? "";

  it("draws the grid and the arcs", () => {
    const { container } = render(<ArcField />);
    expect(container.querySelector("pattern")).not.toBeNull();
    expect(container.querySelectorAll("circle").length).toBeGreaterThan(0);
  });

  it("gives each field its own pattern id", () => {
    // Two of these on one page — a screen and a dialog behind it — and a fixed
    // id makes the second one's rect point at the first one's pattern.
    const { container } = render(
      <>
        <ArcField />
        <ArcField />
      </>,
    );
    const [first, second] = Array.from(container.querySelectorAll("svg"));
    expect(patternId(first)).not.toBe("");
    expect(patternId(first)).not.toBe(patternId(second));
  });

  it("fills each rect from its own pattern", () => {
    const { container } = render(
      <>
        <ArcField />
        <ArcField />
      </>,
    );
    const fields = Array.from(container.querySelectorAll("svg"));
    expect(fields).toHaveLength(2);
    for (const svg of fields) {
      expect(svg.querySelector("rect")?.getAttribute("fill")).toBe(`url(#${patternId(svg)})`);
    }
  });

  it("uses an id a selector can be built from", () => {
    // React's own ids carry punctuation, and `#:r0:` is not a selector — the
    // browser resolves `url(#…)` leniently, a stylesheet or a query does not.
    const { container } = render(<ArcField />);
    const id = patternId(container.querySelector("svg") as Element);
    expect(id).toMatch(/^[A-Za-z][A-Za-z0-9_-]*$/);
    expect(() => container.querySelector(`#${id}`)).not.toThrow();
  });

  it("is hidden from assistive technology", () => {
    const { container } = render(<ArcField />);
    expect(container.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("takes no pointer events, so it cannot swallow a click", () => {
    const { container } = render(<ArcField />);
    expect(container.querySelector("svg")?.getAttribute("class")).toContain("pointer-events-none");
  });

  it("forwards className, so it can be scoped to a panel", () => {
    const { container } = render(<ArcField className="absolute" />);
    expect(container.querySelector("svg.absolute")).not.toBeNull();
  });
});
