import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Table, filterTableData, computeVisibleRange, rowPitch, type Column } from "./Table";

/**
 * Compact ages as seconds, standing in for core's `ageSeconds`, which the kit
 * has no dependency on. What the test below is about is the Table honouring
 * `getSortValue` when the visible text does not order correctly — "1y" against
 * "300d" — not whether this parser is right; core tests that where it lives.
 */
function ageSeconds(age: string): number {
  const [, n, unit] = /^(\d+)([smhdy])$/.exec(age) ?? [];
  const scale: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86400, y: 31536000 };
  return Number(n) * (scale[unit] ?? 0);
}

afterEach(() => vi.restoreAllMocks());

/**
 * The classic Table's tests, carried over whole. Every behavioural assertion is
 * unchanged — virtualisation, the #298/#299 column-width pinning, sorting,
 * selection, filtering. What moved is the class names the queries hang off:
 * `fl-data-table__row` is now `tbl-row` and `fl-data-table--resized` is
 * `tbl-resized`, because the presentation is rebuilt against the new design
 * while the behaviour is not. (#319)
 */

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
    expect(container.querySelectorAll("tbody tr.tbl-row").length).toBe(1000);
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

    const rows = container.querySelectorAll("tbody tr.tbl-row");
    expect(rows.length).toBeLessThan(100); // a window, not all 1000
    expect(rows.length).toBeGreaterThan(0);
    expect(screen.getByText("row-0")).toBeDefined();
    expect(screen.queryByText("row-900")).toBeNull(); // far off-screen, not rendered
  });

  /** Simulate real layout: 20px rows, and header cells with natural widths. */
  function mockLayout(headWidth = 140) {
    vi.spyOn(HTMLTableRowElement.prototype, "getBoundingClientRect").mockReturnValue({
      height: 20,
    } as DOMRect);
    vi.spyOn(HTMLTableCellElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: headWidth,
    } as DOMRect);
  }

  function renderScrollable(data: { name: string; phase: string }[]) {
    const utils = render(
      <div data-testid="scroll" style={{ overflowY: "auto" }}>
        <Table columns={bigColumns} data={data} getRowKey={(r) => r.name} />
      </div>,
    );
    const sp = screen.getByTestId("scroll");
    Object.defineProperty(sp, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(sp, "scrollTop", { value: 0, writable: true, configurable: true });
    fireEvent.scroll(sp);
    return { ...utils, sp };
  }

  const colWidths = (container: HTMLElement) =>
    Array.from(container.querySelectorAll("colgroup col")).map(
      (col) => (col as HTMLTableColElement).style.width,
    );

  it("pins column widths once a long list virtualizes (#298)", () => {
    mockLayout();
    const { container } = renderScrollable(bigData);
    // Without pinned widths the browser re-derives them from whichever rows are
    // currently rendered, so the columns shift on every scroll.
    expect(colWidths(container).every((w) => w !== "")).toBe(true);
    expect(container.querySelector("table")?.className).toContain("tbl-resized");
  });

  it("keeps those widths identical across scrolls (#298)", () => {
    mockLayout();
    const { container, sp } = renderScrollable(bigData);
    const before = colWidths(container);
    expect(before.every((w) => w !== "")).toBe(true); // guard: not vacuously equal
    Object.defineProperty(sp, "scrollTop", { value: 4000, writable: true, configurable: true });
    fireEvent.scroll(sp);
    expect(screen.queryByText("row-0")).toBeNull(); // the rendered window really moved
    expect(colWidths(container)).toEqual(before);
  });

  it("pins short lists too, so every table behaves the same way (#298)", () => {
    // CRDs, Services and most lists sit under the virtualization threshold.
    // They must not size differently from the long ones.
    mockLayout();
    const { container } = renderScrollable(bigData.slice(0, 10));
    expect(colWidths(container).every((w) => w !== "")).toBe(true);
    expect(container.querySelector("table")?.className).toContain("tbl-resized");
  });

  it("keeps widths pinned when filtering drops a list below the threshold (#298)", () => {
    // Narrowing a virtualized list past the threshold must not hand it back to
    // automatic layout, or the columns snap as the user types.
    mockLayout();
    const { container, rerender } = render(
      <div data-testid="scroll" style={{ overflowY: "auto" }}>
        <Table columns={bigColumns} data={bigData} getRowKey={(r) => r.name} />
      </div>,
    );
    const sp = screen.getByTestId("scroll");
    Object.defineProperty(sp, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(sp, "scrollTop", { value: 0, writable: true, configurable: true });
    fireEvent.scroll(sp);
    const before = colWidths(container);
    expect(before.every((w) => w !== "")).toBe(true);

    rerender(
      <div data-testid="scroll" style={{ overflowY: "auto" }}>
        <Table columns={bigColumns} data={bigData.slice(0, 5)} getRowKey={(r) => r.name} />
      </div>,
    );
    expect(colWidths(container)).toEqual(before);
  });

  it("does not pin an empty table, so widths are measured from real rows", () => {
    mockLayout();
    const { container } = renderScrollable([]);
    expect(container.querySelector("colgroup")).toBeNull(); // empty state, no table
  });
});

