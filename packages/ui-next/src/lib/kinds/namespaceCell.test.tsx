import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Table, type Column } from "@srelens/ui-kit";
import { addNamespace, withNamespaceSelect } from "./namespaceCell";
import type { ListRow } from "./types";

const COLUMNS: Column<ListRow>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
];

const ROWS: ListRow[] = [
  { name: "web-0", namespace: "shop" },
  { name: "prometheus-0", namespace: "monitoring" },
];

function renderList(
  selection: string[],
  onAdd: ((namespace: string) => void) | undefined,
  rows: ListRow[] = ROWS,
  columns: Column<ListRow>[] = COLUMNS,
) {
  const onRowClick = vi.fn();
  const onRowActivate = vi.fn();
  render(
    <Table
      columns={withNamespaceSelect(columns, selection, onAdd)}
      data={rows}
      getRowKey={(row) => `${row.namespace ?? ""}/${row.name}`}
      onRowClick={onRowClick}
      onRowActivate={onRowActivate}
    />,
  );
  return { onRowClick, onRowActivate };
}

describe("addNamespace", () => {
  it("narrows an empty selection — all namespaces — to the one added", () => {
    expect(addNamespace([], "monitoring")).toEqual(["monitoring"]);
  });

  it("adds to the namespaces already selected, after them", () => {
    expect(addNamespace(["shop"], "monitoring")).toEqual(["shop", "monitoring"]);
  });

  it("changes nothing when the namespace is already selected", () => {
    expect(addNamespace(["shop", "monitoring"], "shop")).toEqual(["shop", "monitoring"]);
  });

  it("hands back a new array rather than the selection it was given", () => {
    const selection = ["shop"];
    expect(addNamespace(selection, "shop")).not.toBe(selection);
  });
});

describe("withNamespaceSelect", () => {
  it("makes each namespace a button that says what a click will show", () => {
    renderList([], vi.fn());
    expect(screen.getByRole("button", { name: "Show only namespace monitoring" }).textContent).toBe("monitoring");
    expect(screen.getByRole("button", { name: "Show only namespace shop" })).toBeDefined();
  });

  it("hands the clicked namespace to onAdd, and leaves the row beneath it alone", async () => {
    const onAdd = vi.fn();
    const { onRowClick, onRowActivate } = renderList([], onAdd);

    await userEvent.click(screen.getByRole("button", { name: "Show only namespace monitoring" }));

    expect(onAdd).toHaveBeenCalledTimes(1);
    expect(onAdd).toHaveBeenCalledWith("monitoring");
    // The row peeks on a click. A namespace picked off it must not also open
    // the resource it was read off.
    expect(onRowClick).not.toHaveBeenCalled();
    expect(onRowActivate).not.toHaveBeenCalled();
  });

  it("does not open the row's tab on a double-click of the namespace", async () => {
    const { onRowActivate } = renderList([], vi.fn());
    await userEvent.dblClick(screen.getByRole("button", { name: "Show only namespace shop" }));
    expect(onRowActivate).not.toHaveBeenCalled();
  });

  it("is reached and pressed from the keyboard, without Enter opening the row", async () => {
    const onAdd = vi.fn();
    const { onRowActivate } = renderList([], onAdd);

    screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard(" ");

    expect(onAdd).toHaveBeenCalledTimes(2);
    expect(onAdd).toHaveBeenLastCalledWith("monitoring");
    expect(onRowActivate).not.toHaveBeenCalled();
  });

  it("says a click ADDS when some namespaces are already selected", () => {
    renderList(["shop"], vi.fn());
    expect(screen.getByRole("button", { name: "Also show namespace monitoring" })).toBeDefined();
  });

  it("leaves a namespace that is already selected as plain text — there is nothing to add", () => {
    renderList(["shop"], vi.fn());
    expect(screen.queryByRole("button", { name: /namespace shop/ })).toBeNull();
    expect(screen.getByText("shop")).toBeDefined();
  });

  it("leaves every cell plain when there is no way to add — a namespace-scoped credential", () => {
    renderList([], undefined);
    expect(screen.queryByRole("button", { name: /namespace/ })).toBeNull();
    expect(screen.getByText("monitoring")).toBeDefined();
    expect(screen.getByText("shop")).toBeDefined();
  });

  it("draws nothing for a row with no namespace", () => {
    renderList([], vi.fn(), [{ name: "worker-1" }]);
    expect(screen.queryByRole("button", { name: /namespace/ })).toBeNull();
  });

  it("leaves a namespace column that renders its own cell alone", () => {
    const own: Column<ListRow>[] = [
      { key: "name", header: "Name" },
      { key: "namespace", header: "Namespace", render: (row) => <em>{row.namespace}</em> },
    ];
    renderList([], vi.fn(), ROWS, own);
    expect(screen.queryByRole("button", { name: /namespace/ })).toBeNull();
    expect(screen.getByText("monitoring").tagName).toBe("EM");
  });

  it("touches no other column, and keeps the namespace column's own sorting", () => {
    const decorated = withNamespaceSelect(COLUMNS, [], vi.fn());
    expect(decorated[0]).toBe(COLUMNS[0]);
    expect(decorated[1]).toMatchObject({ key: "namespace", header: "Namespace", sortable: true });
  });
});
