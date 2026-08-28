import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { SurfaceToast } from "./SurfaceToast";

const frame = (container: HTMLElement) => container.firstElementChild as HTMLElement | null;

/**
 * This is placement and nothing else: where a toast sits, given that it is
 * already on screen. The mock's version read a surface off React context and
 * portalled into whatever host it found; neither of those crosses into the kit,
 * so the choice is a prop and the toast stays where it is mounted. (#320)
 */
describe("SurfaceToast", () => {
  it("draws the toast it is handed", () => {
    render(<SurfaceToast title="Scaled to 3" hint="deploy/api in prod" />);
    expect(screen.getByRole("status")).toBeDefined();
    expect(screen.getByText("Scaled to 3")).toBeDefined();
    expect(screen.getByText("deploy/api in prod")).toBeDefined();
  });

  it("passes the tone through, live region and all", () => {
    render(<SurfaceToast title="Scale failed" tone="sev" />);
    expect(screen.getByRole("alert")).toBeDefined();
  });

  it("passes the dismiss through", () => {
    const onClose = vi.fn();
    render(<SurfaceToast title="Scaled to 3" onClose={onClose} />);
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onClose).toHaveBeenCalled();
  });

  it("pins to the corner of its surface by default", () => {
    const { container } = render(<SurfaceToast title="Scaled to 3" />);
    expect(frame(container)?.className).toContain("absolute");
  });

  it("pins to the corner of the window when asked", () => {
    const { container } = render(<SurfaceToast title="Scaled to 3" anchor="window" />);
    expect(frame(container)?.className).toContain("fixed");
    expect(frame(container)?.className).not.toContain("absolute");
  });

  it("stays where it is mounted rather than portalling", () => {
    // The mock's version portalled into a host it read off context. The kit has
    // no such context, and a toast that jumps out of its subtree takes its
    // theme and its stacking context with it.
    const { container } = render(<SurfaceToast title="Scaled to 3" />);
    expect(container.contains(screen.getByRole("status"))).toBe(true);
    expect(frame(container)?.contains(screen.getByRole("status"))).toBe(true);
  });

  it("renders no frame when the toast has nothing to say", () => {
    // An empty positioned box in the corner still takes clicks off whatever is
    // under it.
    const { container, rerender } = render(<SurfaceToast title="Scaled to 3" />);
    expect(frame(container)).not.toBeNull();
    rerender(<SurfaceToast title="" />);
    expect(container.firstElementChild).toBeNull();
  });

  it("forwards className onto the frame, not onto the toast", () => {
    const { container } = render(<SurfaceToast title="Scaled to 3" className="extra" />);
    expect(frame(container)?.classList.contains("extra")).toBe(true);
    expect(screen.getByRole("status").classList.contains("extra")).toBe(false);
  });
});
