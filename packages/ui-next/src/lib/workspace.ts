import { useCallback, useEffect, useSyncExternalStore } from "react";
import { getDefaultNamespace, setDefaultNamespace, settingsStorage } from "@srelens/core";
import type { Tone } from "@srelens/ui-kit";
import type { Storage } from "./tabsPersist";
import {
  currentWorkspace,
  forgetClusterNamespaces,
  renameNamespaceCollection,
  setTabNamespaceCollections,
  setTabNamespaces,
  subscribe as subscribeTabs,
  tabNamespaceCollections,
  tabNamespaces,
} from "./tabsStore";
import { collectionScope, subscribeScopes } from "./collectionScope";
import { markCollectionsUsed, onCollectionRenamed } from "./collections";
import { useTabScope } from "./tabScope";

export type LinkState = "connected" | "connecting" | "disconnected" | "error";

/**
 * What the link says, in words. The mock said it in colour alone — a green dot
 * for connected, a red one for unreachable — which is no readout at all for
 * anyone who cannot separate the two, so the words are the readout and the
 * tone is the second channel. "Unreachable" rather than "Error" for `error`:
 * the failure being reported is the cluster's, not the app's, and the person
 * reading it wants to know which. (#320)
 *
 * It lives beside {@link LinkState} rather than in the status bar that first
 * drew it, because there is now more than one readout of the same fact — the
 * strip along the bottom and the overview rail's `Connection` row — and a
 * second copy of this table is how the two start disagreeing about the same
 * cluster. A link state is not a Kubernetes status, so core has no vocabulary
 * for it; this is the one place that does, and the word and its tone are
 * decided together here so no caller can pair them itself.
 */
export const LINK_WORD: Record<LinkState, string> = {
  connected: "Connected",
  connecting: "Connecting",
  disconnected: "Disconnected",
  error: "Unreachable",
};

export const LINK_TONE: Record<LinkState, Tone> = {
  connected: "ok",
  connecting: "info",
  disconnected: "muted",
  error: "sev",
};

export interface WorkspaceView {
  /** Per cluster. Derived from `ClusterInfo.reachable` and in-flight connects; never persisted. */
  links: Record<string, { state: LinkState; error?: string }>;
  /** Open sidebar groups per stable cluster ID, persisted through settingsStorage. */
  expanded: Record<string, string[]>;
}

/** Live connection status and the reader's persisted per-cluster choices. */
const initial = (): WorkspaceView => ({ links: {}, expanded: {} });
let view: WorkspaceView = initial();
const listeners = new Set<() => void>();

function emit(next: WorkspaceView) {
  view = next;
  for (const l of listeners) l();
}

