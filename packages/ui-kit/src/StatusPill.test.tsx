import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusPill } from "./StatusPill";
import { toneColor } from "./tone";

describe("StatusPill", () => {
  it("renders the status label with a coloured dot", () => {
    const { container } = render(<StatusPill status="Running" kind="success" />);
    expect(screen.getByText("Running")).toBeDefined();
    const dot = container.querySelector(".dot") as HTMLElement;
    expect(dot).not.toBeNull();
    expect(dot.style.background).toContain("--ok");
  });

  it("defaults to a neutral dot", () => {
    const { container } = render(<StatusPill status="Unknown" />);
    expect(screen.getByText("Unknown")).toBeDefined();
    expect((container.querySelector(".dot") as HTMLElement).style.background).toContain("--ink-muted");
  });

  it("resolves every kind to a token, never a palette colour", () => {
    // The classic component wrote `bg-emerald-500`, `bg-amber-500` and
    // `bg-sky-500` — raw palette values that do not follow the theme, and whose
    // failure is invisible until someone switches to light.
    const kinds = ["success", "warning", "danger", "info", "neutral"] as const;
    for (const kind of kinds) {
      const { container, unmount } = render(<StatusPill status="x" kind={kind} />);
      const background = (container.querySelector(".dot") as HTMLElement).style.background;
      expect(background, `${kind} should resolve to a token`).toContain("var(--");
      unmount();
    }
  });

  it("never tells the status in colour alone", () => {
    // A dot with no words is unreadable to a colour-blind user and silent to a
    // screen reader.
    render(<StatusPill status="CrashLoopBackOff" kind="danger" />);
    expect(screen.getByText("CrashLoopBackOff")).toBeDefined();
  });

  it("maps success to the ok tone", () => {
    const { container } = render(<StatusPill status="x" kind="success" />);
    expect((container.querySelector(".dot") as HTMLElement).style.background).toBe(toneColor("ok"));
  });
});
