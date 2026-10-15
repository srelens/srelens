import { useSyncExternalStore } from "react";
import {
  COLLECTIONS_DOCUMENT_VERSION,
  parseCollectionsDocument,
  refreshSetting,
  settingsStorage,
  type Collection,
  type CollectionsDocument,
  type CollectionType,
} from "@srelens/core";

/**
 * The saved collections (#881, #885): one document for the whole app, in the
 * settings store — the desktop's settings file, a web user's settings rows —
 * so they are global and survive a restart, and every window reads the same
 * ones.
 *
 * The document is `{ version: 1, collections: [...] }` and is read leniently
 * (`parseCollectionsDocument`): what this build does not know is kept, so a
 * later build's fields survive a save from this one.
 */
export const COLLECTIONS_KEY = "srelens.collections";

let doc: CollectionsDocument | null = null;
const listeners = new Set<() => void>();

function read(): CollectionsDocument {
  if (doc === null) doc = stored();
  return doc;
}

function stored(): CollectionsDocument {
  try {
    return parseCollectionsDocument(settingsStorage.getItem(COLLECTIONS_KEY));
  } catch {
    return parseCollectionsDocument(null);
  }
}

function notify(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Hear every save, edit and delete. */
export const subscribeCollections = subscribe;

type Change = (collections: Collection[]) => Collection[];

/** Changes made and not yet written, oldest first. */
let waiting: Change[] = [];
let saving: Promise<void> = Promise.resolve();

/**
 * Make a change: at once in this window, and then to the document as the
 * backend has it now.
 *
 * Every window keeps its own copy of the document. Writing that copy whole
 * would put back whatever it had when another window added a collection
 * since — the other window's collection, gone. So each change is a function
 * of the collections, applied to this window's copy for the screen and, once
 * the setting has been read again, to the latest one for the write. Changes
 * still waiting stay applied on top of whatever comes back, so the screen
 * never shows a change undone and then redone.
 */
function change(apply: Change): void {
  waiting.push(apply);
  doc = { ...read(), collections: apply(read().collections) };
  notify();
  saving = saving.then(async () => {
    try {
      await refreshSetting(COLLECTIONS_KEY);
    } catch {
      // Not readable again: this window's copy of the setting is the best there is.
    }
    const latest = stored();
    const next = { ...latest, collections: apply(latest.collections) };
    try {
      settingsStorage.setItem(COLLECTIONS_KEY, JSON.stringify({ ...next, version: COLLECTIONS_DOCUMENT_VERSION }));
    } catch (error) {
      // Storage is unavailable: the change holds for this session.
      console.error("could not save the collections", error);
    }
    waiting = waiting.filter((w) => w !== apply);
    doc = { ...next, collections: waiting.reduce((list, w) => w(list), next.collections) };
    notify();
  });
}

/**
 * Read the setting again, so a page showing the collections shows what
 * another window saved since this one started.
 */
export function refreshCollections(): Promise<void> {
  saving = saving.then(async () => {
    try {
      await refreshSetting(COLLECTIONS_KEY);
    } catch {
      return;
    }
    const latest = stored();
    doc = { ...latest, collections: waiting.reduce((list, w) => w(list), latest.collections) };
    notify();
  });
  return saving;
}

/** Resolves when every change made so far has been written. */
export function collectionsSaved(): Promise<void> {
  return saving;
}

export function getCollections(): readonly Collection[] {
  return read().collections;
}

/** Every saved collection, re-rendering when one is saved, changed or deleted. */
export function useCollections(): readonly Collection[] {
  return useSyncExternalStore(subscribe, getCollections, getCollections);
}

/** The collections of one type, most recently used first, then by name. */
export function byRecentUse(collections: readonly Collection[], type: CollectionType): Collection[] {
  return collections
    .filter((c) => c.type === type)
    .sort((a, b) => (b.lastUsedAt ?? 0) - (a.lastUsedAt ?? 0) || a.name.localeCompare(b.name));
}

/** The fields the editor owns: a save replaces these whole, set or not. */
const EDITED = ["name", "description", "kinds", "members", "patterns", "labelSelector"] as const;

/**
 * Save a collection, replacing the one called `replacing` (its name before an
 * edit) or adding it.
 *
 * The fields the editor shows are replaced whole, so one cleared in the editor
 * is cleared here too — not left at its old value because the draft no longer
 * carries it. What the editor does not show (when it was last used, fields a
 * later build wrote) is kept from the collection it replaces.
 */
export function saveCollection(collection: Collection, replacing?: string): void {
  change((collections) => {
    const at = replacing === undefined ? -1 : collections.findIndex((c) => c.type === collection.type && c.name === replacing);
    if (at < 0) return [...collections, collection];
    const kept: Collection = { ...collections[at] };
    for (const field of EDITED) delete kept[field];
    return collections.map((c, i) => (i === at ? { ...kept, ...collection } : c));
  });
  if (replacing !== undefined && replacing !== collection.name) onRename(collection.type, replacing, collection.name);
}

/** Delete the named collections of one type. */
export function deleteCollections(type: CollectionType, names: readonly string[]): void {
  const gone = new Set(names);
  if (!read().collections.some((c) => c.type === type && gone.has(c.name))) return;
  change((collections) => collections.filter((c) => !(c.type === type && gone.has(c.name))));
}

/** Record that a picker applied these collections, for most-recently-used order. */
export function markCollectionsUsed(type: CollectionType, names: readonly string[], at = Date.now()): void {
  if (names.length === 0) return;
  const used = new Set(names);
  if (!read().collections.some((c) => c.type === type && used.has(c.name))) return;
  change((collections) => collections.map((c) => (c.type === type && used.has(c.name) ? { ...c, lastUsedAt: at } : c)));
}

type RenameListener = (type: CollectionType, from: string, to: string) => void;
const renameListeners = new Set<RenameListener>();

/**
 * Hear a collection being renamed, to move what refers to it by name — the
 * tabs scoped to it — onto the new name. Kept as a listener rather than a call
 * into the tab store, which imports nothing from here.
 */
export function onCollectionRenamed(listener: RenameListener): () => void {
  renameListeners.add(listener);
  return () => renameListeners.delete(listener);
}

function onRename(type: CollectionType, from: string, to: string): void {
  for (const listener of renameListeners) listener(type, from, to);
}

/** Forget the in-memory copy so a test starts from storage. */
export function __resetCollectionsForTests(): void {
  doc = null;
  listeners.clear();
  waiting = [];
  saving = Promise.resolve();
}
