import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * The subjects a bare `/logs` offers as a way in.
 *
 * The doors that carry a subject are the resource row menu's *Follow logs* and
 * the detail pane's *Logs*; the sidebar's bare `/logs` carries nothing, and
 * without this it is a screen that can only say "pick something" and offer no
 * way to. What it offers is what this reader has actually streamed.
 *
 * Same shape as `lib/marks.ts`, `lib/peekWidth.ts` and the workspace's
 * namespace selection, and for the same three reasons: `settingsStorage` by
 * default so the desktop writes the backend's settings file and the web writes
 * `localStorage`, injectable so tests need a Map and no platform, and every
 * accessor wrapped because `localStorage` in a WebView with storage disabled
 * does not return null — it throws outright.
 *
 * Keyed by `ClusterContext.stableId`, the id and never the name, so a context
 * renamed in the kubeconfig keeps the subjects the reader followed on it.
 */
export const RECENT_LOGS_KEY = "srelens.next.recentLogs";

/**
 * How many are kept. Enough to cover a shift's worth of what one reader keeps
 * coming back to, and few enough that the empty state stays an empty state
 * rather than becoming a second resource list.
 */
export const MAX_RECENT_LOGS = 8;

/** A subject that was streamed, and the cluster it was streamed on. */
export interface RecentLogSubject {
  /** `ClusterContext.stableId`. */
  cluster: string;
  /** `Pod` for a single pod; any workload kind `getObject` understands otherwise. */
  kind: string;
  namespace: string;
  name: string;
}

/** The entry's identity — the four fields that make it the subject it is. */
export function recentKey(entry: RecentLogSubject): string {
  return `${entry.cluster}\u0000${entry.kind}\u0000${entry.namespace}\u0000${entry.name}`;
}

/**
 * The one list that answers for an entry: every remembered subject of the same
 * kind in the same namespace is checked by one `listResource`, so a screenful
 * of recents costs a call per kind/namespace pair rather than one per name.
 */
export function scanKey(entry: RecentLogSubject): string {
  return `${entry.kind}\u0000${entry.namespace}`;
}

/** Whether the subject is a single pod — `LogsSubject` reads the route's kind
 *  exactly this way, and the two must not disagree about what a pod is. */
export function isPodSubject(entry: RecentLogSubject): boolean {
  return entry.kind.toLowerCase() === "pod";
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isName = (v: unknown): v is string => typeof v === "string" && v !== "";

function parseEntry(v: unknown): RecentLogSubject | null {
  if (!isRecord(v)) return null;
  if (!isName(v.cluster) || !isName(v.kind) || !isName(v.namespace) || !isName(v.name)) return null;
  return { cluster: v.cluster, kind: v.kind, namespace: v.namespace, name: v.name };
}

/**
 * Anything but a list of subjects reads as no subjects at all.
 *
 * A list rather than the map the marks use, because order IS the data here:
 * most recent first. One entry this build cannot read is dropped on its own
 * rather than taking the others with it — an entry missing a field is a
 * nuisance, an empty list is the screen having nothing to offer. Deduped and
 * capped on the way in as well as on the way out, so a document written by a
 * future build with a larger cap cannot hand this one a longer list than it
 * would ever write.
 */
export function parseStoredRecents(raw: string | null): RecentLogSubject[] {
  if (!raw) return [];
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(doc)) return [];
  const seen = new Set<string>();
  const entries: RecentLogSubject[] = [];
  for (const value of doc) {
    const entry = parseEntry(value);
    if (!entry) continue;
    const key = recentKey(entry);
    if (seen.has(key)) continue;
    seen.add(key);
    entries.push(entry);
    if (entries.length === MAX_RECENT_LOGS) break;
  }
  return entries;
}

let recents: RecentLogSubject[] = [];
const listeners = new Set<() => void>();

/**
 * One array per cluster, kept until the list changes.
 *
 * `recentLogSubjects` composes its answer by filtering, so without this it
 * hands `useSyncExternalStore` a fresh array on every read — which is an
 * infinite re-render, not a subtle inefficiency. Cleared whenever the list is
 * replaced, so it cannot outgrow the clusters.
 */
const snapshots = new Map<string, readonly RecentLogSubject[]>();