/**
 * MEASURED in Chrome, 1500 rows, dpr 1, against `packages/ui-kit/src/styles/kit.css`:
 * every rendered row reports `getBoundingClientRect().height` 27.375 EXCEPT the
 * one sitting directly under the top spacer, which reports 27.000. The table is
 * `border-collapse: collapse` and `.tbl-spacer` declares `border: 0 !important`,
 * so the boundary row has no rule to share with the row above it and comes out a
 * fraction of a pixel shorter than every other row in the list.
 *
 * That is the row the virtualizer used to sample, and the sample is multiplied by
 * the number of rows the window skips. Measured consequence: the scroll
 * container's `scrollHeight` flipped between 41089 and 40544 — 545px — as the
 * sampled row moved in and out of the spacer's shadow, on every scroll, which is
 * the scrollbar resizing under the reader's thumb.
 *
 * The mock below reproduces exactly that geometry: heights, and the tops that
 * follow from them.
 */
function mockCollapsedBorderLayout() {
  const BOUNDARY = 27;
  const INTERIOR = 27.375;
  vi.spyOn(HTMLTableRowElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLTableRowElement) {
      const rows = Array.from(this.parentElement?.querySelectorAll("tr.tbl-row") ?? []);
      const index = rows.indexOf(this);
      // Only the row under a spacer is short; without one, every row is interior.
      const shortFirst = Boolean(
        (rows[0] as HTMLElement | undefined)?.previousElementSibling?.classList.contains(
          "tbl-spacer",
        ),
      );
      const height = index === 0 && shortFirst ? BOUNDARY : INTERIOR;
      const top =
        index <= 0 ? 0 : (shortFirst ? BOUNDARY : INTERIOR) + (index - 1) * INTERIOR;
      return { height, top, width: 140 } as DOMRect;
    },
  );
  vi.spyOn(HTMLTableCellElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 140,
  } as DOMRect);
}

/** Every `<td style="height: …">` a spacer row reserves, in pixels. */
function padHeight(container: HTMLElement): number {
  return Array.from(container.querySelectorAll("tr.tbl-spacer td")).reduce(
    (total, td) => total + parseFloat((td as HTMLTableCellElement).style.height || "0"),
    0,
  );
}

/** The whole list's height as the DOM reports it: the pads, plus the real rows. */
function scrollExtent(container: HTMLElement): number {
  const rows = Array.from(container.querySelectorAll("tbody tr.tbl-row")).reduce(
    (total, row) => total + row.getBoundingClientRect().height,
    0,
  );
  return padHeight(container) + rows;
}

