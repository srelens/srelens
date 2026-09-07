import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Toast } from "./Toast";

const shell = (container: HTMLElement) => container.firstElementChild as HTMLElement | null;
const glyph = () => document.querySelector("[data-glyph]")?.getAttribute("data-glyph");

/**
 * A toast is presentation here: it is handed a message and draws it. Nothing
 * below asks it to queue, time out, portal or appear on its own, because it
 * does none of those — the app's sink does. What is tested is the drawing, the
 * live region it lands in, and the dismiss control. (#320)
 */
describe("Toast", () => {
  it("renders the title and the hint", () => {
    render(<Toast title="Scaled to 3" hint="deploy/api in prod" />);
    expect(screen.getByText("Scaled to 3")).toBeDefined();
    expect(screen.getByText("deploy/api in prod")).toBeDefined();
  });

  it("omits the hint line rather than leaving it empty", () => {
    const { container } = render(<Toast title="Scaled to 3" />);
    expect(screen.getByText("Scaled to 3")).toBeDefined();
    expect(container.querySelector('[data-slot="toast-hint"]')).toBeNull();
  });

  it("omits the hint a conditional resolved to false", () => {
    const { container } = render(<Toast title="Scaled to 3" hint={false} />);
    expect(screen.getByText("Scaled to 3")).toBeDefined();
    expect(container.querySelector('[data-slot="toast-hint"]')).toBeNull();
  });

  it("renders nothing at all when it has nothing to say", () => {
    // An empty toast is a floating blank card over the app, and the message is
    // built from state that can be empty on the first render.
    const { container, rerender } = render(<Toast title="Scaled to 3" />);
    expect(shell(container)).not.toBeNull();
    rerender(<Toast title="" />);
    expect(container.firstElementChild).toBeNull();
  });

  it("still draws a toast that is only a hint", () => {
    const { container } = render(<Toast title="" hint="deploy/api in prod" />);
    expect(shell(container)).not.toBeNull();
    expect(screen.getByText("deploy/api in prod")).toBeDefined();
  });

  it("waits its turn by default, and interrupts for a failure", () => {
    // The only difference that matters to someone who cannot see it appear.
    const { container, rerender } = render(<Toast title="Scaled to 3" />);
    expect(shell(container)?.getAttribute("role")).toBe("status");
    rerender(<Toast title="Scale failed" tone="sev" />);
    expect(shell(container)?.getAttribute("role")).toBe("alert");
  });

  it("does not double up the live region with an aria-live of its own", () => {
    const { container } = render(<Toast title="Scaled to 3" />);
    expect(shell(container)?.getAttribute("aria-live")).toBeNull();
  });

  it("changes the glyph with the tone, not only the colour", () => {
    // The mock drew a tick whatever the tone, so a failure was a red circle
    // with a tick in it — the shape said one thing and the colour another.
    const { rerender } = render(<Toast title="x" tone="ok" />);
    expect(glyph()).toBe("check");
    rerender(<Toast title="x" tone="sev" />);
    expect(glyph()).toBe("warning");
    rerender(<Toast title="x" tone="warn" />);
    expect(glyph()).toBe("warning");
    rerender(<Toast title="x" tone="info" />);
    expect(glyph()).toBe("info");
  });

  it("hides the glyph from assistive technology", () => {
    render(<Toast title="Scaled to 3" />);
    expect(document.querySelector("[data-glyph]")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("takes the disc's colours from tokens", () => {
    // The mock wrote `#fff` for the tick.
    const { container } = render(<Toast title="x" tone="warn" />);
    const disc = container.querySelector('[data-slot="toast-mark"]') as HTMLElement;
    expect(disc.style.background).toContain("--warn");
    expect(disc.style.color).toMatch(/^var\(--/);
  });

  it("offers no dismiss control when there is nothing to dismiss to", () => {
    const { container } = render(<Toast title="Scaled to 3" />);
    expect(shell(container)).not.toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("dismisses through a named button", () => {
    const onClose = vi.fn();
    render(<Toast title="Scaled to 3" onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("names the dismiss button apart when several toasts share a screen", () => {
    render(<Toast title="Scaled to 3" onClose={() => {}} dismissLabel="Dismiss: scaled to 3" />);
    expect(screen.getByRole("button", { name: "Dismiss: scaled to 3" })).toBeDefined();
  });

  it("says type=button on the dismiss control", () => {
    // A bare button inside a form is a submit button, and the kit's Button
    // deliberately leaves `type` alone. This one is the component's own.
    render(<Toast title="Scaled to 3" onClose={() => {}} />);
    expect(screen.getByRole("button", { name: "Dismiss" }).getAttribute("type")).toBe("button");
  });

  it("wears the design's console shell and forwards className", () => {
    const { container } = render(<Toast title="x" className="extra" />);
    expect(shell(container)?.classList.contains("console-shell")).toBe(true);
    expect(shell(container)?.classList.contains("extra")).toBe(true);
  });

  it("puts a title only on the control that can be hovered", () => {
    const { container } = render(<Toast title="Scaled to 3" hint="detail" onClose={() => {}} />);
    const titled = Array.from(container.querySelectorAll("[title]"));
    expect(titled.length).toBeGreaterThan(0);
    expect(titled.every((node) => node.tagName === "BUTTON")).toBe(true);
  });
});
