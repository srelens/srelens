import { useSyncExternalStore } from "react";
import { getDefaultNamespace, setDefaultNamespace, settingsStorage } from "@srelens/core";
import type { Tone } from "@srelens/ui-kit";
import type { Storage } from "./tabsPersist";

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
  /**
   * Namespace selection per cluster, keyed by `ClusterContext.stableId`, never
   * a display name — a context renamed in the kubeconfig keeps its selection.
   * One selection per cluster, shared by every screen looking at that
   * cluster, rather than one per tab. An empty array means "all namespaces",
   * and so does a cluster with no entry at all — a cluster is only ever added
   * here when something narrows it, never seeded up front. This is persisted
   * (`loadNamespaces`/`setNamespaces`,
   * through `settingsStorage`, the same as `marks.ts` and `columnPrefs.ts`):
   * a namespace selection is a standing choice about what a reader wants to
   * see, not a fact about this sitting.
   */
  namespaces: Record<string, string[]>;
}

/** Live connection status and the reader's persisted per-cluster choices. */
const initial = (): WorkspaceView => ({ links: {}, expanded: {}, namespaces: {} });
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
  return Object.keys(v.links).length === 0 && Object.keys(v.expanded).length === 0 && Object.keys(v.namespaces).length === 0;
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

export function setLink(id: string, state: LinkState, error?: string): void {
  const current = view.links[id];
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

export const NAMESPACES_KEY = "srelens.next.namespaces";

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");

/**
 * Anything but a map of `stableId -> namespace names` reads as no stored
 * selection at all. One cluster's entry that is not a string array is
 * dropped on its own rather than taking the rest of the document with it —
 * losing one cluster's remembered namespaces is a nuisance, losing every
 * cluster's is not. An empty array is an explicit all-namespaces choice;
 * a missing entry follows the global default.
 */
export const parseStoredNamespaces = parseStoredClusterLists;

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

function saveNamespaces(storage: Storage) {
  try {
    storage.setItem(NAMESPACES_KEY, JSON.stringify(view.namespaces));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: a selection that does not
    // survive the session is better than a selection that cannot be set.
    console.error("could not persist the namespace selection", error);
  }
}

/**
 * Read the saved namespace selections once at boot — and in tests, as often
 * as they like.
 *
 * Guarded like every accessor in `marks.ts`/`columnPrefs.ts`: the settings adapter
 * can refuse reads if backend initialization failed. Boot
 * must survive it, so a refusing storage costs the remembered selections and
 * nothing else. Merged onto the current view rather than replacing it, so a
 * `links`/`expanded` set before boot finishes reading storage is not undone —
 * neither is written here, but both could already be set.
 */
export function loadNamespaces(storage: Storage = settingsStorage): void {
  defaultSelection = readDefaultSelection();
  let next: Record<string, string[]> = {};
  try {
    next = parseStoredNamespaces(storage.getItem(NAMESPACES_KEY));
  } catch (error) {
    console.error("could not read the saved namespace selections", error);
  }
  emit({ ...view, namespaces: next });
}

/**
 * Sets a cluster's namespace selection. Per cluster, not per tab: two tabs on
 * the same cluster agree, because both read this same record.
 */
export function setNamespaces(clusterId: string, namespaces: string[], storage: Storage = settingsStorage): void {
  const current = view.namespaces[clusterId];
  if (current && sameArray(current, namespaces)) return;
  emit({ ...view, namespaces: { ...view.namespaces, [clusterId]: [...namespaces] } });
  saveNamespaces(storage);
}

/** Forget a removed cluster's namespace without disturbing other clusters. */
export function removeNamespaces(clusterId: string, storage: Storage = settingsStorage): void {
  if (!(clusterId in view.namespaces)) return;
  const namespaces = { ...view.namespaces };
  delete namespaces[clusterId];
  emit({ ...view, namespaces });
  saveNamespaces(storage);
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

/** The cluster's namespace selection, re-rendering whoever reads it when it changes. */
export function useNamespaces(clusterId: string | undefined): string[] {
  return useSyncExternalStore(
    subscribe,
    () => (clusterId === undefined ? NO_NAMESPACES : (view.namespaces[clusterId] ?? defaultSelection)),
    () => NO_NAMESPACES,
  );
}