describe("Table virtual scroll extent", () => {
  const cols: Column<{ name: string; phase: string }>[] = [
    { key: "name", header: "Name" },
    { key: "phase", header: "Phase" },
  ];
  const data = Array.from({ length: 1000 }, (_, i) => ({ name: `row-${i}`, phase: "x" }));

  function scrolled(to: number) {
    const utils = render(
      <div data-testid="scroll" style={{ overflowY: "auto" }}>
        <Table columns={cols} data={data} getRowKey={(r) => r.name} />
      </div>,
    );
    const sp = screen.getByTestId("scroll");
    Object.defineProperty(sp, "clientHeight", { value: 200, configurable: true });
    Object.defineProperty(sp, "scrollTop", { value: 0, writable: true, configurable: true });
    fireEvent.scroll(sp);
    Object.defineProperty(sp, "scrollTop", { value: to, writable: true, configurable: true });
    // Twice: `measure` reads the DOM the *previous* render left behind, so the
    // first scroll to a new position still sees the window it is replacing. The
    // second is the one that samples a row under a spacer — which is exactly the
    // steady state a reader scrolling a long list is in.
    fireEvent.scroll(sp);
    fireEvent.scroll(sp);
    return { ...utils, sp };
  }

  it("reserves the interior row pitch per skipped row, not the boundary row's height", () => {
    mockCollapsedBorderLayout();
    const { container } = scrolled(4000);
    const { start } = computeVisibleRange({
      scrollTop: 4000,
      viewportHeight: 200,
      rowHeight: 27.375,
      total: 1000,
      overscan: 8,
    });
    const top = parseFloat(
      (container.querySelector("tr.tbl-spacer td") as HTMLTableCellElement).style.height,
    );
    expect(top).toBeCloseTo(start * 27.375, 3);
    // Guard: the two units really do disagree, so this is not vacuously true.
    expect(start * 27.375).not.toBeCloseTo(
      computeVisibleRange({
        scrollTop: 4000,
        viewportHeight: 200,
        rowHeight: 27,
        total: 1000,
        overscan: 8,
      }).start * 27,
      3,
    );
  });

  it("keeps the scroll extent steady as the list scrolls", () => {
    mockCollapsedBorderLayout();
    const { container, sp } = scrolled(0);
    const atTop = scrollExtent(container);
    expect(atTop).toBeGreaterThan(20000); // guard: a real extent, not zero
    Object.defineProperty(sp, "scrollTop", { value: 4000, writable: true, configurable: true });
    fireEvent.scroll(sp);
    fireEvent.scroll(sp);
    // 1px of slack: the boundary row genuinely is 0.375px shorter than the rest.
    // The defect this pins moved the extent by 375px on this list, and by a
    // measured 545px on the 1500-row list it was found on.
    expect(scrollExtent(container)).toBeCloseTo(atTop, 0);
  });
});

describe("Table sorting", () => {
  it("orders 64-bit integers exactly, beyond Number's safe range", () => {
    // Number() collapses these two to the same value, so a numeric sort key
    // cannot separate them. Negative, because string collation happens to get
    // large positives right and would hide the gap: it compares "-...993"
    // against "-...992" by magnitude and puts them the wrong way round.
    // The comparator must also not use arithmetic on a bigint, which throws.
    const rows = [
      { id: "b", n: -9007199254740992n },
      { id: "a", n: -9007199254740993n },
    ];
    const cols: Column<(typeof rows)[number]>[] = [
      { key: "id", header: "Id" },
      { key: "n", header: "N", getSortValue: (r) => r.n },
    ];
    render(<Table columns={cols} data={rows} getRowKey={(r) => r.id} />);
    fireEvent.click(screen.getByRole("button", { name: "Sort by N" }));
    const order = Array.from(document.querySelectorAll("tbody tr.tbl-row")).map(
      (tr) => tr.textContent?.[0],
    );
    expect(order).toEqual(["a", "b"]);
  });

  it("sorts rows with no value below real ones when mixing bigint and number", () => {
    const rows = [
      { id: "big", n: 12n as bigint | number },
      { id: "none", n: Number.NEGATIVE_INFINITY },
      { id: "small", n: 3n },
    ];
    const cols: Column<(typeof rows)[number]>[] = [
      { key: "id", header: "Id" },
      { key: "n", header: "N", getSortValue: (r) => r.n },
    ];
    render(<Table columns={cols} data={rows} getRowKey={(r) => r.id} />);
    fireEvent.click(screen.getByRole("button", { name: "Sort by N" }));
    const order = Array.from(document.querySelectorAll("tbody tr.tbl-row")).map(
      (tr) => tr.textContent?.slice(0, 5),
    );
    expect(order?.[0]).toContain("none");
  });
});

