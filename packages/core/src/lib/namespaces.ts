/**
 * Namespace selection helpers. The selection is a set of namespace names; an
 * empty set means "all namespaces". It's persisted/threaded as a comma-joined
 * string so the tab/settings interface stays a plain string.
 */

/** Parse the persisted comma string into a deduped set (empty = all). */
export function parseNamespaceSelection(value: string): string[] {
  const seen = new Set<string>();
  for (const part of value.split(",")) {
    const ns = part.trim();
    if (ns) seen.add(ns);
  }
  return [...seen];
}

/** Serialize a selection set back to the persisted comma string. */
export function serializeNamespaceSelection(selection: string[]): string {
  return selection.join(",");
}

/**
 * The namespace to open a watch on for a selection. One selected namespace →
 * that namespace (efficient); none or many → all namespaces (`""`), then
 * filtered client-side by {@link rowInSelection}.
 */
export function watchNamespaceForSelection(selection: string[]): string {
  return selection.length === 1 ? selection[0] : "";
}

/** Whether a row's namespace passes the selection (empty selection = all). */
export function rowInSelection(rowNamespace: string, selection: string[]): boolean {
  return selection.length === 0 || selection.includes(rowNamespace);
}
