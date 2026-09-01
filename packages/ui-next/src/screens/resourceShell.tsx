import { Alert, Button, EmptyState, LoadingState, MultiSelect, Screen, Spinner, type Column, type TableSort } from "@srelens/ui-kit";
import { useContextsError, useContextsStatus } from "../lib/clusters";
import { toggleColumn } from "../lib/columnPrefs";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { setTabView, useTabs, useTabView } from "../lib/tabsStore";

/**
 * Shell pieces `Resources.tsx` (one kind per `/k/<slug>` tab) and
 * `Workloads.tsx` (five kinds unioned at `/resources`) both need, verbatim.
 *
 * A whole-branch review found a batch of fixes landing on `Resources.tsx`
 * that never reached `Workloads.tsx`, because the shell the two screens open
 * with was duplicated rather than shared — there was nowhere for a fix
 * applied once to live. What's here is only the pieces that are *actually*
 * identical between the two: the no-cluster guard, the namespace picker's
 * loading/error treatment, and the tab-bound sort/filter/filterKey plumbing.
 *
 * What's deliberately NOT here: the row-menu wiring, the list-loading and
 * per-kind error banners, and the table itself. `Resources.tsx` composes
 * around one descriptor and one row type; `Workloads.tsx` aggregates five
 * fixed watches into a union row. Forcing those through one shared component
 * would cost a worse abstraction than the duplication it replaces — see the
 * two screens' own module comments.
 */

/**
 * The guard every screen opens with: no cluster context to call core with, so
 * there is nothing to list. At the call site it is a `return` before any hook
 * runs, not a branch inside a hook-calling body — the hooks below are this
 * component's own, which is a different component and so a different list.
 *
 * **Why this is three states and not one sentence.** `useActiveContext` is the
 * intersection of the workspace's active cluster and the *loaded* context
 * list, and it answers `undefined` when either half is missing. That collapsed
 * three unrelated situations into "No cluster in focus — pick a cluster in the
 * rail", including the one where a cluster plainly *is* in focus: every tab
 * carrying its chip, the title bar carrying its name, and `listContexts`
 * having refused underneath. The reader was being blamed for a backend
 * failure srelens had already read and thrown away. It knew "I could not list
 * the contexts" and said "you have not picked one" — the string asserting more
 * than srelens knows, on six screens at once, because they all render this.
 *
 * So the copy follows the store's own three states and nothing else:
 *
 * - **listed, none picked** — the original sentence, which is right here and
 *   is left exactly as it was.
 * - **not listed yet** — a spinner. The reader has nothing to do about a
 *   listing that has not come back, and an instruction they cannot act on is
 *   worse than no instruction.
 * - **refused** — the reason, classified by `describeError` like every other
 *   failure in this app, with the backend's own words a disclosure away.
 *
 * There is deliberately no fourth state for "refused, but a cluster is
 * selected". The failure is the same failure and the remedy is the same
 * remedy; splitting it would only let the two halves drift.
 */
export function NoClusterScreen({ title, noun }: { title: string; noun: string }) {
  const status = useContextsStatus();
  const error = useContextsError();
  return (
    <Screen title={title} fill>
      {status === "loading" ? (
        <LoadingState label="Loading clusters" className="flex-1" />
      ) : status === "failed" ? (
        // The contextual half of the message is this component's; the detail
        // under it is the classification, per `errorCopy`'s rule.
        <FailureState title="Clusters could not be listed" error={error} className="my-auto" />
      ) : (
        <EmptyState
          title="No cluster in focus"
          hint={`Pick a cluster in the rail to list its ${noun}.`}
          className="flex-1"
        />
      )}
    </Screen>
  );
}

/**
 * The namespace picker's two states: a disabled, spinning stand-in while
 * `namespaces` is still `null`, and the real picker once it has answered.
 * Zero options while `namespaces` is null reads as "this cluster has no
 * namespaces" — a bare `MultiSelect options={(namespaces ?? []).map(...)}`
 * says exactly that, which is what let this drift between the two screens in
 * the first place.
 */
export function NamespacePicker({
  namespaces,
  selection,
  onChange,
}: {
  namespaces: string[] | null;
  selection: string[];
  onChange: (next: string[]) => void;
}) {
  if (namespaces === null) {
    return (
      <Button variant="secondary" className="justify-between gap-1.5" disabled aria-label="Namespaces">
        <Spinner label="Loading namespaces" />
        Loading namespaces…
      </Button>
    );
  }
  return (
    <MultiSelect
      options={namespaces.map((ns) => ({ value: ns }))}
      selection={selection}
      onChange={onChange}
      allLabel="All namespaces"
      ariaLabel="Namespaces"
    />
  );
}

/**
 * `useNamespaceOptions`'s failure, surfaced rather than swallowed. Non-fatal:
 * the hook keeps whatever namespaces it had before the failure, so the picker
 * and the rows both keep working — this only says the list behind the picker
 * may be incomplete.
 */
export function NamespaceErrorAlert({ error }: { error: string }) {
  if (!error) return null;
  // The sentence stays; what sits under it is the classification rather than
  // whatever the apiserver's Go or Rust layer happened to print. A 403 on
  // namespaces is the ordinary case here, and "You don't have permission to
  // list namespaces at the cluster scope" is a thing a reader can act on.
  return (
    <FailureAlert
      title="Namespaces could not be listed"
      error={error}
      className="mx-3 mt-3 mb-3"
    />
  );
}