describe("rowPitch", () => {
  it("takes the distance between two interior rows, not the first gap", () => {
    // Chrome's own numbers: a 27.000 boundary row, then 27.375 all the way down.
    expect(rowPitch([0, 27, 54.375, 81.75], 27)).toBeCloseTo(27.375, 5);
  });

  it("falls back to the row's own height when there is no interior gap to read", () => {
    expect(rowPitch([0, 27], 27)).toBe(27);
    expect(rowPitch([0], 27)).toBe(27);
    expect(rowPitch([], 27)).toBe(27);
  });

  it("falls back when the tops are not measurable, which is jsdom", () => {
    // Every rect is zeroed there, so the distance is 0 and the table renders
    // every row rather than dividing the list up by nothing.
    expect(rowPitch([0, 0, 0], 0)).toBe(0);
    expect(rowPitch([NaN, NaN, NaN], 20)).toBe(20);
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
    // filterable is opt-in (design correction, #319 follow-up): the default
    // `columns` fixture doesn't ask for a funnel, so this test opts one in
    // locally rather than relying on what used to be the default.
    const filterableColumns: Column<Row>[] = [
      columns[0],
      { ...columns[1], filterable: true },
    ];
    render(
      <Table
        columns={filterableColumns}
        data={data}
        getRowKey={(r) => r.name}
        onActiveFilterKeyChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Filter search by Phase" }));
    expect(onChange).toHaveBeenCalledWith("phase");
  });

  it("shows no filter funnel for a column that hasn't opted in", () => {
    // Only `filterable: true` earns a funnel now; the old default was
    // "shown unless explicitly false".
    render(
      <Table
        columns={columns}
        data={data}
        getRowKey={(r) => r.name}
        onActiveFilterKeyChange={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "Filter search by Name" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Filter search by Phase" })).toBeNull();
  });

  it("shows a funnel only for columns that set filterable: true, even with a handler", () => {
    const onlyNameFilterable: Column<Row>[] = [
      { ...columns[0], filterable: true },
      columns[1],
    ];
    render(
      <Table
        columns={onlyNameFilterable}
        data={data}
        getRowKey={(r) => r.name}
        onActiveFilterKeyChange={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Filter search by Name" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Filter search by Phase" })).toBeNull();
  });

  it("shows no sort caret for any column at rest, but keeps every column sortable", () => {
    // The always-visible ArrowUpDown placeholder is gone: at rest, a header's
    // `data-on` is "false" whether or not it's sortable, and the sort button
    // still cycles the column when clicked (sortable is unaffected).
    render(<Table columns={columns} data={[...data].reverse()} getRowKey={(r) => r.name} />);
    const nameSort = screen.getByRole("button", { name: "Sort by Name" });
    const phaseSort = screen.getByRole("button", { name: "Sort by Phase" });
    expect(nameSort.getAttribute("data-on")).toBe("false");
    expect(phaseSort.getAttribute("data-on")).toBe("false");

    fireEvent.click(phaseSort);
    // Clicking a column that was never the active sort still sorts it —
    // only the resting indicator changed, not sortability.
    expect(screen.getAllByRole("row")[1].textContent).toContain("Pending");
    expect(phaseSort.getAttribute("data-on")).toBe("true");
    expect(nameSort.getAttribute("data-on")).toBe("false");
  });

  it("marks only the active sort column with data-on, and flips it as sort cycles", () => {
    render(<Table columns={columns} data={data} getRowKey={(r) => r.name} />);
    const nameSort = screen.getByRole("button", { name: "Sort by Name" });

    fireEvent.click(nameSort); // asc
    expect(nameSort.getAttribute("data-on")).toBe("true");
    fireEvent.click(nameSort); // desc
    expect(nameSort.getAttribute("data-on")).toBe("true");
    fireEvent.click(nameSort); // cleared
    expect(nameSort.getAttribute("data-on")).toBe("false");
  });

  it("gives a non-active sortable column's indicator a hover/focus reveal, hidden at rest", () => {
    // jsdom applies no real stylesheet, so the resting/reveal behaviour is
    // asserted through the utility classes that drive it (the same proxy the
    // #298 pinned-width tests use for `.tbl-resized`) rather than computed style.
    render(<Table columns={columns} data={data} getRowKey={(r) => r.name} />);
    const nameSort = screen.getByRole("button", { name: "Sort by Name" });
    const caret = nameSort.querySelector(".th-caret");
    expect(caret).not.toBeNull();
    expect(caret?.className).toContain("opacity-0");
    expect(caret?.className).toContain("group-hover:opacity-100");
    expect(caret?.className).toContain("group-focus-visible:opacity-100");
    // The button itself is the hover/focus group the caret reacts to.
    expect(nameSort.className.split(/\s+/)).toContain("group");
  });

  it("does not hide the active sort column's indicator behind the hover/focus reveal", () => {
    render(<Table columns={columns} data={data} getRowKey={(r) => r.name} />);
    fireEvent.click(screen.getByRole("button", { name: "Sort by Name" }));
    const nameSort = screen.getByRole("button", { name: "Sort by Name" });
    const caret = nameSort.querySelector(".th-caret");
    expect(caret).not.toBeNull();
    expect(caret?.className).not.toContain("opacity-0");
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
    // Reset drops the user's widths and the table re-measures its natural ones
    // (2 columns x the 120px jsdom fallback) rather than falling back to
    // automatic layout, which no table uses any more.
    fireEvent.doubleClick(handle);
    expect(header?.closest("table")?.style.width).toBe("240px");
  });

  it("right-aligns an end-aligned column's header and cells (design correction)", () => {
    // jsdom applies no stylesheet, so the driving attribute is asserted rather
    // than a computed style: `data-align="end"` is what `kit.css` hangs both
    // `text-align: end` and the header's `justify-content: flex-end` off of.
    const rows = [{ name: "web-1", restarts: 3 }];
    const alignedColumns: Column<(typeof rows)[number]>[] = [
      { key: "name", header: "Name" },
      { key: "restarts", header: "Restarts", align: "end" },
    ];
    render(<Table columns={alignedColumns} data={rows} getRowKey={(r) => r.name} />);

    const th = screen.getByText("Restarts").closest("th");
    expect(th?.getAttribute("data-align")).toBe("end");

    const td = screen.getByText("3").closest("td");
    expect(td?.getAttribute("data-align")).toBe("end");
  });

  it("leaves a column start-aligned when `align` is unset", () => {
    render(<Table columns={columns} data={data} getRowKey={(r) => r.name} />);
    const th = screen.getByText("Name").closest("th");
    expect(th?.hasAttribute("data-align")).toBe(false);
    const td = screen.getByText("web-1").closest("td");
    expect(td?.hasAttribute("data-align")).toBe(false);
  });

  it("leaves the checkbox column's own centring alone even when a data column is end-aligned", () => {
    const rows = [{ name: "web-1", restarts: 3 }];
    const alignedColumns: Column<(typeof rows)[number]>[] = [
      { key: "name", header: "Name" },
      { key: "restarts", header: "Restarts", align: "end" },
    ];
    const onChange = vi.fn();
    render(
      <Table
        columns={alignedColumns}
        data={rows}
        getRowKey={(r) => r.name}
        selection={{ selected: new Set(), onChange }}
      />,
    );
    const checkboxHeader = screen.getByLabelText("Select all").closest("th");
    expect(checkboxHeader?.className).toContain("tbl-check");
    expect(checkboxHeader?.hasAttribute("data-align")).toBe(false);
  });
});

/**
 * Group headings inside the body, and the one rule that makes them honest.
 *
 * A grouping describes the ORDER the caller handed the table, so the moment a
 * header sort reorders the list the headings would be labelling rows that no
 * longer sit under them. They are dropped for as long as a sort is active and
 * come back when it is cleared — a heading that silently became wrong under
 * sort is worse than no heading at all.
 */
/**
 * A column pinned to the end of the table.
 *
 * **jsdom has no layout, so this is the plumbing and nothing more**: that the
 * attribute the stylesheet pins on reaches both the header cell and every body
 * cell of that column, and no other. Whether the cell actually stays on screen
 * is a browser fact, measured in one (see the task report), and no assertion
 * here can stand in for it.
 */
describe("Table sticky columns", () => {
  const cols: Column<{ name: string }>[] = [
    { key: "name", header: "Name" },
    { key: "actions", header: "", sticky: "end" },
  ];
  const rows = [{ name: "a" }, { name: "b" }];

  it("marks the pinned column's header and every one of its cells", () => {
    const { container } = render(<Table columns={cols} data={rows} getRowKey={(r) => r.name} />);
    const pinned = container.querySelectorAll('[data-sticky="end"]');
    // One `th`, one `td` per row — and nothing else in the table.
    expect([...pinned].map((el) => el.tagName)).toEqual(["TH", "TD", "TD"]);
  });

  it("leaves every other column unmarked", () => {
    const { container } = render(<Table columns={cols} data={rows} getRowKey={(r) => r.name} />);
    const first = container.querySelector("tbody tr td");
    expect(first?.hasAttribute("data-sticky")).toBe(false);
    expect(container.querySelector("thead th")?.hasAttribute("data-sticky")).toBe(false);
  });

  it("marks nothing when no column asks to be pinned", () => {
    const { container } = render(
      <Table columns={[{ key: "name", header: "Name" }]} data={rows} getRowKey={(r) => r.name} />,
    );
    expect(container.querySelectorAll("[data-sticky]")).toHaveLength(0);
  });
});

describe("Table row groups", () => {
  interface Cluster {
    name: string;
    source: string;
    latency: number;
  }

  const cols: Column<Cluster>[] = [
    { key: "name", header: "Name" },
    { key: "source", header: "Source" },
    { key: "latency", header: "Latency" },
  ];

  // Two groups, four rows, and the caller's order already groups them: the
  // table's job here is to draw the boundary, not to find it.
  const rows: Cluster[] = [
    { name: "prod-eu", source: "Kubeconfig", latency: 41 },
    { name: "staging", source: "Kubeconfig", latency: 12 },
    { name: "kind-dev", source: "Local", latency: 3 },
    { name: "k3d-lab", source: "Local", latency: 1 },
  ];

  const group = (row: Cluster) => ({ key: row.source, label: `${row.source} group` });

  function renderGrouped() {
    return render(
      <Table columns={cols} data={rows} getRowKey={(r) => r.name} rowGroup={group} />,
    );
  }

  const headings = (container: HTMLElement) =>
    [...container.querySelectorAll('[data-slot="table-group"]')].map((tr) => tr.textContent);

  it("heads each group once, in the caller's own order", () => {
    const { container } = renderGrouped();
    expect(headings(container)).toEqual(["Kubeconfig group", "Local group"]);
  });

  it("puts each heading directly above the first row of its group", () => {
    const { container } = renderGrouped();
    const bodyRows = [...container.querySelectorAll("tbody tr")].map((tr) => [
      tr.getAttribute("data-slot") === "table-group" ? "GROUP" : "row",
      tr.textContent,
    ]);
    expect(bodyRows).toEqual([
      ["GROUP", "Kubeconfig group"],
      ["row", "prod-euKubeconfig41"],
      ["row", "stagingKubeconfig12"],
      ["GROUP", "Local group"],
      ["row", "kind-devLocal3"],
      ["row", "k3d-labLocal1"],
    ]);
  });

  it("spans every column, the checkbox one included", () => {
    const { container } = render(
      <Table
        columns={cols}
        data={rows}
        getRowKey={(r) => r.name}
        rowGroup={group}
        selection={{ selected: new Set<string>(), onChange: vi.fn() }}
      />,
    );
    const heading = container.querySelector('[data-slot="table-group"] th');
    expect(heading?.getAttribute("colspan")).toBe("4");
  });

  it("draws no heading at all without a rowGroup", () => {
    const { container } = render(<Table columns={cols} data={rows} getRowKey={(r) => r.name} />);
    expect(headings(container)).toEqual([]);
  });

  it("drops every heading while a sort is active, because the order it described is gone", () => {
    const { container } = renderGrouped();
    fireEvent.click(screen.getByRole("button", { name: "Sort by Latency" }));

    // The rows are reordered across the groups — so the headings have to go.
    expect(
      [...container.querySelectorAll("tbody tr.tbl-row")].map((tr) => tr.textContent),
    ).toEqual(["k3d-labLocal1", "kind-devLocal3", "stagingKubeconfig12", "prod-euKubeconfig41"]);
    expect(headings(container)).toEqual([]);
  });

  it("brings them back when the reader clears the sort", () => {
    const { container } = renderGrouped();
    const sort = screen.getByRole("button", { name: "Sort by Latency" });
    fireEvent.click(sort); // ascending
    fireEvent.click(sort); // descending
    expect(headings(container)).toEqual([]);
    fireEvent.click(sort); // cleared — the caller's order is back
    expect(headings(container)).toEqual(["Kubeconfig group", "Local group"]);
  });

  it("keeps a heading out of the rows a reader can reach", () => {
    const { container } = render(
      <Table
        columns={cols}
        data={rows}
        getRowKey={(r) => r.name}
        rowGroup={group}
        onRowActivate={vi.fn()}
      />,
    );
    const heading = container.querySelector('[data-slot="table-group"]');
    // No tab stop, no `tbl-row`: the arrow keys and Enter walk the data rows.
    expect(heading?.hasAttribute("tabindex")).toBe(false);
    expect(heading?.className).not.toContain("tbl-row");
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

describe("row gestures", () => {
  const ROWS = [
    { id: "a", name: "alpha" },
    { id: "b", name: "beta" },
    { id: "c", name: "gamma" },
  ];
  const COLS = [{ key: "name", header: "Name" }];
  const table = (props: Record<string, unknown>) =>
    render(<Table columns={COLS} data={ROWS} getRowKey={(r) => r.id} {...props} />);

  it("activates a row on double-click", () => {
    const onRowActivate = vi.fn();
    table({ onRowActivate });
    fireEvent.doubleClick(screen.getByText("beta").closest("tr")!);
    expect(onRowActivate).toHaveBeenCalledWith(ROWS[1]);
  });

  it("activates the focused row on Enter, so opening a resource is not pointer-only", () => {
    const onRowActivate = vi.fn();
    table({ onRowActivate });
    const row = screen.getByText("alpha").closest("tr")!;
    row.focus();
    fireEvent.keyDown(row, { key: "Enter" });
    expect(onRowActivate).toHaveBeenCalledWith(ROWS[0]);
  });

  it("carries one tab stop and moves it with the arrows", () => {
    table({ onRowActivate: vi.fn() });
    const rows = screen.getAllByRole("row").filter((r) => r.hasAttribute("tabindex"));
    expect(rows.filter((r) => r.getAttribute("tabindex") === "0")).toHaveLength(1);
    fireEvent.keyDown(rows[0], { key: "ArrowDown" });
    expect(rows[1].getAttribute("tabindex")).toBe("0");
    expect(rows[0].getAttribute("tabindex")).toBe("-1");
    expect(document.activeElement).toBe(rows[1]);
  });

  it("starts the tab stop on the selected row, not the first", () => {
    table({ onRowActivate: vi.fn(), selectedKey: "c" });
    expect(screen.getByText("gamma").closest("tr")!.getAttribute("tabindex")).toBe("0");
    expect(screen.getByText("alpha").closest("tr")!.getAttribute("tabindex")).toBe("-1");
  });

  it("falls back to a rendered row when selectedKey names one that isn't rendered", () => {
    // A selected row scrolled out of the virtualised window (stood in here by
    // a selectedKey matching nothing in `data`) must not leave the table with
    // zero tab stops — the stop falls back to the first rendered row instead.
    table({ onRowActivate: vi.fn(), selectedKey: "not-a-real-row" });
    const rows = screen.getAllByRole("row").filter((r) => r.hasAttribute("tabindex"));
    expect(rows.filter((r) => r.getAttribute("tabindex") === "0")).toHaveLength(1);
    expect(screen.getByText("alpha").closest("tr")!.getAttribute("tabindex")).toBe("0");
  });

  it("leaves rows unfocusable when neither gesture is supplied", () => {
    table({ onRowClick: vi.fn() });
    // No jest-dom in this package: `hasAttribute` is the vanilla equivalent
    // of `.not.toHaveAttribute("tabindex")`.
    expect(screen.getByText("alpha").closest("tr")!.hasAttribute("tabindex")).toBe(false);
  });

  it("opens a per-row menu on right-click, built from that row", async () => {
    const onPick = vi.fn();
    table({ rowMenu: (r: { name: string }) => [{ label: `Delete ${r.name}`, onPick }] });
    fireEvent.contextMenu(screen.getByText("beta").closest("tr")!);
    await userEvent.click(await screen.findByText("Delete beta"));
    expect(onPick).toHaveBeenCalled();
  });
});
