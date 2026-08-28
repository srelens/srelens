import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Spinner } from "./Spinner";

/** The classic component's tests, carried over. (#318) */
describe("Spinner", () => {
  it("exposes an accessible status role with a default label", () => {
    render(<Spinner />);
    expect(screen.getByRole("status").getAttribute("aria-label")).toBe("Loading");
  });

  it("uses a custom label when provided", () => {
    render(<Spinner label="Fetching pods" />);
    expect(screen.getByLabelText("Fetching pods")).toBeDefined();
  });

  it("renders an animated svg ring with a muted track circle", () => {
    const { container } = render(<Spinner />);
    const svg = container.querySelector("svg");
    expect(svg?.classList.contains("animate-spin")).toBe(true);
    expect(container.querySelector("circle")).not.toBeNull();
  });

  it("forwards extra classes onto the svg", () => {
    // `text-primary` in the classic test was a classic token with no equivalent
    // here; the assertion is about forwarding, so the class is just a carrier.
    const { container } = render(<Spinner className="size-8 text-muted" />);
    const svg = container.querySelector("svg");
    expect(svg?.classList.contains("size-8")).toBe(true);
    expect(svg?.classList.contains("text-muted")).toBe(true);
  });

  it("draws itself in the inherited colour, never a named one", () => {
    // Why it needed no restyling: nothing here names a colour, so it already
    // followed whatever theme it landed in.
    const { container } = render(<Spinner />);
    const strokes = [...container.querySelectorAll("[stroke]")].map((n) => n.getAttribute("stroke"));
    expect(strokes.length).toBeGreaterThan(0);
    expect(strokes.every((s) => s === "currentColor")).toBe(true);
  });
});
