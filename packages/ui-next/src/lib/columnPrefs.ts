import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * Which columns a resource-list screen hides, remembered between launches.
 *
 * This is `lib/marks.ts` with a different payload: a hidden-columns list is a
 * preference about a *kind* — "pods", "deployments" — not about a tab, so it
 * lives here in one module-level record keyed by kind, and persists through
 * `settingsStorage` the same way a mark does: the desktop's backend settings
 * file, or `localStorage` on the web, injectable so tests need a Map and no
 * platform.
 */
export const COLUMN_PREFS_KEY = "srelens.next.columns";

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isString = (v: unknown): v is string => typeof v === "string";

/**
 * Anything but a map of kind -> column keys reads as no hidden columns at
 * all. A kind whose value is not an array is dropped on its own, and a
 * non-string entry inside an otherwise good array is dropped individually —
 * losing one kind's or one column's preference is a nuisance, losing every
 * kind's is not.
 */
export function parseStoredColumnPrefs(raw: string | null): Record<string, string[]> {
  if (!raw) return {};
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!isRecord(doc)) return {};
  const prefs: Record<string, string[]> = {};
  for (const [kind, value] of Object.entries(doc)) {
    if (!Array.isArray(value)) continue;
    prefs[kind] = value.filter(isString);
  }
  return prefs;
}

let prefs: Record<string, string[]> = {};
const listeners = new Set<() => void>();

/**
 * `hiddenColumns` composes its answer, so it has to hand back the *same* set
 * every time nothing has changed: `useSyncExternalStore` tears down and
 * re-renders forever on a snapshot that is a fresh `Set` on every read.
 * Cleared whenever the record is replaced, so it cannot outgrow the kinds.
 */
const snapshots = new Map<string, ReadonlySet<string>>();

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

/**
 * Read the saved column preferences once at boot — and in tests, as often as
 * they like.
 *
 * Guarded like every accessor in `marks.ts`/`tabsPersist.ts`: `settingsStorage`
 * falls back to raw `localStorage` when the backend file is unavailable, and
 * `localStorage` throws outright in a WebView with storage disabled. Boot must
 * reach `setBooted(true)`, so a refusing storage costs the preferences and
 * nothing else.
 *
 * Also the fix for a bug this codebase already shipped once: a module that
 * never loads what is on disk spreads over an empty record on its first
 * write and erases every other kind's stored entry. Calling this at boot is
 * what keeps `toggleColumn`'s spread starting from the real, saved record.
 */
export function loadColumnPrefs(storage: Storage = settingsStorage): void {
  let next: Record<string, string[]> = {};
  try {
    next = parseStoredColumnPrefs(storage.getItem(COLUMN_PREFS_KEY));
  } catch (error) {
    console.error("could not read the saved column preferences", error);
  }
  prefs = next;
  emit();
}

function save(storage: Storage) {
  try {
    storage.setItem(COLUMN_PREFS_KEY, JSON.stringify(prefs));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: a preference that does not
    // survive the session is better than a preference that cannot be set.
    console.error("could not persist the column preferences", error);
  }
}

/** The columns hidden for a kind, as a stable set until it next changes. */
export function hiddenColumns(kind: string): ReadonlySet<string> {
  const cached = snapshots.get(kind);
  if (cached) return cached;
  const set = new Set(prefs[kind] ?? []);
  snapshots.set(kind, set);
  return set;
}

/** Hide a shown column, or show a hidden one, for one kind. */
export function toggleColumn(kind: string, key: string, storage: Storage = settingsStorage): void {
  const next = new Set(prefs[kind] ?? []);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  prefs = { ...prefs, [kind]: [...next] };
  emit();
  save(storage);
}

/** Forget a kind's hidden columns, putting it back to showing all of them. */
export function resetColumns(kind: string, storage: Storage = settingsStorage): void {
  if (!(kind in prefs)) return;
  const { [kind]: _dropped, ...rest } = prefs;
  prefs = rest;
  emit();
  save(storage);
}

/** The columns hidden for a kind, re-rendering whoever reads it when they change. */
export function useHiddenColumns(kind: string): ReadonlySet<string> {
  return useSyncExternalStore(
    subscribe,
    () => hiddenColumns(kind),
    () => hiddenColumns(kind),
  );
}
