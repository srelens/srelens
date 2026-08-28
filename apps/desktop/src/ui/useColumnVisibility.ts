import { useCallback, useEffect, useMemo, useState } from "react";
import { loadHiddenColumns, saveHiddenColumns } from "@srelens/core";
import type { Column } from "./Table";
import type { ColumnOption } from "./ColumnPicker";

export interface ColumnVisibility<T> {
  /** Columns to actually render (identifier column + non-hidden). */
  visibleColumns: Column<T>[];
  /** Options for the ColumnPicker (label falls back to key for non-string headers). */
  columnOptions: ColumnOption[];
  /** Currently hidden column keys. */
  hidden: ReadonlySet<string>;
  /** Toggle a column's visibility (no-op for the pinned identifier column). */
  toggle: (key: string) => void;
  /** The always-visible identifier column key (the first column). */
  pinnedKey?: string;
}

/**
 * Per-view show/hide column state, persisted in localStorage under `viewKey`.
 * The first column is the row identifier and is always kept. Persistence is
 * keyed only by the view, so a table's layout is shared across clusters.
 */
/** A column's label if it has a usable (non-empty string) header, else "". */
function columnLabel<T>(column: Column<T>): string {
  return typeof column.header === "string" ? column.header.trim() : "";
}

export function useColumnVisibility<T>(viewKey: string, columns: Column<T>[]): ColumnVisibility<T> {
  const pinnedKey = columns[0]?.key;
  const [hidden, setHidden] = useState<Set<string>>(() => new Set(loadHiddenColumns(viewKey)));

  // Reload when the view changes (e.g. switching resource kind in one browser).
  useEffect(() => {
    setHidden(new Set(loadHiddenColumns(viewKey)));
  }, [viewKey]);

  // Only labelled, non-identifier columns can be hidden. The pinned first column
  // and headerless columns (e.g. a row-actions cell) are always shown.
  const isHideable = useCallback(
    (column: Column<T>) => column.key !== pinnedKey && columnLabel(column) !== "",
    [pinnedKey],
  );

  const visibleColumns = useMemo(
    () => columns.filter((column) => !isHideable(column) || !hidden.has(column.key)),
    [columns, hidden, isHideable],
  );

  const columnOptions: ColumnOption[] = useMemo(
    () =>
      columns
        .filter((column) => column.key === pinnedKey || columnLabel(column) !== "")
        .map((column) => ({ key: column.key, label: columnLabel(column) || column.key })),
    [columns, pinnedKey],
  );

  const toggle = useCallback(
    (key: string) => {
      if (key === pinnedKey) return;
      setHidden((prev) => {
        const next = new Set(prev);
        if (next.has(key)) next.delete(key);
        else next.add(key);
        saveHiddenColumns(viewKey, [...next]);
        return next;
      });
    },
    [viewKey, pinnedKey],
  );

  return { visibleColumns, columnOptions, hidden, toggle, pinnedKey };
}
