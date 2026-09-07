import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
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
  /** Which sidebar sections are open. Not persisted. */
  expanded: string[];
  /**
   * Namespace selection per cluster, keyed by `ClusterContext.stableId`, never
   * a display name — a context renamed in the kubeconfig keeps its selection.
   * One selection per cluster, shared by every screen looking at that
   * cluster, rather than one per tab. An empty array means "all namespaces",
   * and so does a cluster with no entry at all — a cluster is only ever added
   * here when something narrows it, never seeded up front. Unlike `links` and
   * `expanded`, this *is* persisted (`loadNamespaces`/`setNamespaces`,
   * through `settingsStorage`, the same as `marks.ts` and `columnPrefs.ts`):
   * a namespace selection is a standing choice about what a reader wants to
   * see, not a fact about this sitting.
   */
  namespaces: Record<string, string[]>;
}

/**
 * What the current workspace looks like right now, as distinct from what it
 * contains. The tab store owns clusters, tabs and the active cluster and is
 * written to disk; this owns three things, kept for two different reasons.
 * `links` and `expanded` should not outlive the window — a cluster's
 * reachability is a fact about now and an expanded section is a fact about
 * this sitting — so neither is ever read from or written to storage.
 * `namespaces` is the odd one out: a namespace selection is a standing
 * choice about what a reader wants to see, so it alone survives a restart.
 * (It was added to this struct later and inherited non-persistence by
 * accident of where it lives rather than by that argument — see
 * `loadNamespaces` below for where it actually persists.)
 */
const initial = (): WorkspaceView => ({ links: {}, expanded: [], namespaces: {} });
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
  return Object.keys(v.links).length === 0 && v.expanded.length === 0 && Object.keys(v.namespaces).length === 0;
}

export function getView(): WorkspaceView {
  return view;
}

/**
 * Whether {@link seedExpandedOnce} has already run for this window's
 * lifetime. Module-level rather than a ref kept on `Nav`: a ref resets every
 * time the component remounts, so a ref-guarded seed cannot tell "nothing has
 * opened a group yet" from "the user just closed all of them" — both show up
 * as an empty `expanded` on the next mount. A flag that survives remounts is
 * what makes the two distinguishable. `resetView` clears it alongside the
 * rest of the view because tests use one call to `resetView` as "a fresh
 * window"; production never calls `resetView` at all.
 */
let seeded = false;

export function resetView(): void {
  seeded = false;
  if (isInitial(view)) return;
  emit(initial());
}

/**
 * Seeds `expanded` with `ids`, but only the first time this is ever called
 * for the running window — not once per mount of whatever calls it. Everything
 * else about the sidebar's folds already works whether `Nav` is mounted once
 * or remounted a dozen times; this is the one piece of it that must not.
 */
export function seedExpandedOnce(ids: string[]): void {
  if (seeded) return;
  seeded = true;
  if (view.expanded.length === 0) setExpanded(ids);
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

export function toggleExpanded(id: string): void {
  const expanded = view.expanded.includes(id) ? view.expanded.filter((x) => x !== id) : [...view.expanded, id];
  emit({ ...view, expanded });
}

export function setExpanded(ids: string[]): void {
  if (sameArray(view.expanded, ids)) return;
  emit({ ...view, expanded: [...ids] });
}

export const NAMESPACES_KEY = "srelens.next.namespaces";

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");

/**
 * Anything but a map of `stableId -> namespace names` reads as no stored
 * selection at all. One cluster's entry that is not a string array is
 * dropped on its own rather than taking the rest of the document with it —
 * losing one cluster's remembered namespaces is a nuisance, losing every
 * cluster's is not. An entry that is a valid but empty array is dropped too,
 * for the same reason `setNamespaces` never writes one: it means the same
 * thing as no entry at all, and a document should not accumulate one per
 * cluster a reader ever looked at.
 */
export function parseStoredNamespaces(raw: string | null): Record<string, string[]> {
  if (!raw) return {};
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!isRecord(doc)) return {};
  const namespaces: Record<string, string[]> = {};
  for (const [id, value] of Object.entries(doc)) {
    if (isStringArray(value) && value.length > 0) namespaces[id] = value;
  }
  return namespaces;
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
 * Guarded like every accessor in `marks.ts`/`columnPrefs.ts`: `settingsStorage`
 * falls back to raw `localStorage` when the backend file is unavailable, and
 * `localStorage` throws outright in a WebView with storage disabled. Boot
 * must survive it, so a refusing storage costs the remembered selections and
 * nothing else. Merged onto the current view rather than replacing it, so a
 * `links`/`expanded` set before boot finishes reading storage is not undone —
 * neither is ever written here, but both could in principle already be set.
 */
export function loadNamespaces(storage: Storage = settingsStorage): void {
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
  if (namespaces.length === 0) {
    if (!current) return;
    const { [clusterId]: _dropped, ...rest } = view.namespaces;
    emit({ ...view, namespaces: rest });
    saveNamespaces(storage);
    return;
  }
  if (current && sameArray(current, namespaces)) return;
  emit({ ...view, namespaces: { ...view.namespaces, [clusterId]: [...namespaces] } });
  saveNamespaces(storage);
}

/** A stable empty selection, so an unset cluster's snapshot never changes identity. */
const NO_NAMESPACES: string[] = [];

/** The cluster's namespace selection, re-rendering whoever reads it when it changes. */
export function useNamespaces(clusterId: string | undefined): string[] {
  return useSyncExternalStore(
    subscribe,
    () => (clusterId === undefined ? NO_NAMESPACES : (view.namespaces[clusterId] ?? NO_NAMESPACES)),
    () => NO_NAMESPACES,
  );
}
