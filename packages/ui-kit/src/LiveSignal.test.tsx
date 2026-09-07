import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { LiveSignal } from "./LiveSignal";

/**
 * The dot is the whole component, so the tests are about what the dot means
 * and whether anything but a sighted reader can find it out. (#320)
 */
describe("LiveSignal", () => {
  const dot = (container: HTMLElement) => container.querySelector(".live-dot") as HTMLElement;

  it("shows the label it is given", () => {
    render(<LiveSignal label="Streaming events" />);
    expect(screen.getByText("Streaming events")).toBeDefined();
  });

  it("stands for a live stream by default", () => {
    render(<LiveSignal />);
    expect(screen.getByText("Live signal")).toBeDefined();
  });

  it("falls back to a label when the slot resolved to nothing", () => {
    // `label={connected && "Streaming"}` is the ordinary way to write this, and
    // a bare coloured dot means nothing on its own.
    render(<LiveSignal label={false} />);
    expect(screen.getByText("Live signal")).toBeDefined();
  });

  it("falls back to a label for an empty string too", () => {
    render(<LiveSignal label="" />);
    expect(screen.getByText("Live signal")).toBeDefined();
  });

  it("reports itself as a status, so a change to it is announced", () => {
    render(<LiveSignal label="Streaming events" />);
    expect(screen.getByRole("status").textContent).toBe("Streaming events");
  });

  it("tints the dot with the tone", () => {
    const { container } = render(<LiveSignal tone="ok" />);
    expect(dot(container).style.background).toContain("var(--ok)");
  });

  it("takes severity as its resting tone", () => {
    const { container } = render(<LiveSignal />);
    expect(dot(container).style.background).toContain("var(--sev)");
  });

  it("keeps the dot out of the announcement", () => {
    // It carries no information the label does not; read out, it is noise.
    const { container } = render(<LiveSignal label="Streaming events" />);
    expect(dot(container).getAttribute("aria-hidden")).toBe("true");
  });

  it("keeps the pulse class the stylesheet animates", () => {
    const { container } = render(<LiveSignal />);
    expect(dot(container)).not.toBeNull();
  });

  it("marks the tone on the dot for a stylesheet to reach", () => {
    const { container } = render(<LiveSignal tone="warn" />);
    expect(dot(container).getAttribute("data-tone")).toBe("warn");
  });

  it("forwards className onto the row", () => {
    const { container } = render(<LiveSignal className="extra" />);
    expect(container.querySelector(".extra")).not.toBeNull();
  });
});
