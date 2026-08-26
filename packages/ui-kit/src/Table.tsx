import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { cx } from "./cx";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";
import { EmptyState } from "./EmptyState";


/* Inline rather than an icon-set import: the kit takes no dependency on
   lucide, and these four are the only glyphs it needs. */
const glyph = { width: 12, height: 12, viewBox: "0 0 24 24", fill: "none", "aria-hidden": true } as const;

function ArrowUpDown() {
  return (
    <svg {...glyph}>
      <path d="m7 15 5 5 5-5M7 9l5-5 5 5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function ArrowUp() {
  return (
    <svg {...glyph}>
      <path d="M12 19V5m-7 7 7-7 7 7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function ArrowDown() {
  return (
    <svg {...glyph}>
      <path d="M12 5v14m7-7-7 7-7-7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function Filter() {
  return (
    <svg {...glyph}>
      <path d="M3 5h18l-7 8v6l-4 2v-8Z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
    </svg>
  );
}

/**
 * Which column a table is sorted by, and which way.
 *
 * Declared here rather than imported: `@srelens/core` has the same shape, for
 * the tab view state it serializes into the session file, and the kit may not
 * reach the service layer. Structurally identical, so the app hands core's
 * straight in with no adapter. Two declarations until step 11 deletes the
 * classic table that still imports core's. (#319)
 */
export interface TableSort {
  key: string;
  direction: "asc" | "desc";
}

/**
 * The sort a header click produces: a new column starts ascending, the same
 * column reverses, and a third click clears it. Cycling through "no sort" is
 * what lets a user get back to the server's own order without reloading.
 */
export function nextSort(current: TableSort | null | undefined, key: string): TableSort | null {
  if (!current || current.key !== key) return { key, direction: "asc" };
  if (current.direction === "asc") return { key, direction: "desc" };
  return null;
}

export interface Column<T> {
  key: string;
  header: ReactNode;
  /** Render the cell for a row; defaults to `String(row[key])`. */
  render?: (row: T) => ReactNode;
  /** Value used for sorting and filtering when it differs from `row[key]`. */
  getValue?: (row: T) => unknown;
  /** Sort-only value, overriding `getValue`/`row[key]` for the comparator —
   *  for columns whose display text doesn't order correctly (e.g. compact
   *  ages, where "1y" must outrank "300d") while filtering stays on the
   *  visible text. */
  getSortValue?: (row: T) => unknown;
  /** Whether clicking the header sorts this column; defaults to true. Every
   *  sortable column stays clickable regardless of which one is the active
   *  sort — this only gates the header button existing at all. */
  sortable?: boolean;
  /** Dual role with opposite defaults:
   *  (1) Opt-in for the header filter funnel button: `filterable === true` shows
   *      a funnel that scopes the toolbar search to this column alone.
   *  (2) Opt-out for filterTableData's free-text search scope: `filterable !== false`
   *      includes this column in the default search-all-columns path (when no
   *      funnel is active). An undefined column still searches; only explicit
   *      `filterable: false` excludes it entirely from searches.
   *  Most columns don't need a funnel; off by default for that reason. */
  filterable?: boolean;
  /** Header and cell alignment. Logical values, not "left"/"right": the table
   *  renders in right-to-left locales eventually, where `end` does the right
   *  thing and `right` would not. Defaults to `start` — set `end` for numeric
   *  columns (READY, RESTARTS, CPU, MEMORY, AGE) so their digits line up. */
  align?: "start" | "end";
  minWidth?: number;
}

/** Opt-in multi-selection: the parent holds the set of selected row keys, the
 *  Table owns the interaction (toggle, shift-click range over the sorted order,
 *  select-all-of-filtered) and reports the new set. */
export interface TableSelection {
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
}

export interface TableProps<T> {
  columns: Column<T>[];
  data: T[];
  getRowKey: (row: T) => string;
  onRowClick?: (row: T) => void;
  /** Row key currently selected (highlighted). */
  selectedKey?: string;
  /** When set, renders a leading checkbox column for bulk selection. */
  selection?: TableSelection;
  /** Shown when `data` is empty. */
  emptyText?: ReactNode;
  /** Second line of the empty state: what the reader can do about it. */
  emptyHint?: ReactNode;
  /** Column currently used by the toolbar search; null searches every column. */
  activeFilterKey?: string | null;
  onActiveFilterKeyChange?: (key: string | null) => void;
  /** Controlled sort. Supply both to own it (so it can live on the tab and
   *  survive a switch, #254); omit them and the table keeps its own. */
  sort?: TableSort | null;
  onSortChange?: (sort: TableSort | null) => void;
  /**
   * The "open this properly" gesture — double-click, or Enter on the focused
   * row. Not `onDoubleClick`: a pointer-only route to opening a row is the
   * fault this kit refuses everywhere else, so the keyboard half is part of
   * the prop rather than the caller's problem.
   */
  onRowActivate?: (row: T) => void;
  /**
   * The row's context menu. The kit owns the `<tr>`, so the kit owns the menu
   * wrapped around it; a caller cannot reach between the table and its rows.
   */
  rowMenu?: (row: T) => ContextMenuItem[];
  /** Names each row's menu for assistive technology, e.g. "Pod actions". */
  rowMenuLabel?: string;
}

/** Width of the leading bulk-selection column; mirrored in styles.css. */
const CHECKBOX_COLUMN_WIDTH = 36;

function getColumnValue<T>(row: T, column: Column<T>): unknown {
  return column.getValue ? column.getValue(row) : (row as Record<string, unknown>)[column.key];
}

/**
 * The slice of rows to render for a virtualized list. Returns the full range
 * when `rowHeight` is unknown (0) — e.g. before measurement or in jsdom — so the
 * table degrades to rendering everything rather than dividing by zero.
 */
export function computeVisibleRange({
  scrollTop,
  viewportHeight,
  rowHeight,
  total,
  overscan,
}: {
  scrollTop: number;
  viewportHeight: number;
  rowHeight: number;
  total: number;
  overscan: number;
}): { start: number; end: number } {
  if (rowHeight <= 0) return { start: 0, end: total };
  const firstVisible = Math.floor(scrollTop / rowHeight);
  const visibleCount = Math.ceil(viewportHeight / rowHeight);
  const end = Math.min(total, firstVisible + visibleCount + overscan);
  const start = Math.min(Math.max(0, firstVisible - overscan), end);
  return { start, end };
}

/**
 * How far down the list one row advances it — the unit the spacer rows reserve
 * space in.
 *
 * Taken from the distance between two *interior* rows, not from the height of
 * the first rendered one, which is what this used to read and is a different
 * number. The table is `border-collapse: collapse`, so every rule is shared
 * between the two rows it separates; `.tbl-spacer` declares `border: 0`, so the
 * row directly under the top spacer has no rule to share and comes out shorter
 * than the rest. MEASURED in Chrome on a 1500-row pod list: 27.000 for that one
 * row against 27.375 for every other, at every scroll position.
 *
 * `topPad`/`bottomPad` multiply this by the number of rows the window skips, so
 * sampling the short row scaled a 0.375px error to 545px of `scrollHeight`, and
 * the extent flipped between the two values as the sampled row moved in and out
 * of the spacer's shadow — the scrollbar resizing under the reader's thumb on
 * every scroll, which is what "scroll is not smooth" was.
 *
 * `tops` is the top of each of the first few rendered rows, in order.
 * `fallbackHeight` is the first row's own height, used when there are too few
 * rows to take a distance from, and when the tops are not real numbers — which
 * is jsdom, where every rect is zeroed and the table renders every row anyway.
 */
export function rowPitch(tops: readonly number[], fallbackHeight: number): number {
  // tops[2] - tops[1], never tops[1] - tops[0]: the first gap is the short one.
  const pitch = tops.length >= 3 ? tops[2] - tops[1] : NaN;
  return Number.isFinite(pitch) && pitch > 0 ? pitch : fallbackHeight;
}

/** Apply the toolbar query to one selected column, or all searchable columns. */
export function filterTableData<T>(
  data: T[],
  columns: Column<T>[],
  query: string,
  activeFilterKey: string | null,
): T[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return data;
  const searchable = activeFilterKey
    ? columns.filter((column) => column.key === activeFilterKey)
    : columns.filter((column) => column.filterable !== false);
  if (searchable.length === 0) return data;
  return data.filter((row) =>
    searchable.some((column) =>
      String(getColumnValue(row, column) ?? "").toLocaleLowerCase().includes(normalized),
    ),
  );
}

/** Generic data table used for every resource list in the app (shadcn Table). */
export function Table<T>({
  columns,
  data,
  getRowKey,
  onRowClick,
  selectedKey,
  selection,
  emptyText = "No items",
  emptyHint,
  activeFilterKey = null,
  onActiveFilterKeyChange,
  sort: controlledSort,
  onSortChange,
  onRowActivate,
  rowMenu,
  rowMenuLabel,
}: TableProps<T>) {
  // Controlled when a change handler is supplied, otherwise self-managed —
  // tables outside the tabbed workspace (the MCP audit list, for instance)
  // have no tab to store a sort on.
  const [internalSort, setInternalSort] = useState<TableSort | null>(null);
  const sort = onSortChange ? (controlledSort ?? null) : internalSort;
  // One tab stop for the table, moved by the arrows: the WAI-ARIA grid
  // pattern. A stop per row would put hundreds of them between the filter bar
  // and whatever follows the table.
  const interactive = Boolean(onRowActivate || rowMenu);
  const rowRefs = useRef(new Map<string, HTMLTableRowElement>());
  const [focusKey, setFocusKey] = useState<string | null>(null);
  // Anchor for shift-click range selection (a key in sorted/visible order).
  const selectionAnchor = useRef<string | null>(null);
  const [columnWidths, setColumnWidths] = useState<Record<string, number>>({});
  // Whether `columnWidths` was measured for the user (#298) rather than chosen
  // by them. Auto-sized widths re-measure when the table is resized; widths the
  // user dragged are theirs to keep.
  const autoSized = useRef(false);
  const containerWidth = useRef(0);
  const columnSignature = columns.map((column) => column.key).join("|");
  const rootRef = useRef<HTMLDivElement>(null);
  const [metrics, setMetrics] = useState({ scrollTop: 0, viewportHeight: 0, rowHeight: 0 });

  useEffect(() => setColumnWidths({}), [columnSignature]);

  const visibleData = useMemo(() => {
    if (!sort) return data;
    const column = columns.find((candidate) => candidate.key === sort.key);
    if (!column) return data;
    const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
    return data
      .map((row, index) => ({ row, index }))
      .sort((a, b) => {
        const left = column.getSortValue ? column.getSortValue(a.row) : getColumnValue(a.row, column);
        const right = column.getSortValue ? column.getSortValue(b.row) : getColumnValue(b.row, column);
        let result: number;
        if (typeof left === "number" && typeof right === "number") result = left - right;
        else if (typeof left === "bigint" || typeof right === "bigint") {
          // Compare, never subtract: bigint arithmetic with a number throws,
          // and columns mix the two (a bigint value against a -Infinity for
          // "unset"). Relational operators are defined across both and stay
          // exact past Number.MAX_SAFE_INTEGER, which string collation is not:
          // it orders negatives by magnitude.
          const l = left as bigint | number;
          const r = right as bigint | number;
          result = l < r ? -1 : l > r ? 1 : 0;
        } else result = collator.compare(String(left ?? ""), String(right ?? ""));
        return result ? result * (sort.direction === "asc" ? 1 : -1) : a.index - b.index;
      })
      .map(({ row }) => row);
  }, [columns, data, sort]);

  // Selection: keys in the order the user sees them (for shift-range + select-all).
  const visibleKeys = useMemo(() => visibleData.map(getRowKey), [visibleData, getRowKey]);
  const colCount = columns.length + (selection ? 1 : 0);
  const allVisibleSelected =
    !!selection && visibleKeys.length > 0 && visibleKeys.every((k) => selection.selected.has(k));

  const toggleAllVisible = () => {
    if (!selection) return;
    const next = new Set(selection.selected);
    if (allVisibleSelected) visibleKeys.forEach((k) => next.delete(k));
    else visibleKeys.forEach((k) => next.add(k));
    selection.onChange(next);
  };

  const toggleRow = (key: string, shift: boolean) => {
    if (!selection) return;
    const next = new Set(selection.selected);
    const anchor = selectionAnchor.current;
    const from = anchor ? visibleKeys.indexOf(anchor) : -1;
    const to = visibleKeys.indexOf(key);
    if (shift && from >= 0 && to >= 0) {
      const [lo, hi] = from < to ? [from, to] : [to, from];
      for (let i = lo; i <= hi; i++) next.add(visibleKeys[i]);
    } else if (next.has(key)) {
      next.delete(key);
    } else {
      next.add(key);
    }
    selectionAnchor.current = key;
    selection.onChange(next);
  };

  const cycleSort = (key: string) => {
    const next = nextSort(sort, key);
    if (onSortChange) onSortChange(next);
    else setInternalSort(next);
  };

  /** Current on-screen width of every column, for pinning into `columnWidths`. */
  const measureColumns = (table: Element | null | undefined): Record<string, number> => {
    const headers = table?.querySelectorAll<HTMLTableCellElement>("thead tr:first-child th");
    // The checkbox column (when present) is th[0], so data columns are offset.
    const headOffset = selection ? 1 : 0;
    return Object.fromEntries(
      columns.map((candidate, index) => [
        candidate.key,
        Math.round(
          headers?.[index + headOffset]?.getBoundingClientRect().width ||
            columnWidths[candidate.key] ||
            120,
        ),
      ]),
    );
  };

  const startColumnResize = (event: React.PointerEvent<HTMLDivElement>, column: Column<T>) => {
    event.preventDefault();
    event.stopPropagation();
    const measured = measureColumns(event.currentTarget.closest("table"));
    autoSized.current = false;
    const startWidth = measured[column.key];
    const startX = event.clientX;
    setColumnWidths(measured);

    const onMove = (moveEvent: PointerEvent) => {
      const minWidth = column.minWidth ?? 72;
      setColumnWidths((current) => ({
        ...current,
        [column.key]: Math.max(minWidth, startWidth + moveEvent.clientX - startX),
      }));
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      document.body.classList.remove("fl-is-resizing-column");
    };
    document.body.classList.add("fl-is-resizing-column");
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const resizeColumnWithKeyboard = (
    event: React.KeyboardEvent<HTMLDivElement>,
    column: Column<T>,
  ) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const measured = measureColumns(event.currentTarget.closest("table"));
    autoSized.current = false;
    const delta = event.key === "ArrowRight" ? 16 : -16;
    setColumnWidths((current) => {
      const baseline = Object.keys(current).length ? current : measured;
      return {
        ...baseline,
        [column.key]: Math.max(column.minWidth ?? 72, baseline[column.key] + delta),
      };
    });
  };

  // The checkbox column is fixed-width and never resizable, but it still takes
  // up space: leaving it out made the declared table width narrower than the
  // colgroup asks for, so fixed layout had to steal the difference back from
  // the data columns.
  const tableWidth = Object.keys(columnWidths).length
    ? Object.values(columnWidths).reduce((total, width) => total + width, 0) +
      (selection ? CHECKBOX_COLUMN_WIDTH : 0)
    : undefined;

  // Virtualize long lists: render only the rows near the viewport plus spacer
  // rows that reserve the scroll height. Below the threshold — or before the row
  // height can be measured (jsdom, first paint) — render everything.
  const VIRTUALIZE_THRESHOLD = 60;
  const OVERSCAN = 8;

  useEffect(() => {
    const root = rootRef.current;
    if (!root || visibleData.length <= VIRTUALIZE_THRESHOLD) return;
    let scrollParent: HTMLElement | null = root.parentElement;
    while (scrollParent && scrollParent !== document.body) {
      const overflowY = getComputedStyle(scrollParent).overflowY;
      if (overflowY === "auto" || overflowY === "scroll") break;
      scrollParent = scrollParent.parentElement;
    }
    if (!scrollParent) return;
    const parent = scrollParent;
    const measure = () => {
      // Three rects, not one: `rowPitch` needs two interior tops, and three is
      // as many forced layouts as this is allowed to cost on a scroll tick.
      const sample = Array.from(root.querySelectorAll<HTMLElement>("tbody tr.tbl-row"))
        .slice(0, 3)
        .map((row) => row.getBoundingClientRect());
      setMetrics({
        scrollTop: parent.scrollTop,
        viewportHeight: parent.clientHeight,
        rowHeight: sample.length
          ? rowPitch(
              sample.map((rect) => rect.top),
              sample[0].height,
            )
          : 0,
      });
    };
    measure();
    parent.addEventListener("scroll", measure, { passive: true });
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null;
    observer?.observe(parent);
    return () => {
      parent.removeEventListener("scroll", measure);
      observer?.disconnect();
    };
  }, [visibleData.length]);

  const virtualize = visibleData.length > VIRTUALIZE_THRESHOLD && metrics.rowHeight > 0;
  const range = virtualize
    ? computeVisibleRange({
        scrollTop: metrics.scrollTop,
        viewportHeight: metrics.viewportHeight,
        rowHeight: metrics.rowHeight,
        total: visibleData.length,
        overscan: OVERSCAN,
      })
    : { start: 0, end: visibleData.length };
  const windowRows = virtualize ? visibleData.slice(range.start, range.end) : visibleData;
  const topPad = virtualize ? range.start * metrics.rowHeight : 0;
  const bottomPad = virtualize ? (visibleData.length - range.end) * metrics.rowHeight : 0;

  // The stop prefers the focused/selected row, but only when that row is
  // actually rendered — a selected row scrolled out of the virtualised window
  // must not strand the table with zero tab stops. Falls back to the first
  // rendered row, so there is always exactly one stop whenever there is a row
  // to hold it.
  const windowKeys = windowRows.map(getRowKey);
  const preferredKey = focusKey ?? selectedKey ?? null;
  const stopKey =
    preferredKey && windowKeys.includes(preferredKey) ? preferredKey : (windowKeys[0] ?? null);

  function onRowKeyDown(event: ReactKeyboardEvent<HTMLTableRowElement>, row: T) {
    if (event.key === "Enter" && onRowActivate) {
      event.preventDefault();
      onRowActivate(row);
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const keys = windowKeys;
    const index = keys.indexOf(getRowKey(row));
    if (index < 0) return;
    const next = event.key === "ArrowDown" ? index + 1 : index - 1;
    if (next < 0 || next >= keys.length) return;
    // Otherwise the table scrolls under the focus it just moved.
    event.preventDefault();
    setFocusKey(keys[next]);
    rowRefs.current.get(keys[next])?.focus();
  }

  const isEmpty = data.length === 0;

  // Pin the natural column widths once the table has rows to measure.
  //
  // Automatic table layout sizes columns from the rows *currently rendered*.
  // Virtualization swaps that set on every scroll, so a long list's columns
  // visibly shifted as the user scrolled (#298). Measuring in a layout effect
  // keeps it off-screen: the widths are read and applied before paint.
  //
  // This deliberately applies to every table, not just the ones long enough to
  // virtualize. Pinning only above the threshold made short lists -- CRDs,
  // Services, most views -- size by a different rule from long ones, and left a
  // list that was filtered down past the threshold stranded with widths it
  // could neither keep consistently nor recompute. Sizing every table the same
  // way is both uniform and simpler.
  useLayoutEffect(() => {
    if (isEmpty || Object.keys(columnWidths).length > 0) return;
    const table = rootRef.current?.querySelector("table");
    // Nothing to measure until a real row exists: a table showing only the
    // "no matching items" placeholder would freeze the placeholder's widths.
    if (!table?.querySelector("tbody tr.tbl-row")) return;
    setColumnWidths(measureColumns(table));
    autoSized.current = true;
    // measureColumns reads `columns`/`selection`, which `columnSignature` tracks.
  }, [isEmpty, visibleData.length, columnWidths, columnSignature]);

  // Pinned widths would otherwise survive a window resize, so an auto-sized
  // table would stop tracking the space available to it. Drop the measurement
  // when the container's width changes and the effect above re-takes it at the
  // new size. Widths the user dragged are left alone.
  useEffect(() => {
    if (isEmpty) return;
    const container = rootRef.current?.querySelector<HTMLElement>('[data-slot="table-container"]');
    if (!container || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (container.clientWidth === containerWidth.current) return;
      containerWidth.current = container.clientWidth;
      if (autoSized.current) setColumnWidths({});
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [isEmpty]);

  if (isEmpty) {
    return <EmptyState title={emptyText} hint={emptyHint} />;
  }
  return (
    <div ref={rootRef} style={{ display: "contents" }}>
    <table
      className={cx(
        "tbl",
        tableWidth ? "tbl-resized" : null,
        onRowClick && "tbl-interactive",
      )}
      style={tableWidth ? { width: tableWidth, minWidth: "100%" } : undefined}
    >
      <colgroup>
        {selection && <col style={{ width: CHECKBOX_COLUMN_WIDTH }} />}
        {columns.map((column) => (
          <col
            key={column.key}
            style={columnWidths[column.key] ? { width: columnWidths[column.key] } : undefined}
          />
        ))}
      </colgroup>
      <thead className="sticky top-0 z-10">
        <tr className="hover:bg-transparent">
          {selection && (
            <th className="tbl-check">
              <input
                type="checkbox"
                aria-label="Select all"
                checked={allVisibleSelected}
                ref={(el) => {
                  if (el) el.indeterminate = !allVisibleSelected && visibleKeys.some((k) => selection.selected.has(k));
                }}
                onChange={toggleAllVisible}
              />
            </th>
          )}
          {columns.map((c) => (
            <th key={c.key} data-align={c.align === "end" ? "end" : undefined}>
              <div className="th-head">
                <button
                  type="button"
                  className="th-sort group"
                  onClick={() => cycleSort(c.key)}
                  disabled={c.sortable === false}
                  aria-label={`Sort by ${typeof c.header === "string" ? c.header : c.key}`}
                  data-on={c.sortable !== false && sort?.key === c.key}
                >
                  <span>{c.header}</span>
                  {c.sortable !== false && (
                    sort?.key === c.key ? (
                      <span className="th-caret">
                        {sort.direction === "asc" ? <ArrowUp /> : <ArrowDown />}
                      </span>
                    ) : (
                      // Hidden at rest — the design shows a caret only on the
                      // active sort column — but revealed on hover/keyboard
                      // focus so the button stays discoverable without it.
                      // Reveal is driven by Tailwind utilities (group-hover:opacity-100,
                      // group-focus-visible:opacity-100) that beat the .th-caret component
                      // layer rule (opacity: 0) because Tailwind's utilities layer is
                      // declared after kit.css's @layer components. If .th-caret ever
                      // leaves the component layer or gains !important, this reveal breaks
                      // silently — the affordance disappears for keyboard and pointer users.
                      <span className="th-caret opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100">
                        <ArrowUpDown />
                      </span>
                    )
                  )}
                </button>
                {c.filterable === true && onActiveFilterKeyChange && (
                  <button
                    type="button"
                    className={cx("th-filter", activeFilterKey === c.key && "is-active")}
                    onClick={() => onActiveFilterKeyChange(activeFilterKey === c.key ? null : c.key)}
                    aria-label={`Filter search by ${typeof c.header === "string" ? c.header : c.key}`}
                    aria-pressed={activeFilterKey === c.key}
                  >
                    <Filter />
                  </button>
                )}
              </div>
              <div
                role="separator"
                aria-orientation="vertical"
                aria-label={`Resize ${typeof c.header === "string" ? c.header : c.key} column`}
                tabIndex={0}
                className="th-resize"
                onPointerDown={(event) => startColumnResize(event, c)}
                onKeyDown={(event) => resizeColumnWithKeyboard(event, c)}
                onDoubleClick={() => setColumnWidths({})}
                title="Drag to resize; double-click to reset"
              />
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {topPad > 0 && (
          <tr aria-hidden="true" className="tbl-spacer">
            <td colSpan={colCount} style={{ height: topPad, padding: 0, border: 0 }} />
          </tr>
        )}
        {windowRows.map((row) => {
          const rowKey = getRowKey(row);
          const selected = selectedKey === rowKey;
          const checked = selection?.selected.has(rowKey) ?? false;
          const body = (
            <tr
              key={rowKey}
              ref={(node) => {
                if (node) rowRefs.current.set(rowKey, node);
                else rowRefs.current.delete(rowKey);
              }}
              aria-selected={selected}
              data-state={selected || checked ? "selected" : undefined}
              tabIndex={interactive ? (rowKey === stopKey ? 0 : -1) : undefined}
              onFocus={interactive ? () => setFocusKey(rowKey) : undefined}
              onKeyDown={interactive ? (e) => onRowKeyDown(e, row) : undefined}
              onClick={onRowClick ? () => onRowClick(row) : undefined}
              onDoubleClick={onRowActivate ? () => onRowActivate(row) : undefined}
              className={cx("tbl-row", (onRowClick || interactive) && "cursor-pointer")}
            >
              {selection && (
                <td
                  className="tbl-check"
                  onClick={(e) => e.stopPropagation()}
                >
                  <input
                    type="checkbox"
                    aria-label={`Select ${rowKey}`}
                    checked={checked}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => toggleRow(rowKey, (e.nativeEvent as MouseEvent).shiftKey)}
                  />
                </td>
              )}
              {columns.map((c) => (
                <td key={c.key} data-align={c.align === "end" ? "end" : undefined}>
                  {c.render ? c.render(row) : String((row as Record<string, unknown>)[c.key])}
                </td>
              ))}
            </tr>
          );
          return rowMenu ? (
            <ContextMenu key={rowKey} items={rowMenu(row)} label={rowMenuLabel}>
              {body}
            </ContextMenu>
          ) : (
            body
          );
        })}
        {bottomPad > 0 && (
          <tr aria-hidden="true" className="tbl-spacer">
            <td colSpan={colCount} style={{ height: bottomPad, padding: 0, border: 0 }} />
          </tr>
        )}
        {visibleData.length === 0 && (
          <tr>
            <td colSpan={colCount} className="tbl-empty">
              No matching items
            </td>
          </tr>
        )}
      </tbody>
    </table>
    </div>
  );
}