/**
 * Whether a remembered selection is stale — every namespace it names is gone
 * from the cluster, rather than merely holding no rows of this particular
 * kind right now. The two look identical from the row count alone, which is
 * exactly why this is decided separately: emptiness is ordinary, a selection
 * naming nothing the cluster has is not. `namespaces === null` (still
 * loading) is not evidence of anything, since nothing has been checked yet.
 */
export function selectionIsStale(selection: string[], namespaces: string[] | null): boolean {
  return selection.length > 0 && namespaces !== null && selection.every((ns) => !namespaces.includes(ns));
}

/**
 * What a stale selection is worth telling the reader, with the same
 * recovery a manual clear gives: back to "all namespaces". Silence here
 * would read as "this cluster has none of what you're looking for" when the
 * true story is "the namespaces you last picked — from a deleted namespace,
 * or a kubeconfig that now points somewhere else — aren't there to look in".
 * Left in place rather than reset on its own: an automatic reset the reader
 * never asked for is a second surprise stacked on the first, and the
 * selection may yet be exactly right again once whatever changed changes
 * back.
 */
export function StaleSelectionAlert({
  selection,
  namespaces,
  onReset,
}: {
  selection: string[];
  namespaces: string[] | null;
  onReset: () => void;
}) {
  if (!selectionIsStale(selection, namespaces)) return null;
  return (
    <Alert
      tone="warn"
      title="Remembered namespaces are gone"
      onDismiss={onReset}
      dismissLabel="Show all namespaces"
      className="mx-3 mt-3 mb-3"
    >
      {`${selection.join(", ")} no longer exist on this cluster.`}
    </Alert>
  );
}

/**
 * One resource LIST tab's view state — its sort, its filter text and the
 * column that filter names. Nothing to do with `ResourceTabView`, the screen
 * that fills a tab with one resource; this is the list's.
 */
export interface ResourceListTabView {
  tabId: string;
  sort: TableSort | null;
  filter: string;
  filterKey: string | null;
  regex: boolean;
  setFilter: (value: string) => void;
  setSort: (next: TableSort | null) => void;
  setFilterKey: (key: string | null) => void;
  setRegex: (on: boolean) => void;
}

/**
 * Sort, filter text and filter column live on the route's own tab, so they
 * survive a restart with it (#254) — component state would pass every render
 * assertion and lose all three on the next launch. This screen's *own* tab,
 * not whichever one is active: `Window` mounts every tab's body and merely
 * hides the inactive ones, so reading the active tab's view would have a
 * background list re-sorting and re-filtering itself on every keystroke
 * typed in an unrelated tab.
 *
 * `filterKey` is derived rather than merely cleared when a column is hidden:
 * hidden columns belong to the kind and are shared by every tab looking at
 * it, while the filter key belongs to one tab — so the column a filter key
 * names can be hidden from another tab, in another workspace, while this one
 * is not even mounted, and both halves persist independently.
 */
export function useResourceTabView<T>(route: string, columns: readonly Column<T>[]): ResourceListTabView {
  const { tabs } = useTabs();
  const tabId = tabs.find((tab) => tab.route === route)?.id ?? "";
  const view = useTabView(tabId);
  const sort = view.sort ?? null;
  const filter = view.filter ?? "";
  const filterKey = view.filterKey && columns.some((column) => column.key === view.filterKey) ? view.filterKey : null;
  const regex = view.regex ?? false;
  return {
    tabId,
    sort,
    filter,
    filterKey,
    regex,
    setFilter: (value) => setTabView(tabId, { filter: value }),
    setSort: (next) => setTabView(tabId, { sort: next }),
    setFilterKey: (key) => setTabView(tabId, { filterKey: key }),
    setRegex: (on) => setTabView(tabId, { regex: on }),
  };
}

/** `ColumnPicker`'s own shape, built from the kind's (or the union's) full
 *  column set — before hiding, so a hidden column can still be re-offered. */
export function columnOptionsFor<T>(columns: readonly Column<T>[]): { key: string; label: string }[] {
  return columns.map((column) => ({
    key: column.key,
    label: typeof column.header === "string" ? column.header : column.key,
  }));
}

/** Hiding the column the search is pointed at leaves a filter nobody can see
 *  and nothing can match — the classic design shipped exactly that. */
export function toggleColumnVisibility(params: {
  key: string;
  storageKey: string;
  hidden: ReadonlySet<string>;
  filterKey: string | null;
  tabId: string;
}): void {
  const { key, storageKey, hidden, filterKey, tabId } = params;
  if (!hidden.has(key) && filterKey === key) setTabView(tabId, { filterKey: null });
  toggleColumn(storageKey, key);
}

/** "This kind has none" and "the filter matched none" are different facts,
 *  and the second one is the reader's own doing — same wording, same
 *  distinction, in both screens. */
export function emptyTableCopy(
  count: number,
  noun: string,
  clusterName: string,
  scopeSuffix: string,
): { emptyText: string; emptyHint: string } {
  return count === 0
    ? { emptyText: `No ${noun}`, emptyHint: `${clusterName} has no ${noun}${scopeSuffix}.` }
    : { emptyText: `No ${noun} match this filter`, emptyHint: `Clear the filter to see all ${count}.` };
}
