import { useCallback, useEffect, useRef, type RefObject } from "react";
import type { Column } from "@srelens/ui-kit";
import type { ListRow } from "./types";

/** The column this decorates — every namespaced kind's set spells it this way. */
const NAMESPACE_KEY = "namespace";

/**
 * What a click on a namespace does to the selection: adds it.
 *
 * An empty selection is "all namespaces", so adding to it narrows the list to
 * the one clicked; a selection that already names some gains another. Either
 * way the result is the old selection with this namespace in it, which is what
 * ticking it in the picker gives — one rule, and the picker and the cell
 * cannot come to disagree about it.
 */
export function addNamespace(selection: readonly string[], namespace: string): string[] {
  return selection.includes(namespace) ? [...selection] : [...selection, namespace];
}

/** What the click will do, said the way the reader will see it happen. */
function hint(selection: readonly string[], namespace: string): string {
  return selection.length === 0 ? `Show only namespace ${namespace}` : `Also show namespace ${namespace}`;
}

/**
 * Make the Namespace column's values clickable: a click adds that namespace
 * to the tab's namespace selection (#821).
 *
 * The value a reader wants to narrow by is already under the cursor, on the
 * row that caught their eye; reaching it through the picker meant opening it
 * and finding the same name again in a list of sixty.
 *
 * Layered on like `withRowAffordances`, after hiding and outside what
 * `ColumnPicker` and `filterTableData` see, so the column still sorts and
 * filters on the bare string. One decorator rather than a renderer on each of
 * the thirty-odd `{ key: "namespace" }` columns, which is thirty places for the
 * next kind's column to be the one that forgot.
 *
 * Three cells stay plain text, because a control that does nothing is worse
 * than no control:
 *
 * - a namespace already in the selection — there is nothing to add;
 * - any cell when `onAdd` is absent — a namespace-scoped credential has one
 *   namespace and no way to ask for another;
 * - a row with no namespace.
 *
 * A column that brings its own `render` is left alone: it has decided what its
 * cell is.
 *
 * The click, the double-click and the Enter/Space that activate the button all
 * stop at it, for the reason `AskChip` gives: the row underneath peeks on a
 * click and opens a tab on a double-click or Enter, and a namespace picked
 * must not also open the resource it happened to be read off.
 *
 * `onAdd` is handed the row as well as the namespace, so the screen can bring
 * focus back to it — see {@link useRowRefocus}.
 */
export function withNamespaceSelect<Row extends ListRow>(
  columns: Column<Row>[],
  selection: readonly string[],
  onAdd: ((namespace: string, row: Row) => void) | undefined,
): Column<Row>[] {
  if (!onAdd) return columns;
  return columns.map((column) => {
    if (column.key !== NAMESPACE_KEY || column.render) return column;
    return {
      ...column,
      render: (row: Row) => {
        const namespace = row.namespace;
        if (!namespace) return null;
        if (selection.includes(namespace)) return namespace;
        const label = hint(selection, namespace);
        return (
          <button
            type="button"
            // Dotted at rest, solid under the pointer or the keyboard: a
            // name with nothing drawn on it reads as text, and nobody clicks
            // text to find out. Dotted because this is one cell in every row.
            className="max-w-full cursor-pointer truncate text-left underline decoration-dotted underline-offset-2 hover:decoration-solid focus-visible:decoration-solid"
            title={label}
            aria-label={label}
            onClick={(e) => {
              e.stopPropagation();
              // This button is about to be replaced by plain text — its
              // namespace will be in the selection — and a focused element
              // that leaves the document drops focus to the page, which for a
              // keyboard reader is their place in the table gone. The row is
              // the table's own focus stop, so focus is handed to it first.
              // (A list that reloads takes the row away too; `useRowRefocus`
              // is what brings focus back then.)
              e.currentTarget.closest("tr")?.focus();
              onAdd(namespace, row);
            }}
            onDoubleClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") e.stopPropagation();
            }}
          >
            {namespace}
          </button>
        );
      },
    };
  });
}

/** How long after a pick focus is still brought back. Past this the reader
 *  has had the page to themselves, and focus appearing on a row would be
 *  srelens moving it, not returning it. */
export const REFOCUS_WITHIN_MS = 10_000;

/**
 * Bring keyboard focus back to one row after the table it was in is rebuilt.
 *
 * Picking a namespace changes the selection, a changed selection is a new
 * listing, and a list that is listing again shows a loading state where the
 * table was — every row, the focused one included, leaves the document, and
 * focus falls to the page. A keyboard reader who pressed Enter on a namespace
 * was left at the top of the window with the table to walk into again.
 *
 * `remember(key)` names the row (by the table's own `getRowKey`); when rows
 * are next on screen under `scope` and nothing else holds focus, that row is
 * focused. It gives up — without touching focus — as soon as the reader has
 * put focus somewhere outside the table themselves, or after {@link
 * REFOCUS_WITHIN_MS}.
 *
 * Checked after every render of the screen that calls it rather than on a
 * dependency list: what it waits on is the table's rows being in the DOM,
 * which is not a value this hook is handed.
 */
export function useRowRefocus(): { scope: RefObject<HTMLDivElement | null>; remember: (key: string) => void } {
  const scope = useRef<HTMLDivElement | null>(null);
  const pending = useRef<{ key: string; at: number } | null>(null);
  const remember = useCallback((key: string) => {
    pending.current = { key, at: Date.now() };
  }, []);

  useEffect(() => {
    const wanted = pending.current;
    if (!wanted) return;
    if (Date.now() - wanted.at > REFOCUS_WITHIN_MS) {
      pending.current = null;
      return;
    }
    const active = document.activeElement;
    const lost = active === null || active === document.body;
    if (!lost) {
      // Still inside the table: nothing to restore yet, and it may yet be
      // rebuilt. Anywhere else is the reader's own move, and theirs to keep.
      if (!scope.current?.contains(active)) pending.current = null;
      return;
    }
    const rows = scope.current?.querySelectorAll<HTMLElement>("tr[data-row-key]") ?? [];
    for (const row of rows) {
      if (row.dataset.rowKey !== wanted.key) continue;
      row.focus();
      pending.current = null;
      return;
    }
  });

  return { scope, remember };
}
