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
 * The ONE namespace a selection names, or `""` for none or several.
 *
 * Not how a selection is listed any more: several namespaces are listed one
 * at a time by `watchNamespaces` (#688), because a credential scoped to a few
 * namespaces is refused the cluster-scope list this used to open for them.
 * What is left is its use as a single-namespace hint — a detail route's
 * fallback, an app column's scope — where "several" genuinely means none.
 */
export function watchNamespaceForSelection(selection: string[]): string {
  return selection.length === 1 ? selection[0] : "";
}

/** Whether a row's namespace passes the selection (empty selection = all). */
export function rowInSelection(rowNamespace: string, selection: string[]): boolean {
  return selection.length === 0 || selection.includes(rowNamespace);
}

/** How many namespaces {@link namespacePhrase} names before it counts the rest. */
const NAMED_NAMESPACES = 3;

/**
 * A list of namespaces as words for a sentence — "team-a and team-b", or
 * "5 namespaces: a, b, c and 2 more" once naming them all would make the
 * sentence about the list instead of the failure.
 */
export function namespacePhrase(namespaces: string[]): string {
  if (namespaces.length === 1) return namespaces[0];
  if (namespaces.length <= NAMED_NAMESPACES) {
    return `${namespaces.slice(0, -1).join(", ")} and ${namespaces.at(-1)}`;
  }
  const named = namespaces.slice(0, NAMED_NAMESPACES).join(", ");
  return `${namespaces.length} namespaces: ${named} and ${namespaces.length - NAMED_NAMESPACES} more`;
}
