import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import { Table, filterTableData, computeVisibleRange, type Column } from "./Table";
import { ageSeconds } from "../lib/age";

afterEach(() => vi.restoreAllMocks());

describe("Table virtualization", () => {
  const bigColumns: Column<{ name: string; phase: string }>[] = [
    { key: "name", header: "Name" },
    { key: "phase", header: "Phase" },
  ];
  const bigData = Array.from({ length: 1000 }, (_, i) => ({ name: `row-${i}`, phase: "x" }));

  it("renders every row when layout can't be measured (jsdom fallback)", () => {
    const { container } = render(
      <Table columns={bigColumns} data={bigData} getRowKey={(r) => r.name} />,
    );
    // No measurable row height → degrade to rendering all rows.
    expect(container.querySelectorAll("tbody tr.fl-data-table__row").length).toBe(1000);
  });

  it("renders only a window of rows when the row height is measurable", () => {
    // Simulate layout: each row 20px tall inside a 200px scroll viewport.
    vi.spyOn(HTMLTableRowElement.prototype, "getBoundingClientRect").mockReturnValue({
      height: 20,
    } as DOMRect);
    const { container } = render(
      <div data-testid="scroll" style={{ overflowY: "auto" }}>
        <Table columns={bigColumns} data={bigData} getRowKey={(r) => r.name} />
      </div>,
    );
    const sp = screen.getByTestId("scroll");
    Object.defineProperty(sp, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(sp, "scrollTop", { value: 0, writable: true, configurable: true });
    fireEvent.scroll(sp);

    const rows = container.querySelectorAll("tbody tr.fl-data-table__row");
    expect(rows.length).toBeLessThan(100); // a window, not all 1000
    expect(rows.length).toBeGreaterThan(0);
    expect(screen.getByText("row-0")).toBeDefined();
    expect(screen.queryByText("row-900")).toBeNull(); // far off-screen, not rendered
  });
});

describe("computeVisibleRange", () => {
  it("returns the window of rows around the scroll position with overscan", () => {
    // rowHeight 20, viewport 100 → 5 visible rows; scrolled to row 50; overscan 3.
    const r = computeVisibleRange({ scrollTop: 1000, viewportHeight: 100, rowHeight: 20, total: 500, overscan: 3 });
    expect(r.start).toBe(47); // 50 - 3
    expect(r.end).toBe(58); // 50 + ceil(100/20)=5 + 3
  });

  it("clamps to the data bounds", () => {
    expect(computeVisibleRange({ scrollTop: 0, viewportHeight: 100, rowHeight: 20, total: 500, overscan: 3 }).start).toBe(0);
    const end = computeVisibleRange({ scrollTop: 999999, viewportHeight: 100, rowHeight: 20, total: 500, overscan: 3 });
    expect(end.end).toBe(500);
    expect(end.start).toBeLessThanOrEqual(500);
  });

  it("renders everything when the row height is unknown (0)", () => {
    // jsdom / pre-measure: fall back to the full range rather than dividing by zero.
    expect(computeVisibleRange({ scrollTop: 0, viewportHeight: 0, rowHeight: 0, total: 42, overscan: 3 })).toEqual({
      start: 0,
      end: 42,
    });
  });
});

interface Row {
  name: string;
  phase: string;
}

const columns: Column<Row>[] = [
  { key: "name", header: "Name" },
  { key: "phase", header: "Phase", render: (r) => <em>{r.phase}</em> },
];

const data: Row[] = [
  { name: "web-1", phase: "Running" },
  { name: "web-2", phase: "Pending" },
];

describe("Table", () => {
  it("renders headers and rows, using custom cell renderers", () => {
    render(<Table columns={columns} data={data} getRowKey={(r) => r.name} />);
    expect(screen.getByText("Name")).toBeDefined();
    expect(screen.getByText("web-1")).toBeDefined();
    // custom render wraps phase in <em>
    expect(screen.getByText("Running").tagName).toBe("EM");
  });

  it("fires onRowClick with the clicked row", () => {
    const onRowClick = vi.fn();
    render(
      <Table columns={columns} data={data} getRowKey={(r) => r.name} onRowClick={onRowClick} />,
    );
    fireEvent.click(screen.getByText("web-2"));
    expect(onRowClick).toHaveBeenCalledWith({ name: "web-2", phase: "Pending" });
  });

  it("marks the selected row via aria-selected", () => {
    render(
      <Table columns={columns} data={data} getRowKey={(r) => r.name} selectedKey="web-1" />,
    );
    const selected = screen.getByText("web-1").closest("tr");
    expect(selected?.getAttribute("aria-selected")).toBe("true");
  });

  it("shows empty text when there is no data", () => {
    render(
      <Table columns={columns} data={[]} getRowKey={(r) => r.name} emptyText="No pods" />,
    );
    expect(screen.getByText("No pods")).toBeDefined();
  });

  it("cycles column sorting through ascending, descending, and unsorted", () => {
    render(<Table columns={columns} data={[...data].reverse()} getRowKey={(r) => r.name} />);
    const sort = screen.getByRole("button", { name: "Sort by Name" });

    fireEvent.click(sort);
    expect(screen.getAllByRole("row")[1].textContent).toContain("web-1");
    fireEvent.click(sort);
    expect(screen.getAllByRole("row")[1].textContent).toContain("web-2");
    fireEvent.click(sort);
    expect(screen.getAllByRole("row")[1].textContent).toContain("web-2");
  });

  it("sorts by getSortValue while filtering stays on the visible text (#236)", () => {
    const rows = [
      { name: "old", age: "1y" },
      { name: "older", age: "2y" },
      { name: "recent", age: "300d" },
    ];
    const ageColumns = [
      { key: "name", header: "Name" },
      {
        key: "age",
        header: "Age",
        getSortValue: (r: (typeof rows)[number]) => ageSeconds(r.age),
      },
    ];
    render(<Table columns={ageColumns} data={rows} getRowKey={(r) => r.name} />);

    // Ascending: 300d < 1y < 2y — numeric collation on the strings would
    // have put both years first (1 < 2 < 300).
    fireEvent.click(screen.getByRole("button", { name: "Sort by Age" }));
    const names = screen.getAllByRole("row").slice(1).map((r) => r.textContent);
    expect(names).toEqual(["recent300d", "old1y", "older2y"]);

    // The filter still matches the display text, not the sort key.
    expect(filterTableData(rows, ageColumns, "1y", null)).toEqual([{ name: "old", age: "1y" }]);
  });

  it("hands sort changes to the owner when controlled (#254)", () => {
    // The tab owns the sort so it survives a switch; the table must not keep
    // a private copy that silently diverges from what it was handed.
    const onSortChange = vi.fn();
    const { rerender } = render(
      <Table
        columns={columns}
        data={data}
        getRowKey={(r) => r.name}
        sort={null}
        onSortChange={onSortChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Sort by Name" }));
    expect(onSortChange).toHaveBeenCalledWith({ key: "name", direction: "asc" });

    // Still unsorted on screen until the owner feeds the new value back.
    expect(screen.getAllByRole("row")[1].textContent).toContain("web-1");

    rerender(
      <Table
        columns={columns}
        data={[...data].reverse()}
        getRowKey={(r) => r.name}
        sort={{ key: "name", direction: "asc" }}
        onSortChange={onSortChange}
      />,
    );
    expect(screen.getAllByRole("row")[1].textContent).toContain("web-1");
  });

  it("keeps its own sort when uncontrolled", () => {
    // Tables outside the tabbed workspace have no tab to store a sort on.
    render(<Table columns={columns} data={[...data].reverse()} getRowKey={(r) => r.name} />);
    fireEvent.click(screen.getByRole("button", { name: "Sort by Name" }));
    expect(screen.getAllByRole("row")[1].textContent).toContain("web-1");
  });

  it("selects a column for the toolbar search", () => {
    const onChange = vi.fn();
    render(
      <Table
        columns={columns}
        data={data}
        getRowKey={(r) => r.name}
        onActiveFilterKeyChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Filter search by Phase" }));
    expect(onChange).toHaveBeenCalledWith("phase");
  });

  it("filters globally or by the selected column", () => {
    expect(filterTableData(data, columns, "running", null)).toEqual([data[0]]);
    expect(filterTableData(data, columns, "web", "phase")).toEqual([]);
    expect(filterTableData(data, columns, "web-2", "name")).toEqual([data[1]]);
  });

  it("resizes a column with the keyboard and resets it on double click", () => {
    render(<Table columns={columns} data={data} getRowKey={(r) => r.name} />);
    const handle = screen.getByRole("separator", { name: "Resize Name column" });
    const header = screen.getByText("Name").closest("th");

    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(header?.closest("table")?.style.width).toBe("256px");
    fireEvent.doubleClick(handle);
    expect(header?.closest("table")?.style.width).toBe("");
  });
});

describe("Table multi-selection", () => {
  const cols: Column<{ name: string }>[] = [{ key: "name", header: "Name" }];
  const rows = [{ name: "a" }, { name: "b" }, { name: "c" }, { name: "d" }];

  function renderWithSelection(selected = new Set<string>()) {
    const onChange = vi.fn();
    const utils = render(
      <Table
        columns={cols}
        data={rows}
        getRowKey={(r) => r.name}
        selection={{ selected, onChange }}
      />,
    );
    return { onChange, ...utils };
  }

  it("toggles a single row", () => {
    const { onChange } = renderWithSelection();
    fireEvent.click(screen.getByLabelText("Select b"));
    expect([...onChange.mock.calls[0][0]]).toEqual(["b"]);
  });

  it("select-all header selects every (filtered) row", () => {
    const { onChange } = renderWithSelection();
    fireEvent.click(screen.getByLabelText("Select all"));
    expect([...onChange.mock.calls[0][0]].sort()).toEqual(["a", "b", "c", "d"]);
  });

  it("shift-click selects the range from the anchor", () => {
    const { onChange, rerender } = renderWithSelection();
    fireEvent.click(screen.getByLabelText("Select a")); // anchor = a
    const selected = onChange.mock.calls[0][0] as Set<string>;
    rerender(
      <Table columns={cols} data={rows} getRowKey={(r) => r.name} selection={{ selected, onChange }} />,
    );
    fireEvent.click(screen.getByLabelText("Select c"), { shiftKey: true });
    expect([...onChange.mock.calls[1][0]].sort()).toEqual(["a", "b", "c"]);
  });

  it("header checkbox is checked when all rows are selected", () => {
    renderWithSelection(new Set(["a", "b", "c", "d"]));
    expect((screen.getByLabelText("Select all") as HTMLInputElement).checked).toBe(true);
  });
});