function emit() {
  snapshots.clear();
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function save(storage: Storage) {
  try {
    storage.setItem(RECENT_LOGS_KEY, JSON.stringify(recents));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: a way in that does not
    // survive the session is better than one that cannot be offered at all.
    console.error("could not persist the recently followed logs", error);
  }
}

/**
 * Read the saved subjects once at boot — and in tests, as often as they like.
 *
 * Guarded like every accessor in `marks.ts`/`peekWidth.ts`. Boot must reach
 * `setBooted(true)`, so a refusing storage costs the recents and nothing else.
 */
export function loadRecentLogSubjects(storage: Storage = settingsStorage): void {
  let next: RecentLogSubject[] = [];
  try {
    next = parseStoredRecents(storage.getItem(RECENT_LOGS_KEY));
  } catch (error) {
    console.error("could not read the recently followed logs", error);
  }
  recents = next;
  emit();
}

/**
 * This subject was just streamed. Front of the list, once — a subject followed
 * again is the same subject, not a second entry — and the oldest fall off the
 * end.
 */
export function rememberLogSubject(entry: RecentLogSubject, storage: Storage = settingsStorage): void {
  const key = recentKey(entry);
  const rest = recents.filter((e) => recentKey(e) !== key);
  // Identical head and no eviction means nothing to write and nobody to wake:
  // the screen re-resolves its subject on every remount, and re-persisting the
  // same document each time is a write per tab switch.
  if (rest.length === recents.length - 1 && recents[0] !== undefined && recentKey(recents[0]) === key) return;
  recents = [entry, ...rest].slice(0, MAX_RECENT_LOGS);
  emit();
  save(storage);
}

/** Forget these subjects — see {@link reviewRecents} for the only thing that
 *  asks, and why it only ever asks about pods. */
export function forgetLogSubjects(keys: readonly string[], storage: Storage = settingsStorage): void {
  if (keys.length === 0) return;
  const dropped = new Set(keys);
  const next = recents.filter((e) => !dropped.has(recentKey(e)));
  if (next.length === recents.length) return;
  recents = next;
  emit();
  save(storage);
}

/** A stable empty list, so a cluster with no recents never changes identity. */
const NONE: readonly RecentLogSubject[] = [];

/** What this cluster has streamed, most recent first. */
export function recentLogSubjects(cluster: string): readonly RecentLogSubject[] {
  const cached = snapshots.get(cluster);
  if (cached) return cached;
  const mine = recents.filter((e) => e.cluster === cluster);
  const answer = mine.length === 0 ? NONE : mine;
  snapshots.set(cluster, answer);
  return answer;
}

/** The cluster's recents, re-rendering whoever reads them when they change. */
export function useRecentLogSubjects(cluster: string): readonly RecentLogSubject[] {
  return useSyncExternalStore(
    subscribe,
    () => recentLogSubjects(cluster),
    () => recentLogSubjects(cluster),
  );
}

/**
 * What one `listResource` said about a kind in a namespace: the names it
 * found, or that it could not be read.
 *
 * A failure is kept as a failure rather than collapsed to an empty list —
 * "the cluster has none of these" and "nobody could ask the cluster" lead to
 * opposite decisions below, and an empty list would quietly turn every
 * remembered subject into a dead one the moment a VPN dropped.
 */
export type SubjectScan = { names: readonly string[] } | { error: true };

/**
 * How sure this screen is that the subject is still there. `unverified` is not
 * a hedge, it is the honest answer for a subject nobody could check.
 */
export type RecentPresence = "present" | "gone" | "unverified";

export interface OfferedRecent {
  entry: RecentLogSubject;
  presence: RecentPresence;
}

/**
 * Which remembered subjects to offer, and which to stop remembering.
 *
 * **A remembered subject that no longer exists must not be offered as though
 * it does.** A list of dead pods that error when clicked is worse than no list
 * — the reader trusted it. So an entry is offered only once something is known
 * about it: an entry with no scan yet is left out entirely, which is the same
 * rule `selectionIsStale` follows for a namespace list that has not answered
 * (`namespaces === null` is not evidence of anything).
 *
 * **A pod and a workload are not the same kind of missing.** A pod's name
 * carries its replica-set hash and a random suffix, so a pod the cluster has
 * replaced — which is every pod, on every deploy — is gone for good and its
 * name will never name anything again: it is dropped from the list and
 * forgotten, or the cap silts up with corpses and pushes the live workloads
 * out. A Deployment's name outlives its pods; one that is missing has been
 * scaled away, moved or not yet re-applied, and may well come back under
 * exactly this name. That is worth telling the reader about — the namespace
 * selection's stale banner says the same thing about a namespace — so it is
 * kept, shown, and not offered.
 *
 * Nothing is ever forgotten on a scan that failed. Losing what the reader
 * followed because the cluster was briefly unreachable is a bad trade against
 * a list that is briefly optimistic.
 */
export function reviewRecents(
  entries: readonly RecentLogSubject[],
  scans: ReadonlyMap<string, SubjectScan>,
): { offered: OfferedRecent[]; forget: RecentLogSubject[] } {
  const offered: OfferedRecent[] = [];
  const forget: RecentLogSubject[] = [];
  for (const entry of entries) {
    const scan = scans.get(scanKey(entry));
    if (scan === undefined) continue;
    if ("error" in scan) {
      offered.push({ entry, presence: "unverified" });
      continue;
    }
    if (scan.names.includes(entry.name)) {
      offered.push({ entry, presence: "present" });
      continue;
    }
    if (isPodSubject(entry)) forget.push(entry);
    else offered.push({ entry, presence: "gone" });
  }
  return { offered, forget };
}
