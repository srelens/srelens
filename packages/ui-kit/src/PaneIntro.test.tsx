import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { PaneIntro } from "./PaneIntro";

describe("PaneIntro", () => {
  it("names the pane with a section heading", () => {
    render(<PaneIntro title="Skills" />);
    expect(screen.getByRole("heading", { level: 2, name: "Skills" })).toBeDefined();
  });

  it("says what the pane holds under its name", () => {
    render(<PaneIntro title="Workspaces" description="Each workspace has its own clusters and tabs." />);
    const line = screen.getByText("Each workspace has its own clusters and tabs.");
    expect(line.tagName).toBe("P");
    // A Panel head's voice: the muted caption size under a 13px title.
    expect(line.className).toContain("text-[0.75rem]");
    expect(line.className).toContain("text-muted");
  });

  it("draws no line when there is nothing to say", () => {
    const { container } = render(<PaneIntro title="Updates" description={false} />);
    expect(container.querySelector("p")).toBeNull();
  });

  it("puts the pane-wide actions beside its name", () => {
    render(<PaneIntro title="Apps" actions={<button type="button">Refresh</button>} />);
    expect(screen.getByRole("button", { name: "Refresh" })).toBeDefined();
  });

  it("is ruled off from the pane under it, with a Panel head's inset", () => {
    const { container } = render(<PaneIntro title="Contexts" />);
    const intro = container.firstElementChild as HTMLElement;
    expect(intro.className).toContain("border-b");
    expect(intro.className).toContain("px-3");
    expect(intro.className).toContain("py-2");
  });

  it("forwards className", () => {
    const { container } = render(<PaneIntro title="Skills" className="extra" />);
    expect((container.firstElementChild as HTMLElement).classList.contains("extra")).toBe(true);
  });
});
