import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Eyebrow } from "./Eyebrow";

/** New: the mock shipped this component with no tests at all. (#320) */
describe("Eyebrow", () => {
  it("renders its content", () => {
    render(<Eyebrow>since</Eyebrow>);
    expect(screen.getByText("since")).toBeDefined();
  });

  it("wears the label voice", () => {
    const { container } = render(<Eyebrow>container</Eyebrow>);
    expect(container.querySelector(".eyebrow")).not.toBeNull();
  });

  it("takes no colour of its own by default", () => {
    // The class already says --ink-muted. An inline colour written on every
    // eyebrow would win over any theme rule that ever tries to change it, so
    // the untinted state carries no style attribute at all.
    const { container } = render(<Eyebrow>since</Eyebrow>);
    expect(container.querySelector(".eyebrow")?.getAttribute("style")).toBeNull();
  });

  it("colours itself from the tone's token when given one", () => {
    const { container } = render(<Eyebrow tone="warn">degraded</Eyebrow>);
    const el = container.querySelector<HTMLElement>(".eyebrow");
    expect(el?.style.color).toContain("--warn");
    expect(el?.getAttribute("data-tone")).toBe("warn");
  });

  it("says nothing about tone when it has none", () => {
    // An untoned eyebrow is the common case; a data-tone of "" or "undefined"
    // would make `[data-tone]` match every one of them.
    const { container } = render(<Eyebrow>since</Eyebrow>);
    expect(container.querySelector(".eyebrow")?.hasAttribute("data-tone")).toBe(false);
  });

  it("forwards className", () => {
    const { container } = render(<Eyebrow className="mb-1.5">or</Eyebrow>);
    expect(container.querySelector(".eyebrow.mb-1\\.5")).not.toBeNull();
  });
});
