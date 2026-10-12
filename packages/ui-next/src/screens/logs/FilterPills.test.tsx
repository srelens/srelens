import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { parseLogQuery, type QueryClause } from "@srelens/core";
import { FilterPills } from "./FilterPills";

function pills(query: string) {
  const parsed = parseLogQuery(query);
  const clauses: readonly QueryClause[] = parsed.ok ? parsed.clauses : [];
  const onQueryChange = vi.fn();
  const { container } = render(<FilterPills query={query} clauses={clauses} onQueryChange={onQueryChange} />);
  return { container, onQueryChange };
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
});
