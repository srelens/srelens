import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AgentMark } from "./AgentMark";

/**
 * The mark is a brand glyph, so what there is to test is the arithmetic that
 * sizes it and the question of whether it says anything to a screen reader.
 * (#320)
 */
describe("AgentMark", () => {
  const mark = (container: HTMLElement) => container.querySelector(".agent-mark") as HTMLElement;

  it("draws itself at the size it is given", () => {
    const { container } = render(<AgentMark size={24} />);
    expect(mark(container).style.width).toBe("24px");
    expect(mark(container).style.height).toBe("24px");
  });

  it("scales the glyph with the mark", () => {
    const { container } = render(<AgentMark size={40} />);
    const glyph = container.querySelector("svg") as SVGElement;
    // 0.58 of the box, the proportion the mock drew it at.
    expect(glyph.getAttribute("width")).toBe("23");
  });

  it("refuses a size that would collapse it", () => {
    // A count of zero, a bad measurement, a negative from arithmetic upstream:
    // the mock passed all of them to `width` and drew nothing at all.
    const { container } = render(<AgentMark size={-8} />);
    expect(Number.parseInt(mark(container).style.width, 10)).toBeGreaterThanOrEqual(12);
  });

  it("refuses a size that would swallow the page", () => {
    const { container } = render(<AgentMark size={4000} />);
    expect(Number.parseInt(mark(container).style.width, 10)).toBeLessThanOrEqual(64);
  });

  it("rounds a fractional size rather than passing it through", () => {
    const { container } = render(<AgentMark size={19.4} />);
    expect(mark(container).style.width).toBe("19px");
  });

  it("softens its corners on the larger sizes", () => {
    const { container: small } = render(<AgentMark size={19} />);
    const { container: large } = render(<AgentMark size={28} />);
    expect(mark(small).style.borderRadius).toBe("5px");
    expect(mark(large).style.borderRadius).toBe("6px");
  });

  it("says nothing when it is standing next to the word it illustrates", () => {
    // The ordinary case: a mark beside "Agent". Announced, it would be read
    // twice.
    const { container } = render(<AgentMark />);
    expect(mark(container).getAttribute("aria-hidden")).toBe("true");
  });

  it("names itself when it is standing alone", () => {
    render(<AgentMark label="srelens agent" />);
    expect(screen.getByRole("img", { name: "srelens agent" })).toBeDefined();
  });

  it("stops hiding itself once it has a name", () => {
    const { container } = render(<AgentMark label="srelens agent" />);
    expect(mark(container).getAttribute("aria-hidden")).toBeNull();
  });

  it("keeps the glyph out of the name it was given", () => {
    // Two names on one element is one too many.
    const { container } = render(<AgentMark label="srelens agent" />);
    expect(container.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("forwards className onto the mark", () => {
    const { container } = render(<AgentMark className="extra" />);
    expect(container.querySelector(".agent-mark.extra")).not.toBeNull();
  });
});
