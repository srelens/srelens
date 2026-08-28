import { useSyncExternalStore } from "react";
import type { ClusterContext } from "@srelens/core";
import type { TableSort } from "@srelens/ui-kit";
import {
  CLOSED_CAP,
  defaultState,
  makeTab,
  newId,
  reconcile,
  relabel,
  type Tab,
  type TabsState,
  type Workspace,
} from "./tabs";

/**
 * The tab store: module-level state, one hook, plain functions for actions.
 *
 * Mock-style on purpose — the shell's state lives in ui-next, not in core. What
 * is not the mock's: ids are random rather than counted (see `newId`), the
 * closed list is per workspace rather than global, and nothing here persists
 * or reads persistence. `tabsPersist.ts` subscribes to this store and owns
 * the file; keeping the two apart is what lets this one be tested with no
 * storage at all.
 *
 * Every action returns the workspace it was given when it would change
 * nothing, and `emit` skips an unchanged state, so a no-op never wakes a
 * subscriber — which matters because one of those subscribers writes a file.
 */
/**
 * Built on first read rather than while this module is evaluating.
 *
 * `defaultState` calls `describe`, which lives in `lib/routes` — the same
 * module that holds the table of screens, and screens read their state back
 * out of this store. A call made from the module body runs in the middle of
 * that cycle, when whichever module happened to load first has not yet
 * assigned its imports, and it took half the package down with it. Deferring
 * it by one call is enough: nothing asks for the state until something
 * renders, by which time every module has finished loading.
 */
let _state: TabsState | null = null;
const listeners = new Set<() => void>();

function cur(): TabsState {
  return (_state ??= defaultState([]));
}

function emit(next: TabsState) {
  if (next === cur()) return;
  _state = next;
  for (const l of listeners) l();
}

export function getState(): TabsState {
  return cur();
}

/** Replace the whole state — for boot and for tests. */
export function setState(next: TabsState): void {
  emit(next);
}

/**
 * Make every workspace consistent with the clusters that actually exist.
 *
 * **Called after a listing that answered, from the contexts store rather than
 * from a screen.** `Window` does this at boot against the restored state, and
 * for a while it was the only caller: `/connections`' `Refresh all` and
 * `/connect`'s `reload` both replaced the context list and left the workspace
 * pointing at whatever it had, so removing or renaming the ACTIVE context and
 * refreshing left `activeCluster` naming a context nobody declares —
 * `useActiveContext()` `undefined`, and every cluster-scoped screen on its
 * no-cluster state while other clusters sat in the table. `reconcile` is what
 * picks the first survivor, and it did not run.
 *
 * It lives behind the contexts store's own write (see `setContexts`) so that
 * every listing goes through it, including a fourth caller nobody has written
 * yet. `reconcile` returns the state untouched when nothing needed changing and
 * `emit` skips an unchanged state, so a refresh that answers with the same
 * clusters wakes nobody — one of the subscribers writes a file.
 */
