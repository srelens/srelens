import type { FormEvent } from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ErrorState } from "./ErrorState";

describe("ErrorState", () => {
  it("is an alert", () => {
    // A failed load appears without the user having asked for anything, so it
    // has to announce itself; a silent card is a screen-reader dead end.
    render(<ErrorState title="Could not load pods" />);
    expect(screen.getByRole("alert")).toBeDefined();
  });

  it("renders the title", () => {
    render(<ErrorState title="Could not load pods" />);
    expect(screen.getByText("Could not load pods")).toBeDefined();
  });

  it("renders a title given as a node, not just a string", () => {
    render(<ErrorState title={<em>Could not load pods</em>} />);
    expect(screen.getByText("Could not load pods").tagName).toBe("EM");
  });

  it("renders the detail when one is given", () => {
    render(<ErrorState title="Could not load pods" detail="connection refused" />);
    expect(screen.getByText("connection refused")).toBeDefined();
  });

  it("omits the detail element, not just its text, when none is given", () => {
    // An empty paragraph still occupies a line and separates the title from
    // the buttons that are meant to sit right under it.
    const { container } = render(<ErrorState title="Could not load pods" />);
    expect(container.querySelector('[data-slot="detail"]')).toBeNull();
  });

  it("keeps the warning glyph out of the announcement", () => {
    // The alert should read as its words. A glyph that exposed itself would
    // prefix every failure with a name the caller never wrote.
    const { container } = render(<ErrorState title="Could not load pods" />);
    const glyph = container.querySelector("svg");
    expect(glyph).not.toBeNull();
    expect(glyph?.getAttribute("aria-hidden")).toBe("true");
  });

  it("fires onRetry from a button labelled Retry by default", () => {
    const onRetry = vi.fn();
    render(<ErrorState title="Could not load pods" onRetry={onRetry} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("labels the retry button with retryLabel when one is given", () => {
    const onRetry = vi.fn();
    render(<ErrorState title="Could not load pods" onRetry={onRetry} retryLabel="Reconnect" />);
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("renders the secondary action and fires its onClick", () => {
    const onClick = vi.fn();
    render(
      <ErrorState
        title="Could not load pods"
        action={{ label: "Diagnose in Toolbox", onClick }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Diagnose in Toolbox" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("keeps the two actions independent", () => {
    // Both are secondary buttons side by side, so it is worth pinning that a
    // press reaches the one that was pressed and not its neighbour.
    const onRetry = vi.fn();
    const onClick = vi.fn();
    render(
      <ErrorState
        title="Could not load pods"
        onRetry={onRetry}
        action={{ label: "Diagnose in Toolbox", onClick }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("omits the actions row entirely when there is nothing to do", () => {
    // Not merely buttonless: an empty flex row keeps its gap and padding, so
    // the card would end in a band of blank space under the detail.
    const { container } = render(<ErrorState title="Could not load pods" detail="no retry" />);
    expect(container.querySelector('[data-slot="actions"]')).toBeNull();
    expect(screen.queryAllByRole("button")).toHaveLength(0);
  });

  it("forwards className onto the root", () => {
    const { container } = render(<ErrorState title="Could not load pods" className="extra" />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.classList.contains("extra")).toBe(true);
    // Merged, not replacing the component's own layout classes.
    expect(root.className.trim()).not.toBe("extra");
  });
  it("does not submit a form it is standing in", () => {
    // `Button` deliberately leaves `type` alone, so a bare one in a form is a
    // submit button (bd24d1a). These two are this component's own, not the
    // caller's, so it is this component that owes them a type: a failed load
    // inside a form would otherwise retry and submit the form with it.
    // (#325 review)
    const onSubmit = vi.fn((e: FormEvent) => e.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <ErrorState
          title="Could not load pods"
          onRetry={() => {}}
          action={{ label: "Diagnose in Toolbox", onClick: () => {} }}
        />
      </form>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    fireEvent.click(screen.getByRole("button", { name: "Diagnose in Toolbox" }));
    expect(onSubmit).not.toHaveBeenCalled();
  });
  it("treats a conditional detail that resolved to false as absent", () => {
    // (#325 review)
    const { container } = render(<ErrorState title="Could not load pods" detail={false} />);
    expect(container.querySelector('[data-slot="detail"]')).toBeNull();
  });

  it("folds the original message away under the detail rather than dropping it", () => {
    const raw = "ApiError: Unauthorized (Status { code: Some(401) })";
    const { container } = render(
      <ErrorState
        title="Could not list pods on prod-eu"
        detail="The cluster rejected your credentials."
        raw={raw}
      />,
    );
    const disclosure = container.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(disclosure).not.toBeNull();
    expect(disclosure.open).toBe(false);
    expect(disclosure.textContent).toContain(raw);
    // And nowhere in an attribute — the rule PairList and KV settled. (#331)
    for (const node of Array.from(container.querySelectorAll("*"))) {
      for (const attribute of Array.from(node.attributes)) {
        expect(attribute.value).not.toContain(raw);
      }
    }
  });

  it("shows no disclosure when the detail IS the original message", () => {
    // A caller whose `describeError` fell through to the generic case has the
    // same string in both fields; drawing it twice reads as two problems.
    const { container } = render(
      <ErrorState title="Could not load pods" detail="connection refused" />,
    );
    expect(container.querySelector('[data-slot="raw"]')).toBeNull();
  });
});
