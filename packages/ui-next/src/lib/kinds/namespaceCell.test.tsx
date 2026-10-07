import { useState } from "react";
import { afterEach, describe, it, expect, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Table, type Column } from "@srelens/ui-kit";
import { REFOCUS_WITHIN_MS, addNamespace, useRowRefocus, withNamespaceSelect } from "./namespaceCell";
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

  it("reads as a link before it is hovered — underlined at rest, not only under the pointer", () => {
    renderList([], vi.fn());
    const classes = screen.getByRole("button", { name: "Show only namespace shop" }).className.split(/\s+/);
    expect(classes).toContain("underline");
    expect(classes).toContain("decoration-dotted");
    expect(classes).toContain("hover:decoration-solid");
    expect(classes).toContain("focus-visible:decoration-solid");
  });

  it("hands the clicked namespace to onAdd, and leaves the row beneath it alone", async () => {
    const onAdd = vi.fn();
    const { onRowClick, onRowActivate } = renderList([], onAdd);

    await userEvent.click(screen.getByRole("button", { name: "Show only namespace monitoring" }));

    expect(onAdd).toHaveBeenCalledTimes(1);
    // The row too, so the screen can bring focus back to it.
    expect(onAdd).toHaveBeenCalledWith("monitoring", ROWS[1]);
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
    // Focus is handed to the row on a press (see the focus test below), so
    // the second key is pressed from the button again.
    screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
    await userEvent.keyboard(" ");

    expect(onAdd).toHaveBeenCalledTimes(2);
    expect(onAdd).toHaveBeenLastCalledWith("monitoring", ROWS[1]);
    expect(onRowActivate).not.toHaveBeenCalled();
  });

  it("keeps a keyboard reader's place: focus moves to the row when the button gives way to text", async () => {
    // Selecting the namespace is what makes its own cell plain text, so the
    // focused button is removed by the very press that used it.
    function Live() {
      const [selection, setSelection] = useState<string[]>([]);
      return (
        <Table
          columns={withNamespaceSelect(COLUMNS, selection, (ns) => setSelection(addNamespace(selection, ns)))}
          data={ROWS}
          getRowKey={(row) => `${row.namespace ?? ""}/${row.name}`}
          onRowClick={() => {}}
          onRowActivate={() => {}}
        />
      );
    }
    render(<Live />);
    const button = screen.getByRole("button", { name: "Show only namespace monitoring" });
    button.focus();

    await userEvent.keyboard("{Enter}");

    expect(screen.queryByRole("button", { name: /namespace monitoring/ })).toBeNull();
    const row = screen.getByText("prometheus-0").closest("tr");
    expect(document.activeElement).toBe(row);
    expect(document.activeElement).not.toBe(document.body);
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

/**
 * The list reloads under a new selection, and a list that is loading shows no
 * table: every row leaves the document, the focused one with it (PR #832
 * review). `Reloading` is that screen in miniature — a pick blanks the table,
 * and `finish()` brings the rows back.
 */
describe("useRowRefocus", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  const keyOf = (row: ListRow) => `${row.namespace ?? ""}/${row.name}`;
  let finish!: () => void;

  function Reloading({ rows = ROWS }: { rows?: ListRow[] }) {
    const [selection, setSelection] = useState<string[]>([]);
    const [loading, setLoading] = useState(false);
    const refocus = useRowRefocus();
    finish = () => setLoading(false);
    const shown = selection.length ? rows.filter((r) => selection.includes(r.namespace ?? "")) : rows;
    return (
      <>
        <input aria-label="Filter" />
        <div ref={refocus.scope}>
          {loading ? (
            <p>Loading</p>
          ) : (
            <Table
              columns={withNamespaceSelect(COLUMNS, selection, (ns, row) => {
                refocus.remember(keyOf(row));
                setSelection(addNamespace(selection, ns));
                setLoading(true);
              })}
              data={shown}
              getRowKey={keyOf}
              onRowClick={() => {}}
              onRowActivate={() => {}}
            />
          )}
        </div>
      </>
    );
  }

  const rowOf = (name: string) => screen.getByText(name).closest("tr");

  it("puts focus back on the row once the reloaded table is on screen", async () => {
    render(<Reloading />);
    screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
    await userEvent.keyboard("{Enter}");

    // The table is gone, and focus with it.
    expect(screen.getByText("Loading")).toBeDefined();
    expect(document.activeElement).toBe(document.body);

    act(() => finish());
    expect(document.activeElement).toBe(rowOf("prometheus-0"));
  });

  it("brings it back after a pointer pick as well", async () => {
    render(<Reloading />);
    await userEvent.click(screen.getByRole("button", { name: "Show only namespace shop" }));
    act(() => finish());
    expect(document.activeElement).toBe(rowOf("web-0"));
  });

  it("leaves focus where the reader put it while the list was loading", async () => {
    render(<Reloading />);
    screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
    await userEvent.keyboard("{Enter}");

    const filter = screen.getByRole("textbox", { name: "Filter" });
    filter.focus();
    act(() => finish());

    expect(document.activeElement).toBe(filter);
    // And it has given up for good: focus dropping later is not its to fix.
    act(() => filter.blur());
    act(() => finish());
    expect(document.activeElement).toBe(document.body);
  });

  describe("when the reader moves on without taking focus anywhere (PR #832 review)", () => {
    // `document.activeElement` is `body` in all of these, exactly as it is
    // when the table has merely been taken away — so where focus sits cannot
    // be what tells them apart.
    async function pickThenLoad() {
      render(
        <>
          <h1>Pods</h1>
          <Reloading />
        </>,
      );
      screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
      await userEvent.keyboard("{Enter}");
      expect(document.activeElement).toBe(document.body);
    }

    it("leaves focus alone after a click on something that takes no focus", async () => {
      await pickThenLoad();
      fireEvent.pointerDown(screen.getByRole("heading", { name: "Pods" }));
      act(() => finish());
      expect(document.activeElement).toBe(document.body);
    });

    it("leaves focus alone after a key pressed with focus on the page", async () => {
      await pickThenLoad();
      fireEvent.keyDown(document.body, { key: "Tab" });
      act(() => finish());
      expect(document.activeElement).toBe(document.body);
    });

    it("still brings focus back after a click inside the table's own area", async () => {
      await pickThenLoad();
      // The loading state is where the table was; a click on it is not a move away.
      fireEvent.pointerDown(screen.getByText("Loading"));
      act(() => finish());
      expect(document.activeElement).toBe(rowOf("prometheus-0"));
    });

    it("stops listening once it has brought focus back", async () => {
      const remove = vi.spyOn(document, "removeEventListener");
      await pickThenLoad();
      act(() => finish());
      expect(document.activeElement).toBe(rowOf("prometheus-0"));
      expect(remove).toHaveBeenCalledWith("pointerdown", expect.any(Function), true);
      expect(remove).toHaveBeenCalledWith("keydown", expect.any(Function), true);
      remove.mockRestore();
    });

    it("stops listening when the screen goes away with a row still remembered", async () => {
      const remove = vi.spyOn(document, "removeEventListener");
      const view = render(<Reloading />);
      screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
      await userEvent.keyboard("{Enter}");
      remove.mockClear();

      view.unmount();
      expect(remove).toHaveBeenCalledWith("pointerdown", expect.any(Function), true);
      remove.mockRestore();
    });
  });

  it("does nothing when the row did not come back", async () => {
    const view = render(<Reloading />);
    screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
    await userEvent.keyboard("{Enter}");

    // Deleted while the list was loading.
    view.rerender(<Reloading rows={[ROWS[0]]} />);
    act(() => finish());
    expect(document.activeElement).toBe(document.body);
  });

  it("gives up after a while rather than moving focus on a reader who has moved on", async () => {
    vi.useFakeTimers({ now: new Date("2026-10-07T10:00:00.000Z"), shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<Reloading />);
    screen.getByRole("button", { name: "Show only namespace monitoring" }).focus();
    await user.keyboard("{Enter}");

    vi.setSystemTime(Date.now() + REFOCUS_WITHIN_MS + 1);
    act(() => finish());
    expect(document.activeElement).toBe(document.body);
  });

  it("never moves focus when no row was remembered", () => {
    render(<Reloading />);
    act(() => finish());
    expect(document.activeElement).toBe(document.body);
  });
});
