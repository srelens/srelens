// Per-tab list view state (#254).
//
// Sort, the search text, and the filtered column used to live in component
// state, which meant they died the moment a tab was switched away — App
// renders only the active tab, keyed by its id, so the previous view is
// unmounted outright. Search had the opposite problem: one App-level value
// shared by every tab, so it leaked from one list into another.
//
// Holding them on the tab fixes both, and because tabs are already serialized
// (lib/openTabs.ts) it also makes them survive a restart.

/** A table's sort column and direction; null means "unsorted". */
export interface TableSort {
  key: string;
  direction: "asc" | "desc";
}

/** The slice of a tab's state that belongs to its list view. */
export interface TabViewState {
  sort?: TableSort | null;
  /** Column the search box is scoped to; null searches every column. */
  filterColumn?: string | null;
  query?: string;
}

/**
 * The sort a header click should produce: ascending, then descending, then
 * back to unsorted. Pure so the cycle is testable without a rendered table,
 * and so a controlled table can compute it from the value it was handed.
 */
export function nextSort(current: TableSort | null | undefined, key: string): TableSort | null {
  if (!current || current.key !== key) return { key, direction: "asc" };
  if (current.direction === "asc") return { key, direction: "desc" };
  return null;
}

/**
 * Apply a patch to a tab's view state, returning undefined when the result
 * carries nothing. Dropping an empty object keeps it out of the serialized
 * tab, so a session file isn't littered with `"view": {}` for every tab the
 * user never sorted or searched.
 */
export function applyViewPatch(
  current: TabViewState | undefined,
  patch: Partial<TabViewState>,
): TabViewState | undefined {
  const next: TabViewState = { ...current, ...patch };
  // An explicitly cleared field is dropped rather than stored as a null/empty
  // that would round-trip through the session file to no effect.
  if (next.sort == null) delete next.sort;
  if (next.filterColumn == null) delete next.filterColumn;
  if (!next.query) delete next.query;
  return Object.keys(next).length > 0 ? next : undefined;
}
