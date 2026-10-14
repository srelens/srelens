import { useSyncExternalStore } from "react";
import {
  COLLECTIONS_DOCUMENT_VERSION,
  parseCollectionsDocument,
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
  if (doc === null) {
    try {
      doc = parseCollectionsDocument(settingsStorage.getItem(COLLECTIONS_KEY));
    } catch {
      doc = parseCollectionsDocument(null);
    }
  }
  return doc;
}

function keep(next: CollectionsDocument): void {
  doc = next;
  try {
    settingsStorage.setItem(COLLECTIONS_KEY, JSON.stringify({ ...next, version: COLLECTIONS_DOCUMENT_VERSION }));
  } catch (error) {
    // Storage is unavailable: the change holds for this session.
    console.error("could not save the collections", error);
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Hear every save, edit and delete. */
export const subscribeCollections = subscribe;

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

/**
 * Save a collection, replacing the one called `replacing` (its name before an
 * edit) or adding it. Fields the editor does not show are kept from the
 * collection it replaces.
 */
export function saveCollection(collection: Collection, replacing?: string): void {
  const current = read().collections;
  const at = replacing === undefined ? -1 : current.findIndex((c) => c.type === collection.type && c.name === replacing);
  const collections =
    at < 0
      ? [...current, collection]
      : current.map((c, i) => (i === at ? { ...c, ...collection } : c));
  keep({ ...read(), collections });
}

/** Delete the named collections of one type. */
export function deleteCollections(type: CollectionType, names: readonly string[]): void {
  const gone = new Set(names);
  const current = read().collections;
  const collections = current.filter((c) => !(c.type === type && gone.has(c.name)));
  if (collections.length !== current.length) keep({ ...read(), collections });
}

/** Record that a picker applied these collections, for most-recently-used order. */
export function markCollectionsUsed(type: CollectionType, names: readonly string[], at = Date.now()): void {
  if (names.length === 0) return;
  const used = new Set(names);
  const current = read().collections;
  if (!current.some((c) => c.type === type && used.has(c.name))) return;
  keep({
    ...read(),
    collections: current.map((c) => (c.type === type && used.has(c.name) ? { ...c, lastUsedAt: at } : c)),
  });
}

/** Forget the in-memory copy so a test starts from storage. */
export function __resetCollectionsForTests(): void {
  doc = null;
  listeners.clear();
}
