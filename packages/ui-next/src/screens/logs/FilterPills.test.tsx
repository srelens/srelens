import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { parseLogQuery, type QueryClause } from "@srelens/core";
import { FilterPills } from "./FilterPills";

function pills(query: string) {
  const parsed = parseLogQuery(query);
  const clauses: readonly QueryClause[] = parsed.ok ? parsed.clauses : [];
  const onQueryChange = vi.fn();
  const onRemoved = vi.fn();
  const { container } = render(
    <FilterPills query={query} clauses={clauses} onQueryChange={onQueryChange} onRemoved={onRemoved} />,
  );
  return { container, onQueryChange, onRemoved };
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

  it("draws Clear all as the kit's text button", () => {
    pills("a b");
    expect(screen.getByRole("button", { name: "Clear all" }).classList.contains("text-btn")).toBe(true);
  });

  it("lets a long unbroken clause wrap inside the pane", () => {
    const { container } = pills("x".repeat(120));
    const pill = container.querySelector('[data-slot="pill"]');
    expect(pill?.classList.contains("max-w-full")).toBe(true);
    expect(pill?.classList.contains("break-all")).toBe(true);
  });
});
