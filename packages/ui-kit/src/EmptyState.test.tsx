import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { EmptyState } from "./EmptyState";

describe("EmptyState", () => {
  it("renders the title", () => {
    render(<EmptyState title="No pods" />);
    expect(screen.getByText("No pods")).toBeDefined();
  });

  it("renders a title given as a node, not just a string", () => {
    render(<EmptyState title={<em>No pods</em>} />);
    expect(screen.getByText("No pods").tagName).toBe("EM");
  });

  it("renders the hint when one is given", () => {
    render(<EmptyState title="No pods" hint="Nothing is scheduled in this namespace." />);
    expect(screen.getByText("Nothing is scheduled in this namespace.")).toBeDefined();
  });

  it("omits the hint element, not just its text, when none is given", () => {
    // An empty hint line is still a line: it holds vertical space and pushes
    // the action away from the title.
    const { container } = render(<EmptyState title="No pods" />);
    expect(container.querySelector('[data-slot="hint"]')).toBeNull();
  });

  it("renders the caller's action control as given", () => {
    render(<EmptyState title="No pods" action={<button type="button">Create pod</button>} />);
    // The slot is for a control the caller owns, so it arrives intact rather
    // than as a label this component wraps in a button of its own.
    expect(screen.getByRole("button", { name: "Create pod" })).toBeDefined();
  });

  it("omits the action element, not just its content, when none is given", () => {
    const { container } = render(<EmptyState title="No pods" />);
    expect(container.querySelector('[data-slot="action"]')).toBeNull();
  });

  it("keeps the title as the only content when nothing else is given", () => {
    const { container } = render(<EmptyState title="No pods" />);
    expect(container.firstElementChild?.children).toHaveLength(1);
  });

  it("forwards className onto the root", () => {
    const { container } = render(<EmptyState title="No pods" className="extra" />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.classList.contains("extra")).toBe(true);
    // Merged, not replacing the component's own layout classes.
    expect(root.className.trim()).not.toBe("extra");
  });

  it("announces nothing: a loaded-but-empty result is not a status", () => {
    // LoadingState owns the live region for an in-flight load; an empty result
    // that also announced itself would double up on the same content area.
    const { container } = render(<EmptyState title="No pods" />);
    expect(container.querySelector('[role="status"], [role="alert"]')).toBeNull();
  });
  it("treats a conditional slot that resolved to false as absent", () => {
    // `action={canCreate && <Button />}` passes `false`. The wrapper would
    // still take its margin, leaving the gap the caller meant to remove.
    // (#325 review)
    const { container } = render(<EmptyState title="No pods" hint={false} action={false} />);
    expect(container.querySelector('[data-slot="hint"]')).toBeNull();
    expect(container.querySelector('[data-slot="action"]')).toBeNull();
  });
});

/**
 * The rail-sized form. `py-10` around three wrapped lines in a 286px rail
 * spends more height stating an absence than the section below it gets to
 * exist in — the cluster overview's `Fleet` went below the fold behind one.
 */
describe("a compact empty state", () => {
  it("spends a quarter of the padding, and a step less type", () => {
    const { container } = render(<EmptyState title="No incident feed yet" hint="Not yet." compact />);
    const root = container.firstElementChild;
    expect(root?.getAttribute("data-compact")).toBe("true");
    expect(root?.className).toContain("py-3");
    expect(root?.className).not.toContain("py-10");
  });

  it("emits one padding, never two competing ones", () => {
    // Both forms are utilities, and two utilities setting the same property
    // are resolved by the generated stylesheet's order rather than by the
    // order the JSX writes them. Exactly one set is emitted, so a caller
    // cannot be surprised by which one wins.
    const compact = render(<EmptyState title="t" compact />).container.firstElementChild;
    expect(compact?.className).not.toContain("px-6");
    expect(compact?.className).not.toContain("gap-1.5");
  });

  it("is the page-sized form unless asked", () => {
    const { container } = render(<EmptyState title="No pods" />);
    const root = container.firstElementChild;
    expect(root?.getAttribute("data-compact")).toBeNull();
    expect(root?.className).toContain("py-10");
  });
});
