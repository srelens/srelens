import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowUp, ArrowUpDown, Filter } from "lucide-react";
import {
  Table as ShadTable,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { cn } from "@/ui/utils";
import { EmptyState } from "./Dashboard";
import { nextSort, type TableSort } from "@srelens/core";

export interface Column<T> {
  key: string;
  header: React.ReactNode;
  /** Render the cell for a row; defaults to `String(row[key])`. */
  render?: (row: T) => React.ReactNode;
  /** Value used for sorting and filtering when it differs from `row[key]`. */
  getValue?: (row: T) => unknown;
  /** Sort-only value, overriding `getValue`/`row[key]` for the comparator —
   *  for columns whose display text doesn't order correctly (e.g. compact
   *  ages, where "1y" must outrank "300d") while filtering stays on the
   *  visible text. */
  getSortValue?: (row: T) => unknown;
  sortable?: boolean;
  filterable?: boolean;
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
  emptyText?: React.ReactNode;
  /** Second line of the empty state: what the reader can do about it. */
  emptyHint?: React.ReactNode;
  /** Column currently used by the toolbar search; null searches every column. */
  activeFilterKey?: string | null;
  onActiveFilterKeyChange?: (key: string | null) => void;
  /** Controlled sort. Supply both to own it (so it can live on the tab and
   *  survive a switch, #254); omit them and the table keeps its own. */
  sort?: TableSort | null;
  onSortChange?: (sort: TableSort | null) => void;
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
}: TableProps<T>) {
  // Controlled when a change handler is supplied, otherwise self-managed —
  // tables outside the tabbed workspace (the MCP audit list, for instance)
  // have no tab to store a sort on.
  const [internalSort, setInternalSort] = useState<TableSort | null>(null);
  const sort = onSortChange ? (controlledSort ?? null) : internalSort;
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
      const firstRow = root.querySelector<HTMLElement>("tbody tr.fl-data-table__row");
      setMetrics({
        scrollTop: parent.scrollTop,
        viewportHeight: parent.clientHeight,
        rowHeight: firstRow ? firstRow.getBoundingClientRect().height : 0,
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
    if (!table?.querySelector("tbody tr.fl-data-table__row")) return;
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
    return <EmptyState title={emptyText} description={emptyHint} />;
  }
  return (
    <div ref={rootRef} style={{ display: "contents" }}>
    <ShadTable
      className={cn(
        "fl-data-table",
        tableWidth && "fl-data-table--resized",
        onRowClick && "fl-data-table--interactive",
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
      <TableHeader className="sticky top-0 z-10">
        <TableRow className="hover:bg-transparent">
          {selection && (
            <TableHead className="fl-data-table__head fl-data-table__checkbox">
              <input
                type="checkbox"
                aria-label="Select all"
                checked={allVisibleSelected}
                ref={(el) => {
                  if (el) el.indeterminate = !allVisibleSelected && visibleKeys.some((k) => selection.selected.has(k));
                }}
                onChange={toggleAllVisible}
              />
            </TableHead>
          )}
          {columns.map((c) => (
            <TableHead key={c.key} className="fl-data-table__head">
              <div className="fl-data-table__head-content">
                <button
                  type="button"
                  className="fl-data-table__sort"
                  onClick={() => cycleSort(c.key)}
                  disabled={c.sortable === false}
                  aria-label={`Sort by ${typeof c.header === "string" ? c.header : c.key}`}
                >
                  <span>{c.header}</span>
                  {c.sortable !== false &&
                    (sort?.key !== c.key ? (
                      <ArrowUpDown aria-hidden="true" />
                    ) : sort.direction === "asc" ? (
                      <ArrowUp aria-hidden="true" />
                    ) : (
                      <ArrowDown aria-hidden="true" />
                    ))}
                </button>
                {c.filterable !== false && onActiveFilterKeyChange && (
                  <button
                    type="button"
                    className={cn("fl-data-table__filter-toggle", activeFilterKey === c.key && "is-active")}
                    onClick={() => onActiveFilterKeyChange(activeFilterKey === c.key ? null : c.key)}
                    aria-label={`Filter search by ${typeof c.header === "string" ? c.header : c.key}`}
                    aria-pressed={activeFilterKey === c.key}
                  >
                    <Filter aria-hidden="true" />
                  </button>
                )}
              </div>
              <div
                role="separator"
                aria-orientation="vertical"
                aria-label={`Resize ${typeof c.header === "string" ? c.header : c.key} column`}
                tabIndex={0}
                className="fl-data-table__resize-handle"
                onPointerDown={(event) => startColumnResize(event, c)}
                onKeyDown={(event) => resizeColumnWithKeyboard(event, c)}
                onDoubleClick={() => setColumnWidths({})}
                title="Drag to resize; double-click to reset"
              />
            </TableHead>
          ))}
        </TableRow>
      </TableHeader>
      <TableBody>
        {topPad > 0 && (
          <tr aria-hidden="true" className="fl-data-table__spacer">
            <td colSpan={colCount} style={{ height: topPad, padding: 0, border: 0 }} />
          </tr>
        )}
        {windowRows.map((row) => {
          const rowKey = getRowKey(row);
          const selected = selectedKey === rowKey;
          const checked = selection?.selected.has(rowKey) ?? false;
          return (
            <TableRow
              key={rowKey}
              aria-selected={selected}
              data-state={selected || checked ? "selected" : undefined}
              onClick={onRowClick ? () => onRowClick(row) : undefined}
              className={cn("fl-data-table__row", onRowClick && "cursor-pointer")}
            >
              {selection && (
                <TableCell
                  className="fl-data-table__cell fl-data-table__checkbox"
                  onClick={(e) => e.stopPropagation()}
                >
                  <input
                    type="checkbox"
                    aria-label={`Select ${rowKey}`}
                    checked={checked}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => toggleRow(rowKey, (e.nativeEvent as MouseEvent).shiftKey)}
                  />
                </TableCell>
              )}
              {columns.map((c) => (
                <TableCell key={c.key} className="fl-data-table__cell">
                  {c.render ? c.render(row) : String((row as Record<string, unknown>)[c.key])}
                </TableCell>
              ))}
            </TableRow>
          );
        })}
        {bottomPad > 0 && (
          <tr aria-hidden="true" className="fl-data-table__spacer">
            <td colSpan={colCount} style={{ height: bottomPad, padding: 0, border: 0 }} />
          </tr>
        )}
        {visibleData.length === 0 && (
          <TableRow>
            <TableCell colSpan={colCount} className="fl-data-table__no-results">
              No matching items
            </TableCell>
          </TableRow>
        )}
      </TableBody>
    </ShadTable>
    </div>
  );
}
