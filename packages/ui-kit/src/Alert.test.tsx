import { describe, it, expect, vi } from "vitest";
import type { FormEvent } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { Alert } from "./Alert";

const glyph = (container: HTMLElement) =>
  container.querySelector("[data-glyph]")?.getAttribute("data-glyph");

describe("Alert", () => {
  it("renders its title and its body", () => {
    render(<Alert title="Quota exceeded">The namespace is at its memory limit.</Alert>);
    expect(screen.getByText("Quota exceeded")).toBeDefined();
    expect(screen.getByText("The namespace is at its memory limit.")).toBeDefined();
  });

  it("interrupts for a severe tone", () => {
    // `alert` is assertive: it cuts across whatever is being read. A failure
    // that arrived unbidden has earned that; nothing quieter has.
    render(<Alert tone="sev" title="Delete failed" />);
    expect(screen.getByRole("alert").textContent).toContain("Delete failed");
  });

  it("waits its turn for anything less severe", () => {
    for (const tone of ["warn", "info", "ok", "accent", "muted"] as const) {
      const { unmount } = render(<Alert tone={tone} title={`${tone} says hello`} />);
      expect(screen.getByRole("status").textContent).toContain("says hello");
      expect(screen.queryByRole("alert")).toBeNull();
      unmount();
    }
  });

  it("defaults to the polite role, as an unclassified message is not an emergency", () => {
    render(<Alert title="Rollout finished" />);
    expect(screen.getByRole("status")).toBeDefined();
  });

  it("fires onDismiss when its dismiss button is pressed", () => {
    const onDismiss = vi.fn();
    render(<Alert tone="warn" title="Node is cordoned" onDismiss={onDismiss} />);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("does not submit a form it is standing in", () => {
    // `Button` deliberately leaves `type` alone, so a bare button in a form is
    // a submit button (bd24d1a). This one is the component's own, not the
    // caller's: dismissing a warning above a form would otherwise submit it.
    // (#320)
    const onSubmit = vi.fn((e: FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Alert tone="warn" title="Check the namespace" onDismiss={() => {}} />
      </form>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("takes the dismiss button's name from the caller when several are on screen", () => {
    render(
      <Alert title="Node is cordoned" onDismiss={() => {}} dismissLabel="Dismiss node warning" />,
    );
    expect(screen.getByRole("button", { name: "Dismiss node warning" })).toBeDefined();
  });

  it("omits the dismiss button when there is no dismissing to do", () => {
    render(<Alert title="Rollout finished" />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("omits the body element, not just its text", () => {
    // The body carries its own top margin and line box; an empty one makes a
    // title-only alert sit taller than it should.
    const { container } = render(<Alert title="Rollout finished" />);
    expect(container.querySelector('[data-slot="alert-body"]')).toBeNull();
  });

  it("treats a conditional body that resolved to false as absent", () => {
    const { container } = render(<Alert title="Rollout finished">{false}</Alert>);
    expect(container.querySelector('[data-slot="alert-body"]')).toBeNull();
  });

  it("draws its glyph itself rather than importing an icon set", () => {
    // The kit depends on no icon set. The mock imported AlertTriangle, Info and
    // X from lucide. (#320)
    const { container } = render(<Alert tone="sev" title="Delete failed" />);
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("hides the glyph from assistive technology, since the tone is already in the role", () => {
    const { container } = render(<Alert tone="sev" title="Delete failed" onDismiss={() => {}} />);
    for (const svg of container.querySelectorAll("svg")) {
      expect(svg.getAttribute("aria-hidden")).toBe("true");
    }
  });

  it("warns with a warning glyph and informs with an informing one", () => {
    expect(glyph(render(<Alert tone="warn" title="a" />).container)).toBe("warning");
    expect(glyph(render(<Alert tone="sev" title="b" />).container)).toBe("warning");
    expect(glyph(render(<Alert tone="info" title="c" />).container)).toBe("info");
  });

  it("takes its colour from the tone", () => {
    const { container } = render(<Alert tone="sev" title="Delete failed" />);
    const root = container.firstElementChild as HTMLElement;
    // The same wash Badge uses, so an alert and a badge of one severity tint
    // identically. jsdom drops the color-mix the border is drawn with, so the
    // wash and the tone attribute are what can be asserted here.
    expect(root.style.background).toContain("--sev-wash");
    expect(root.getAttribute("data-tone")).toBe("sev");
  });

  it("forwards className onto the root without replacing its own", () => {
    const { container } = render(<Alert title="x" className="extra" />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.classList.contains("extra")).toBe(true);
    expect(root.className.trim()).not.toBe("extra");
  });
});