/** Order is significant: the sidebar renders sections in the order given. */
function sameArray(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

function isInitial(v: WorkspaceView): boolean {
  return Object.keys(v.links).length === 0 && Object.keys(v.expanded).length === 0;
}

export function getView(): WorkspaceView {
  return view;
}

export function resetView(): void {
  defaultSelection = readDefaultSelection();
  if (isInitial(view)) return;
  emit(initial());
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useWorkspaceView(): WorkspaceView {
  return useSyncExternalStore(subscribe, getView, getView);
}

export function setLink(id: string, state: LinkState | undefined, error?: string): void {
  const current = view.links[id];
  if (state === undefined) {
    if (!current) return;
    const links = { ...view.links };
    delete links[id];
    emit({ ...view, links });
    return;
  }
  if (current && current.state === state && current.error === error) return;
  const entry = error === undefined ? { state } : { state, error };
  emit({ ...view, links: { ...view.links, [id]: entry } });
}

export const EXPANDED_KEY = "srelens.next.navigationExpanded";

/** Restore choices before mounting the sidebar. Missing clusters start collapsed. */
export function loadExpanded(storage: Storage = settingsStorage): void {
  let expanded: Record<string, string[]> = {};
  try {
    expanded = parseStoredClusterLists(storage.getItem(EXPANDED_KEY));
  } catch (error) {
    console.error("could not read the saved sidebar groups", error);
  }
  emit({ ...view, expanded });
}

export function toggleExpanded(clusterId: string, id: string, storage: Storage = settingsStorage): void {
  const current = view.expanded[clusterId] ?? [];
  setExpanded(clusterId, current.includes(id) ? current.filter((x) => x !== id) : [...current, id], storage);
}

export function setExpanded(clusterId: string, ids: string[], storage: Storage = settingsStorage): void {
  if (sameArray(view.expanded[clusterId] ?? [], ids)) return;
  emit({ ...view, expanded: { ...view.expanded, [clusterId]: [...ids] } });
  try {
    storage.setItem(EXPANDED_KEY, JSON.stringify(view.expanded));
  } catch (error) {
    console.error("could not persist the sidebar groups", error);
  }
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");

function parseStoredClusterLists(raw: string | null): Record<string, string[]> {
  if (!raw) return {};
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!isRecord(doc)) return {};
  return Object.fromEntries(Object.entries(doc).filter((entry): entry is [string, string[]] => isStringArray(entry[1])));
}

/*
 * Namespace selection is per TAB, and per cluster within the tab — it lives
 * on `Tab.namespaces` in the tab store and persists with the tab. It used to
 * be one selection per cluster, shared by every screen on that cluster, and
 * that is exactly the bug it no longer is: narrowing the pods list in one tab
 * narrowed every other tab on the same cluster behind the reader's back.
 *
 * Which tab: the one the screen is mounted in, from `TabScope`. Outside any
 * tab — the dock — the active tab, which is the one the reader is looking at.
 */

/**
 * Sets a tab's namespace selection for a cluster. `tabId` defaults to the
 * active tab; a screen passes its own through {@link useSetNamespaces}.
 */
export function setNamespaces(clusterId: string, namespaces: string[], tabId: string = currentWorkspace().activeId): void {
  setTabNamespaces(tabId, clusterId, namespaces);
}

/** The setter for the tab this component is mounted in. */
export function useSetNamespaces(): (clusterId: string, namespaces: string[]) => void {
  const tabId = useTabScope();
  return useCallback((clusterId: string, namespaces: string[]) => setNamespaces(clusterId, namespaces, tabId ?? undefined), [tabId]);
}

/** Forget a removed cluster's selection in every tab without disturbing other clusters. */
export function removeNamespaces(clusterId: string): void {
  forgetClusterNamespaces(clusterId);
}

/** A stable empty selection, so an unset cluster's snapshot never changes identity. */
const NO_NAMESPACES: string[] = [];
function readDefaultSelection(): string[] {
  const namespace = getDefaultNamespace();
  return namespace ? [namespace] : NO_NAMESPACES;
}
let defaultSelection = readDefaultSelection();

/** A fallback affects only clusters with no explicit selection, including “all”. */
export function setNamespaceDefault(namespace: string): void {
  setDefaultNamespace(namespace);
  defaultSelection = readDefaultSelection();
  emit({ ...view });
}

function subscribeSelection(listener: () => void): () => void {
  const offTabs = subscribeTabs(listener);
  const offView = subscribe(listener);
  const offScopes = subscribeScopes(listener);
  return () => {
    offTabs();
    offView();
    offScopes();
  };
}

// A namespace collection renamed in Settings takes the tabs scoped to it along.
onCollectionRenamed((type, from, to) => {
  if (type === "namespace") renameNamespaceCollection(from, to);
});

/**
 * A tab's selection for a cluster: what its namespace collections resolve
 * to, where it is scoped to any that still exist (#881), else the namespaces
 * it picked, else the default.
 *
 * A collection that resolves to nothing — or has not resolved yet — is
 * `SELECTS_NOTHING`, which lists nothing, never the empty selection, which
 * lists everything. If every collection a tab named has been deleted, the
 * tab falls back to the default namespace, as if it had never chosen.
 */
function selectionFor(tabId: string, clusterId: string): string[] {
  const names = tabNamespaceCollections(tabId, clusterId);
  if (names) {
    const scope = collectionScope(clusterId, names);
    if (scope.status !== "gone") return scope.selection as string[];
  }
  return tabNamespaces(tabId, clusterId) ?? defaultSelection;
}

/** This tab's namespace collections on a cluster, by name; `undefined` when it has none. */
export function useNamespaceCollections(clusterId: string | undefined): string[] | undefined {
  const scoped = useTabScope();
  const read = () => (clusterId === undefined ? undefined : tabNamespaceCollections(scoped ?? currentWorkspace().activeId, clusterId));
  return useSyncExternalStore(subscribeTabs, read, () => undefined);
}

/** Whether a tab is scoped, on a cluster, to a namespace collection that still exists. */
function inCollectionScope(tabId: string, clusterId: string): boolean {
  const names = tabNamespaceCollections(tabId, clusterId);
  return names !== undefined && collectionScope(clusterId, names).status !== "gone";
}

/**
 * Whether this tab is scoped to a namespace collection on the cluster — one
 * that still exists: a tab whose collections have all been deleted is not.
 */
export function useInCollectionScope(clusterId: string | undefined): boolean {
  const scoped = useTabScope();
  const read = () => clusterId !== undefined && inCollectionScope(scoped ?? currentWorkspace().activeId, clusterId);
  return useSyncExternalStore(subscribeSelection, read, () => false);
}

/**
 * Scope this tab to the one namespace restricted credentials can see, as
 * each list does when it loads: they cannot list namespaces, and a request
 * across all of them would be refused.
 *
 * That is not the reader choosing, and a tab scoped to a namespace
 * collection keeps its collection (#881) — it already resolves against the
 * one namespace the listing has, and says the result may be partial. Once
 * that collection is deleted the tab is not scoped any more, and the
 * credentials' namespace is applied again, rather than leaving the list to
 * ask for every namespace and be refused.
 */
export function useFallbackNamespace(clusterId: string, namespace: string): void {
  const tabId = useTabScope();
  // Not "is it scoped" but "does it name collections that are all gone": the
  // reader leaving a collection for namespaces of their own makes it unscoped
  // too, and that choice must stand. Only a deletion should bring the
  // credentials' namespace back.
  const orphaned = useOrphanedCollections(clusterId);
  useEffect(() => {
    const tab = tabId ?? currentWorkspace().activeId;
    if (!namespace || inCollectionScope(tab, clusterId)) return;
    setTabNamespaces(tab, clusterId, [namespace]);
  }, [namespace, clusterId, tabId, orphaned]);
}

/** Whether this tab names collections on the cluster, every one of which has been deleted. */
function useOrphanedCollections(clusterId: string): boolean {
  const scoped = useTabScope();
  const read = () => {
    const tab = scoped ?? currentWorkspace().activeId;
    return tabNamespaceCollections(tab, clusterId) !== undefined && !inCollectionScope(tab, clusterId);
  };
  return useSyncExternalStore(subscribeSelection, read, () => false);
}

/** The collections setter for the tab this component is mounted in. */
export function useSetNamespaceCollections(): (clusterId: string, names: string[]) => void {
  const tabId = useTabScope();
  return useCallback((clusterId: string, names: string[]) => {
    setTabNamespaceCollections(tabId ?? currentWorkspace().activeId, clusterId, names);
    markCollectionsUsed("namespace", names);
  }, [tabId]);
}

/**
 * This tab's namespace selection for the cluster, re-rendering whoever reads
 * it when it changes — and only this tab's: another tab narrowing the same
 * cluster changes nothing here.
 */
export function useNamespaces(clusterId: string | undefined): string[] {
  const scoped = useTabScope();
  const read = () => {
    if (clusterId === undefined) return NO_NAMESPACES;
    return selectionFor(scoped ?? currentWorkspace().activeId, clusterId);
  };
  return useSyncExternalStore(subscribeSelection, read, () => NO_NAMESPACES);
}