export function reconcileClusters(contexts: readonly ClusterContext[]): void {
  emit(reconcile(cur(), [...contexts]));
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function currentWorkspace(): Workspace {
  const state = cur();
  return state.workspaces.find((w) => w.id === state.currentId) ?? state.workspaces[0];
}

/**
 * The cluster the sidebar and status bar are about, per workspace and
 * persisted with it: which cluster you were looking at is worth surviving a
 * restart, the way the open tabs are.
 */
export function activeCluster(): string | null {
  return currentWorkspace().activeCluster ?? null;
}

export function useActiveCluster(): string | null {
  return useSyncExternalStore(subscribe, activeCluster, activeCluster);
}

export function activeRoute(): string {
  const w = currentWorkspace();
  return w.tabs.find((t) => t.id === w.activeId)?.route ?? "/";
}

function patchWorkspace(id: string, patch: (w: Workspace) => Workspace) {
  const state = cur();
  const at = state.workspaces.findIndex((w) => w.id === id);
  if (at < 0) return;
  const next = patch(state.workspaces[at]);
  // Identity is the signal for "nothing changed" — no new array, no emit.
  if (next === state.workspaces[at]) return;
  const workspaces = state.workspaces.slice();
  workspaces[at] = next;
  emit({ ...state, workspaces });
}

function patchCurrent(patch: (w: Workspace) => Workspace) {
  patchWorkspace(cur().currentId, patch);
}

function remember(w: Workspace, dropped: Tab[]): Tab[] {
  return [...dropped, ...w.closed].slice(0, CLOSED_CAP);
}

export function useTabs() {
  const s = useSyncExternalStore(subscribe, getState, getState);
  const workspace = s.workspaces.find((w) => w.id === s.currentId) ?? s.workspaces[0];
  return {
    tabs: workspace.tabs,
    activeId: workspace.activeId,
    workspace,
    workspaces: s.workspaces,
    closed: workspace.closed,
  };
}

/**
 * The tab for a route: the one already open, or a new one.
 *
 * **The dedupe is by route, and a reused tab is relabelled for the cluster the
 * caller named.** Route-only dedupe is what most callers want — one tab per
 * detail route, per list route, per `/overview` — and it stays. What did not
 * hold was returning that tab untouched: `makeTab` fixes the `sub` at creation
 * from `clusterName`, so opening a second cluster onto an `/overview` tab left
 * the strip reading the first cluster while the screen rendered the second.
 *
 * The relabel lives here rather than in `openCluster` because HERE is where the
 * reuse decision is made, and every caller that names a cluster had the same
 * hole: `Nav`, `Status`, `openCluster` and the six screens that open a detail
 * or logs route from a row. Fixing it in one caller would have left the others
 * to find it again. See {@link relabel} for why the new label comes back
 * through the route table rather than from the argument.
 */
export function openTab(route: string, opts: { preview?: boolean; clusterName?: string } = {}): void {
  patchCurrent((w) => {
    const existing = w.tabs.find((t) => t.route === route);
    if (existing) {
      // Opening for real promotes a preview; re-previewing leaves it be.
      const promoted =
        !opts.preview && existing.preview ? { ...existing, preview: false } : existing;
      const next = relabel(promoted, opts.clusterName);
      // Identity is the signal, as everywhere else in this store: nothing to
      // promote and nothing to relabel means no new array and no emit.
      const tabs = next === existing ? w.tabs : w.tabs.map((t) => (t.id === existing.id ? next : t));
      if (tabs === w.tabs && w.activeId === existing.id) return w;
      return { ...w, tabs, activeId: existing.id };
    }
    const next = makeTab(route, opts);
    const previewAt = w.tabs.findIndex((t) => t.preview);
    const tabs =
      opts.preview && previewAt >= 0 ? w.tabs.map((t, i) => (i === previewAt ? next : t)) : [...w.tabs, next];
    return { ...w, tabs, activeId: next.id };
  });
}

/**
 * Always a new tab, even for a route already open — that is what "new tab" means.
 *
 * Never pinned: `pinned` belongs to the seed home tab that must always be there,
 * not to the route, and `makeTab("/")` — what Cmd+T asks for — would otherwise
 * hand back a tab that every close path refuses.
 */
export function newTab(route = "/", clusterName?: string): void {
  patchCurrent((w) => {
    const t: Tab = { ...makeTab(route, { clusterName }), pinned: false };
    return { ...w, tabs: [...w.tabs, t], activeId: t.id };
  });
}

export function activateTab(id: string): void {
  const w = currentWorkspace();
  if (w.activeId === id || !w.tabs.some((t) => t.id === id)) return;
  patchCurrent((w) => ({ ...w, activeId: id }));
}

export function closeTab(id: string): void {
  const w = currentWorkspace();
  const at = w.tabs.findIndex((t) => t.id === id);
  const tab = w.tabs[at];
  if (!tab || tab.pinned || w.tabs.length === 1) return;
  patchCurrent((w) => {
    const tabs = w.tabs.filter((t) => t.id !== id);
    // The right neighbour takes over, then the left at the end of the strip.
    const activeId = w.activeId === id ? (tabs[Math.min(at, tabs.length - 1)] ?? tabs[0]).id : w.activeId;
    return { ...w, tabs, activeId, closed: remember(w, [tab]) };
  });
}

export function closeOthers(id: string): void {
  patchCurrent((w) => {
    if (!w.tabs.some((t) => t.id === id)) return w;
    const dropped = w.tabs.filter((t) => t.id !== id && !t.pinned);
    if (!dropped.length && w.activeId === id) return w;
    return { ...w, tabs: w.tabs.filter((t) => t.id === id || t.pinned), activeId: id, closed: remember(w, dropped) };
  });
}

export function closeToRight(id: string): void {
  patchCurrent((w) => {
    const at = w.tabs.findIndex((t) => t.id === id);
    if (at < 0) return w;
    const dropped = w.tabs.slice(at + 1).filter((t) => !t.pinned);
    if (!dropped.length && w.activeId === id) return w;
    return { ...w, tabs: w.tabs.filter((t, i) => i <= at || t.pinned), activeId: id, closed: remember(w, dropped) };
  });
}

export function closeAll(): void {
  patchCurrent((w) => {
    const keep = w.tabs.filter((t) => t.pinned);
    const dropped = w.tabs.filter((t) => !t.pinned);
    const tabs = keep.length ? keep : [makeTab("/")];
    if (!dropped.length && w.activeId === tabs[0].id) return w;
    return { ...w, tabs, activeId: tabs[0].id, closed: remember(w, dropped) };
  });
}

export function reopenClosed(): void {
  const w = currentWorkspace();
  const [last, ...rest] = w.closed;
  if (!last) return;
  patchCurrent((w) => {
    const revived = { ...last, id: newId() };
    return { ...w, tabs: [...w.tabs, revived], activeId: revived.id, closed: rest };
  });
}

export function duplicateTab(id: string): void {
  patchCurrent((w) => {
    const at = w.tabs.findIndex((t) => t.id === id);
    if (at < 0) return w;
    const copy: Tab = { ...w.tabs[at], id: newId(), preview: false, pinned: false };
    const tabs = [...w.tabs.slice(0, at + 1), copy, ...w.tabs.slice(at + 1)];
    return { ...w, tabs, activeId: copy.id };
  });
}

export function togglePin(id: string): void {
  patchCurrent((w) => {
    if (!w.tabs.some((t) => t.id === id)) return w;
    return { ...w, tabs: w.tabs.map((t) => (t.id === id ? { ...t, pinned: !t.pinned } : t)) };
  });
}

export function cycleTab(delta: 1 | -1): void {
  const w = currentWorkspace();
  const i = w.tabs.findIndex((t) => t.id === w.activeId);
  const next = w.tabs[(i + delta + w.tabs.length) % w.tabs.length];
  if (next) activateTab(next.id);
}

export function selectIndex(n: number): void {
  const tab = currentWorkspace().tabs[n];
  if (tab) activateTab(tab.id);
}

export function switchWorkspace(id: string): void {
  const state = cur();
  if (id === state.currentId || !state.workspaces.some((w) => w.id === id)) return;
  emit({ ...state, currentId: id });
}

export function createWorkspace(name: string, clusters: string[]): string {
  const home = makeTab("/");
  const w: Workspace = { id: newId(), name, clusters, tabs: [home], activeId: home.id, closed: [] };
  if (clusters[0]) w.activeCluster = clusters[0];
  emit({ workspaces: [...cur().workspaces, w], currentId: w.id });
  return w.id;
}

export function renameWorkspace(id: string, name: string): void {
  patchWorkspace(id, (w) => (w.name === name ? w : { ...w, name }));
}

export function removeWorkspace(id: string): void {
  const state = cur();
  if (state.workspaces.length <= 1) return;
  const at = state.workspaces.findIndex((w) => w.id === id);
  if (at < 0) return;
  const workspaces = state.workspaces.filter((w) => w.id !== id);
  const currentId =
    state.currentId === id ? (workspaces[Math.min(at, workspaces.length - 1)] ?? workspaces[0]).id : state.currentId;
  emit({ workspaces, currentId });
}

export function setWorkspaceClusters(id: string, clusters: string[]): void {
  patchWorkspace(id, (w) => {
    const same = w.clusters.length === clusters.length && w.clusters.every((c, i) => c === clusters[i]);
    // The active cluster is an index into this list, so taking a cluster away
    // has to move it — `reconcile` only knows about clusters that vanished
    // from the machine, not ones dropped from a workspace.
    const active = w.activeCluster && clusters.includes(w.activeCluster) ? w.activeCluster : clusters[0];
    if (same && active === w.activeCluster) return w;
    const next: Workspace = { ...w, clusters: [...clusters] };
    if (active) next.activeCluster = active;
    else delete next.activeCluster;
    return next;
  });
}

const EMPTY_VIEW: NonNullable<Tab["view"]> = {};

function sortEqual(a: TableSort | null | undefined, b: TableSort | null | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.key === b.key && a.direction === b.direction;
}

/**
 * A resource list's sort, filter string and active filter column, merged
 * into whatever the tab already has. Guarded like every other action here:
 * one subscriber writes a file, so patching a view with values it already
 * holds must not emit.
 */
export function setTabView(tabId: string, patch: Partial<NonNullable<Tab["view"]>>): void {
  patchCurrent((w) => {
    const at = w.tabs.findIndex((t) => t.id === tabId);
    if (at < 0) return w;
    const current = w.tabs[at].view ?? EMPTY_VIEW;
    const next: NonNullable<Tab["view"]> = { ...current, ...patch };
    if (sortEqual(current.sort, next.sort) && current.filter === next.filter && current.filterKey === next.filterKey) {
      return w;
    }
    const tabs = w.tabs.map((t, i) => (i === at ? { ...t, view: next } : t));
    return { ...w, tabs };
  });
}

export function useTabView(tabId: string): NonNullable<Tab["view"]> {
  const getView = () => currentWorkspace().tabs.find((t) => t.id === tabId)?.view ?? EMPTY_VIEW;
  return useSyncExternalStore(subscribe, getView, getView);
}

/**
 * `null` for "no cluster in focus"; anything the workspace does not have is
 * refused rather than stored, so the field is always an id in `clusters`.
 *
 * **Every cluster-scoped tab is relabelled for the cluster named here**, and
 * that is not a flourish either. No tab is about a cluster of its own: every
 * cluster-scoped screen reads `useActiveContext()`, so the instant this field
 * moves, the mounted screen is rendering the new cluster. A tab whose `sub`
 * still names the old one labels that screen with a cluster it is not showing,
 * and an action started from it runs against one cluster under a tab reading
 * another. The rail's own `onSelect` was exactly that: `setActiveCluster` bare.
 *
 * **The whole workspace, not only the active tab.** Every one of its tabs is
 * about the cluster in focus, so leaving the background ones stale only defers
 * the mislabelling until the reader switches tab — and `relabel` is a no-op for
 * a tab already carrying the name, so the pass costs an identity check per tab.
 * App-scoped routes come back from `describe` with no `sub` at all, so
 * `/settings` and `/connections` keep none: the route table stays the one place
 * that decides whether a route is about a cluster.
 *
 * **`clusterName` rather than a lookup here.** Workspaces hold `stableId`s
 * (#265) and tabs carry context *names*; the store that translates between them
 * is `lib/clusters`, which imports THIS module, so reaching back for the name
 * would be a cycle. The caller who knows the cluster passes its name, exactly
 * as {@link openTab} takes it. No name means the caller said nothing about a
 * cluster and the labels stand — see {@link relabel}.
 *
 * The refusal above covers the labels too: a cluster this workspace does not
 * have does not become the label of tabs it did not switch.
 */
export function setActiveCluster(id: string | null, clusterName?: string): void {
  patchCurrent((w) => {
    if (id !== null && !w.clusters.includes(id)) return w;
    let tabs = w.tabs;
    if (clusterName) {
      const relabelled = w.tabs.map((t) => relabel(t, clusterName));
      // Identity, as everywhere else here: a strip that needed no relabelling
      // keeps its array, so an unchanged switch still emits nothing and the
      // subscriber that writes the settings file stays asleep.
      if (relabelled.some((t, i) => t !== w.tabs[i])) tabs = relabelled;
    }
    if (tabs === w.tabs && (w.activeCluster ?? null) === id) return w;
    const next: Workspace = { ...w, tabs };
    if (id === null) delete next.activeCluster;
    else next.activeCluster = id;
    return next;
  });
}
