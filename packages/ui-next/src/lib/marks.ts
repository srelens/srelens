import { useSyncExternalStore } from "react";
import { avatarColor, avatarInitials, migrateRecordKeys, loadContextProfiles, saveContextProfiles, settingsStorage, type ClusterContext, type ContextProfile, type ContextProfiles } from "@srelens/core";
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

/** Same deterministic initials and colours as the classic design. */
export const initials = avatarInitials;

export function defaultMark(name: string): MarkAppearance {
  return { name, short: initials(name), color: avatarColor(name), mark: "text", withText: true };
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
let profiles: ContextProfiles = {};
const contextNames = new Map<string, string>();
const LEGACY_ICONS = new Set(["cluster", "cloud", "shield", "database", "globe"]);

function withProfile(stableId: string, name: string, base: MarkAppearance): MarkAppearance {
  const profile = profiles[stableId] ?? profiles[name];
  if (!profile || typeof profile !== "object" || Array.isArray(profile)) return base;
  const mark = { ...defaultMark(name) };
  if (typeof profile.displayName === "string") mark.name = profile.displayName;
  if (typeof profile.shortName === "string") mark.short = profile.shortName;
  if (typeof profile.color === "string" && profile.color) mark.color = profile.color;
  if (profile.logo === "initials") mark.mark = "text";
  else if (profile.logo === "custom") { mark.mark = "image"; mark.imageSrc = profile.logoUrl; }
  else if (profile.logo && LEGACY_ICONS.has(profile.logo)) {
    mark.mark = "icon"; mark.icon = profile.markIcon || profile.logo;
  }
  if (profile.logo) mark.withText = profile.showShortName ?? (typeof profile.shortName === "string" && !!profile.shortName.trim());
  return mark;
}

function sharedProfile(stableId: string, mark: MarkAppearance): ContextProfile {
  const contextName = contextNames.get(stableId) ?? mark.name;
  const generatedShort = initials(contextName);
  const legacyIcon = mark.icon && LEGACY_ICONS.has(mark.icon) ? mark.icon as ContextProfile["logo"] : "cluster";
  return {
    displayName: mark.name === contextName ? undefined : mark.name,
    shortName: mark.short === generatedShort ? undefined : mark.short,
    color: mark.color,
    logo: mark.mark === "text" ? "initials" : mark.mark === "image" ? "custom" : legacyIcon,
    logoUrl: mark.imageSrc,
    markIcon: mark.mark === "icon" && !LEGACY_ICONS.has(mark.icon ?? "") ? mark.icon : undefined,
    showShortName: mark.withText,
  };
}
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
  profiles = loadContextProfiles(storage);
  contextNames.clear();
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

/** Display labels match classic: trim custom names and fall back when blank. */
export function getMark(stableId: string, name: string): MarkAppearance {
  return readMark(stableId, name, false);
}

function readMark(stableId: string, name: string, editing: boolean): MarkAppearance {
  // Raw editor values and display labels each need a stable snapshot. The
  // fallback also depends on the current kubeconfig name.
  contextNames.set(stableId, name);
  const key = `${editing}\u0000${stableId}\u0000${name}`;
  const cached = snapshots.get(key);
  if (cached) return cached;
  const saved = withProfile(stableId, name, marks[stableId] ?? defaultMark(name));
  const mark = editing ? saved : { ...saved, name: saved.name.trim() || name };
  snapshots.set(key, mark);
  return mark;
}

/** Give a cluster this appearance, and keep it. */
export function setMark(stableId: string, mark: MarkAppearance, storage: Storage = settingsStorage): void {
  profiles = { ...profiles, [stableId]: { ...profiles[stableId], ...sharedProfile(stableId, mark) } };
  // Canonical profiles are already keyed by stable ID. Remove the imported
  // copy so resetting in classic cannot resurrect an older new-design mark.
  const { [stableId]: _old, ...rest } = marks;
  marks = rest;
  saveContextProfiles(profiles, storage);
  emit();
  save(storage);
}

/** Forget a cluster's appearance in both designs. */
export function resetMark(stableId: string, storage: Storage = settingsStorage): void {
  const name = contextNames.get(stableId);
  const next = { ...profiles };
  delete next[stableId];
  if (name) delete next[name];
  profiles = next;
  const { [stableId]: _old, ...rest } = marks;
  marks = rest;
  saveContextProfiles(profiles, storage);
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

/** Keep raw input in editors so clearing a name or typing spaces is not undone. */
export function useEditableMark(stableId: string, name: string): MarkAppearance {
  return useSyncExternalStore(
    subscribe,
    () => readMark(stableId, name, true),
    () => readMark(stableId, name, true),
  );
}

/** Migrate old name-keyed profiles and import customisations from the new UI. */
export function rememberContextMarks(contexts: readonly ClusterContext[], storage: Storage = settingsStorage): void {
  const migration = migrateRecordKeys(profiles, contexts);
  profiles = migration.migrated;
  let changed = migration.changed;
  for (const context of contexts) {
    contextNames.set(context.stableId, context.name);
    const old = marks[context.stableId];
    if (!old) continue;
    if (!profiles[context.stableId]) profiles = { ...profiles, [context.stableId]: sharedProfile(context.stableId, old) };
    const { [context.stableId]: _old, ...rest } = marks;
    marks = rest;
    changed = true;
  }
  if (changed) { saveContextProfiles(profiles, storage); save(storage); emit(); }
}
