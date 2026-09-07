import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { MarkAppearance } from "@srelens/ui-kit";
import type { Storage } from "./tabsPersist";

/**
 * How each cluster's mark looks, remembered between launches.
 *
 * The kit draws a {@link MarkAppearance} and the shell has to decide what to
 * hand it, so the decision lives here: one module-level record keyed by
 * `ClusterContext.stableId` — the id, never the name, because the whole point
 * is that a context renamed in the kubeconfig keeps the colour its operator
 * gave it.
 *
 * Same shape as `tabsPersist`: `settingsStorage` by default so the desktop
 * writes the backend's settings file and the web writes `localStorage`, but
 * injectable so tests need a Map and no platform.
 */
export const MARKS_KEY = "srelens.next.marks";

/**
 * `prod-eu` → `PE`, `staging` → `ST`.
 *
 * The first letter of each of the first two parts, or the first two letters
 * when there is only one part, so that a single-word name is still told apart
 * from its neighbours. Capped at what {@link MarkAppearance.short} can draw.
 */
export function initials(name: string): string {
  const parts = name.split(/[-_ ]+/).filter(Boolean);
  if (parts.length === 0) return "";
  const letters = parts.length === 1 ? parts[0].slice(0, 2) : parts[0][0] + parts[1][0];
  return letters.toUpperCase().slice(0, 3);
}

/**
 * What a cluster looks like before anyone has customised it.
 *
 * The indigo of the mark palette rather than `var(--accent)`, which is what
 * this used to be: the accent moves with the accent axis, so an uncustomised
 * mark changed colour for anyone who preferred a blue accent, and — because
 * the editor's swatches are radios compared by value — the palette then had
 * nothing checked and no tab stop at all until a colour was picked. The mark
 * tokens are identity rather than meaning and move with nothing.
 */
export function defaultMark(name: string): MarkAppearance {
  return { name, short: initials(name), color: "var(--mark-indigo)", mark: "text", withText: true };
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isString = (v: unknown): v is string => typeof v === "string";

function parseMark(v: unknown): MarkAppearance | null {
  if (!isRecord(v) || !isString(v.name) || !isString(v.short) || !isString(v.color)) return null;
  if (v.mark !== "text" && v.mark !== "icon" && v.mark !== "image") return null;
  const mark: MarkAppearance = { name: v.name, short: v.short, color: v.color, mark: v.mark, withText: v.withText !== false };
  if (isString(v.icon)) mark.icon = v.icon;
  if (isString(v.imageSrc)) mark.imageSrc = v.imageSrc;
  return mark;
}

/**
 * Anything but a map of marks reads as no marks at all.
 *
 * Unlike the workspaces there is no version here and nothing to migrate: the
 * document is a flat `stableId -> appearance` map, and a mark this build
 * cannot read is dropped on its own rather than taking the others with it —
 * losing one cluster's colour is a nuisance, losing all of them is not.
 */
export function parseStoredMarks(raw: string | null): Record<string, MarkAppearance> {
  if (!raw) return {};
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!isRecord(doc)) return {};
  const marks: Record<string, MarkAppearance> = {};
  for (const [id, value] of Object.entries(doc)) {
    const mark = parseMark(value);
    if (mark) marks[id] = mark;
  }
  return marks;
}

let marks: Record<string, MarkAppearance> = {};
const listeners = new Set<() => void>();

/**
 * `getMark` composes its answer, so it has to hand back the *same* object
 * every time nothing has changed: `useSyncExternalStore` tears down and
 * re-renders forever on a snapshot that is a fresh object on every read, and
 * both the unstored default and the rename patch below are fresh objects.
 * Cleared whenever the record is replaced, so it cannot outgrow the clusters.
 */
const snapshots = new Map<string, MarkAppearance>();

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
 * Read the saved marks once at boot — and in tests, as often as they like.
 *
 * Guarded like every accessor in `tabsPersist`: `settingsStorage` falls back
 * to raw `localStorage` when the backend file is unavailable, and
 * `localStorage` throws outright in a WebView with storage disabled. Boot
 * must reach `setBooted(true)`, so a refusing storage costs the colours and
 * nothing else.
 */
export function loadMarks(storage: Storage = settingsStorage): void {
  let next: Record<string, MarkAppearance> = {};
  try {
    next = parseStoredMarks(storage.getItem(MARKS_KEY));
  } catch (error) {
    console.error("could not read the saved cluster marks", error);
  }
  marks = next;
  emit();
}

function save(storage: Storage) {
  try {
    storage.setItem(MARKS_KEY, JSON.stringify(marks));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: an appearance that does not
    // survive the session is better than an appearance that cannot be set.
    console.error("could not persist the cluster marks", error);
  }
}

/**
 * The cluster's mark: the stored appearance if there is one, and otherwise a
 * default seeded from the name the kubeconfig gives the context.
 *
 * A stored mark comes back exactly as stored, `name` included. That name is a
 * display name the operator typed, not a cache of the context's. This used to
 * overwrite it with the `name` argument on the way out, which made the editor's
 * name field inert: every keystroke was stored and then reverted on the very
 * next read (#325 review). `short` is not re-derived either, for the same
 * reason — an edit nobody asked to undo should not be undone.
 *
 * So a cluster renamed in the kubeconfig follows that rename only while nobody
 * has customised it, which is the case the rename mattered for; once someone
 * has named it themselves, that is its name.
 */
export function getMark(stableId: string, name: string): MarkAppearance {
  // Keyed on both, because the unstored answer depends on the name. A stored
  // mark ignores it, and every key then hands back that same one object.
  const key = `${stableId}\u0000${name}`;
  const cached = snapshots.get(key);
  if (cached) return cached;
  const mark = marks[stableId] ?? defaultMark(name);
  snapshots.set(key, mark);
  return mark;
}

/** Give a cluster this appearance, and keep it. */
export function setMark(stableId: string, mark: MarkAppearance, storage: Storage = settingsStorage): void {
  marks = { ...marks, [stableId]: mark };
  emit();
  save(storage);
}

/** Forget a cluster's appearance, putting it back to {@link defaultMark}. */
export function resetMark(stableId: string, storage: Storage = settingsStorage): void {
  if (!(stableId in marks)) return;
  const { [stableId]: _dropped, ...rest } = marks;
  marks = rest;
  emit();
  save(storage);
}

/** The cluster's mark, re-rendering whoever reads it when the mark changes. */
export function useMark(stableId: string, name: string): MarkAppearance {
  return useSyncExternalStore(
    subscribe,
    () => getMark(stableId, name),
    () => getMark(stableId, name),
  );
}
