import { useSyncExternalStore } from "react";
import {
  resolveNamespaceCollections,
  SELECTS_NOTHING,
  type Collection,
  type NamespaceItem,
  type NamespaceResolution,
} from "@srelens/core";
import { getCollections, subscribeCollections } from "./collections";

/**
 * What a tab's namespace collections come to on a cluster (#881).
 *
 * A collection is a rule, not a list, so it is resolved against the
 * namespaces a view can see: the listing the namespace picker loads, labels
 * included. Resolution is the backend's (`collections.resolveNamespaces`),
 * and so asynchronous; this keeps the answer, keyed by the cluster, the
 * collections' definitions and the listing it was worked out from. A change
 * to any of the three — a collection edited in Settings, a namespace created
 * — is a new key, and the views re-resolve on their own.
 *
 * Until there is an answer the selection is {@link SELECTS_NOTHING}, never
 * the empty selection, which means every namespace: a list scoped to "prod"
 * must not show the whole cluster while it waits.
 */

interface Listing {
  items: NamespaceItem[];
  complete: boolean;
  version: number;
}

type Entry =
  | { state: "pending" }
  | { state: "ready"; resolution: NamespaceResolution; selection: readonly string[] }
  | { state: "failed"; error: string };

const listings = new Map<string, Listing>();
const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();
let listingSeq = 0;

function emit(): void {
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function sameItems(a: NamespaceItem[], b: NamespaceItem[]): boolean {
  return (
    a.length === b.length &&
    a.every((item, i) => item.name === b[i].name && JSON.stringify(item.labels ?? {}) === JSON.stringify(b[i].labels ?? {}))
  );
}

/**
 * The namespaces a view loaded for a cluster, with whether that is all of
 * them. The namespace picker reports what it shows; an unchanged listing
 * changes nothing.
 */
export function noteNamespaceListing(clusterId: string, items: NamespaceItem[], complete: boolean): void {
  const known = listings.get(clusterId);
  if (known && known.complete === complete && sameItems(known.items, items)) return;
  listings.set(clusterId, { items, complete, version: ++listingSeq });
  // Answers worked out from the listing this one replaces are of no more use.
  for (const key of [...entries.keys()]) if (key.startsWith(`${clusterId}\u0000`)) entries.delete(key);
  emit();
}

/** The namespaces last loaded for a cluster, if a view has loaded them. */
export function listingFor(clusterId: string): { items: NamespaceItem[]; complete: boolean } | undefined {
  return listings.get(clusterId);
}

/** {@link listingFor}, re-rendering when a view loads a newer listing. */
export function useListing(clusterId: string | undefined): { items: NamespaceItem[]; complete: boolean } | undefined {
  const read = () => (clusterId === undefined ? undefined : listings.get(clusterId));
  return useSyncExternalStore(subscribe, read, read);
}

export type ScopeStatus =
  /** No listing for the cluster yet: nothing to resolve against. */
  | "waiting"
  /** Being resolved. */
  | "resolving"
  | "ready"
  | "failed"
  /** None of the named collections exists any more. */
  | "gone";

export interface CollectionScope {
  status: ScopeStatus;
  /** The selection to list by: the namespaces, or {@link SELECTS_NOTHING}. */
  selection: readonly string[];
  /** The named collections that still exist. */
  collections: Collection[];
  resolution?: NamespaceResolution;
  error?: string;
}

const GONE: CollectionScope = { status: "gone", selection: SELECTS_NOTHING, collections: [] };
const scopes = new Map<string, CollectionScope>();

/** Stable per answer, as `useSyncExternalStore` needs. */
function scopeOf(key: string, make: () => CollectionScope): CollectionScope {
  const made = make();
  const held = scopes.get(key);
  if (
    held &&
    held.status === made.status &&
    held.selection === made.selection &&
    held.resolution === made.resolution &&
    held.error === made.error &&
    held.collections.length === made.collections.length &&
    held.collections.every((c, i) => c === made.collections[i])
  ) {
    return held;
  }
  scopes.set(key, made);
  return made;
}

/** What the collections called `names` come to on `clusterId`, starting the work if it is not under way. */
export function collectionScope(clusterId: string, names: readonly string[]): CollectionScope {
  const wanted = new Set(names);
  const collections = getCollections().filter((c) => c.type === "namespace" && wanted.has(c.name));
  if (collections.length === 0) return GONE;
  const viewKey = `${clusterId}\u0000${names.join("\u0000")}`;
  const listing = listings.get(clusterId);
  if (!listing) {
    return scopeOf(viewKey, () => ({ status: "waiting", selection: SELECTS_NOTHING, collections }));
  }
  const key = `${clusterId}\u0000${listing.version}\u0000${JSON.stringify(collections.map(definition))}`;
  let entry = entries.get(key);
  if (!entry) {
    entry = { state: "pending" };
    entries.set(key, entry);
    resolveNamespaceCollections(collections, listing.items, listing.complete).then(
      (resolution) => {
        if (entries.get(key)?.state !== "pending") return;
        const selection = resolution.namespaces.length > 0 ? resolution.namespaces : SELECTS_NOTHING;
        entries.set(key, { state: "ready", resolution, selection });
        emit();
      },
      (error: unknown) => {
        if (entries.get(key)?.state !== "pending") return;
        entries.set(key, { state: "failed", error: error instanceof Error ? error.message : String(error) });
        emit();
      },
    );
  }
  const settled = entry;
  return scopeOf(viewKey, () => {
    if (settled.state === "ready") {
      return { status: "ready", selection: settled.selection, collections, resolution: settled.resolution };
    }
    if (settled.state === "failed") {
      return { status: "failed", selection: SELECTS_NOTHING, collections, error: settled.error };
    }
    return { status: "resolving", selection: SELECTS_NOTHING, collections };
  });
}

/** What decides a collection's answer; not `lastUsedAt`, which changes on every use. */
function definition(c: Collection) {
  return [c.name, c.members ?? [], c.patterns ?? [], c.labelSelector ?? ""];
}

/** Re-render on a listing, an answer, or a change to the saved collections. */
export function subscribeScopes(listener: () => void): () => void {
  const off = subscribe(listener);
  const offCollections = subscribeCollections(listener);
  return () => {
    off();
    offCollections();
  };
}

/** {@link collectionScope} as a hook, for the picker and the notices beside a list. */
export function useCollectionScope(clusterId: string | undefined, names: readonly string[] | undefined): CollectionScope | null {
  const read = () => (clusterId === undefined || !names || names.length === 0 ? null : collectionScope(clusterId, names));
  return useSyncExternalStore(subscribeScopes, read, read);
}

/** Forget every listing and answer, so a test starts clean. */
export function __resetCollectionScopeForTests(): void {
  listings.clear();
  entries.clear();
  scopes.clear();
  listeners.clear();
}
