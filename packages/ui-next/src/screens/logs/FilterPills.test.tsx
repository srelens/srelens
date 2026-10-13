import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { parseLogQuery, type QueryClause, type TimeRange } from "@srelens/core";
import { FilterPills } from "./FilterPills";

function pills(query: string, range?: TimeRange) {
  const parsed = parseLogQuery(query);
  const clauses: readonly QueryClause[] = parsed.ok ? parsed.clauses : [];
  const onQueryChange = vi.fn();
  const onRangeChange = vi.fn();
  const onRemoved = vi.fn();
  const { container } = render(
    <FilterPills
      query={query}
      clauses={clauses}
      range={range}
      onQueryChange={onQueryChange}
      onRangeChange={onRangeChange}
      onRemoved={onRemoved}
    />,
  );
  return { container, onQueryChange, onRangeChange, onRemoved };
}

const labels = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('[data-slot="pill"]')).map((p) => p.textContent);

describe("FilterPills", () => {
  it("draws nothing while there is nothing to remove", () => {
    expect(pills("  ").container.innerHTML).toBe("");
  });

  it("draws one pill per clause, in query order, with a negation as NOT", () => {
    expect(labels(pills('level:error -system:dev "wrote batch"').container)).toEqual([
      "level:error",
      "NOT system:dev",
      '"wrote batch"',
    ]);
  });

  it("removes exactly the clause whose × was pressed", async () => {
    const { onQueryChange } = pills('level:error -system:dev "wrote batch"');
    await userEvent.click(screen.getByRole("button", { name: "Remove filter -system:dev" }));
    expect(onQueryChange).toHaveBeenCalledWith('level:error "wrote batch"');
  });

  it("clears the whole query", async () => {
    const { onQueryChange } = pills("a b");
    await userEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(onQueryChange).toHaveBeenCalledWith("");
  });

  it("tells the caller a pill was removed, once", async () => {
    // The caller uses this to put focus back in the box, since the × that had
    // it is about to unmount.
    const { onRemoved } = pills("level:error -system:dev");
    await userEvent.click(screen.getByRole("button", { name: "Remove filter level:error" }));
    expect(onRemoved).toHaveBeenCalledTimes(1);
  });

  it("tells the caller Clear all was used, once", async () => {
    const { onRemoved } = pills("a b");
    await userEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(onRemoved).toHaveBeenCalledTimes(1);
  });

  it("draws Clear all as the kit's text button, at the size of the pills beside it", () => {
    pills("a b");
    const clear = screen.getByRole("button", { name: "Clear all" });
    expect(clear.classList.contains("text-btn")).toBe(true);
    // The kit's button takes the size of whatever it sits in, and a flex row of pills is not a font size.
    expect(clear.classList.contains("text-[0.75rem]")).toBe(true);
  });

  it("exports the filter strip alone: a pill is its detail", async () => {
    expect(Object.keys(await import("./FilterPills"))).toEqual(["FilterPills"]);
  });

  it("draws the time window as a pill of its own, and Clear all clears it too", async () => {
    const from = new Date(2026, 9, 12, 10, 33, 0).getTime();
    const { container, onQueryChange, onRangeChange } = pills("", { from, to: from + 90_000 });
    expect(labels(container)).toEqual(["10:33:00 – 10:34:30"]);
    await userEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(onQueryChange).toHaveBeenCalledWith("");
    expect(onRangeChange).toHaveBeenCalledWith(undefined);
  });

  it("removes the window alone with its ×, and tells the caller, once", async () => {
    // Same reason as a clause's ×: the button that had focus is about to unmount.
    const from = new Date(2026, 9, 12, 10, 33, 0).getTime();
    const { onQueryChange, onRangeChange, onRemoved } = pills("a", { from, to: from + 1000 });
    await userEvent.click(screen.getByRole("button", { name: "Remove the time window" }));
    expect(onRangeChange).toHaveBeenCalledWith(undefined);
    expect(onQueryChange).not.toHaveBeenCalled();
    expect(onRemoved).toHaveBeenCalledTimes(1);
  });

  it("clears the window before telling the caller Clear all was used", async () => {
    const from = new Date(2026, 9, 12, 10, 33, 0).getTime();
    const { onRangeChange, onRemoved } = pills("a", { from, to: from + 1000 });
    await userEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(onRangeChange.mock.invocationCallOrder[0]).toBeLessThan(onRemoved.mock.invocationCallOrder[0]);
  });

  it("lets a long unbroken clause run on and scroll inside the pane, rather than wrap", () => {
    // This pinned `break-all`. A clause is machine text, and AGENTS.md says that does not wrap.
    const { container } = pills("x".repeat(120));
    const pill = container.querySelector('[data-slot="pill"]');
    expect(pill?.classList.contains("max-w-full")).toBe(true);
    expect(pill?.classList.contains("whitespace-nowrap")).toBe(true);
    expect(pill?.classList.contains("overflow-x-auto")).toBe(true);
    expect(pill?.classList.contains("break-all")).toBe(false);
  });
});
