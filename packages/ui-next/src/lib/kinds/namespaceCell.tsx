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
 */
export function withNamespaceSelect<Row extends ListRow>(
  columns: Column<Row>[],
  selection: readonly string[],
  onAdd: ((namespace: string) => void) | undefined,
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
            className="max-w-full cursor-pointer truncate text-left hover:underline focus-visible:underline"
            title={label}
            aria-label={label}
            onClick={(e) => {
              e.stopPropagation();
              onAdd(namespace);
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
