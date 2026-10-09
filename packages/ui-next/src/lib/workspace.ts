import { useCallback, useSyncExternalStore } from "react";
import { getDefaultNamespace, setDefaultNamespace, settingsStorage } from "@srelens/core";
import type { Tone } from "@srelens/ui-kit";
import type { Storage } from "./tabsPersist";
import { currentWorkspace, forgetClusterNamespaces, setTabNamespaces, subscribe as subscribeTabs, tabNamespaces } from "./tabsStore";
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
  return () => {
    offTabs();
    offView();
  };
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
    return tabNamespaces(scoped ?? currentWorkspace().activeId, clusterId) ?? defaultSelection;
  };
  return useSyncExternalStore(subscribeSelection, read, () => NO_NAMESPACES);
}
