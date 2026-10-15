// Collections (#881, #885): saved groupings that act as filters. The model,
// validation and matching live in the backend (`srelens_kube::collections`),
// so every surface resolves a collection the same way; this is the typed
// boundary to it, and the shape the frontend keeps collections in.
import { invokeCapability, type Invoker } from "../transport/transport";

export type CollectionType = "namespace" | "resource";

/** One saved collection. Fields this build does not know are kept as they are. */
export interface Collection {
  name: string;
  description?: string;
  type: CollectionType;
  /** Resource collections: `group/Kind`, or the bare `Kind` of the core group. */
  kinds?: string[];
  /** Explicit names, always included. */
  members?: string[];
  /** Name globs, `*` for any run of characters, case-sensitive. */
  patterns?: string[];
  /** A label selector, as `kubectl get -l` takes it. */
  labelSelector?: string;
  /** When a picker last applied it, for most-recently-used order. */
  lastUsedAt?: number;
  [unknown: string]: unknown;
}

/** The stored document: `{ version: 1, collections: [...] }`. */
export interface CollectionsDocument {
  version: number;
  collections: Collection[];
  [unknown: string]: unknown;
}

export const COLLECTIONS_DOCUMENT_VERSION = 1;

/** Something wrong with a collection, against its field (and entry, for a list field). */
export interface CollectionProblem {
  field: "name" | "description" | "kinds" | "members" | "patterns" | "labelSelector" | string;
  index?: number;
  message: string;
}

export type MemberStatus = "found" | "missing" | "unknown";

export interface CollectionMatch {
  name: string;
  matched: string[];
  members: Array<{ name: string; status: MemberStatus }>;
  /** Set when the collection could not be resolved; it then matched nothing. */
  problem?: string;
}

export interface NamespaceResolution {
  /** The union of every collection's namespaces. */
  namespaces: string[];
  collections: CollectionMatch[];
  /** True when the listing was partial (restricted credentials). */
  partial: boolean;
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const strings = (v: unknown): string[] | undefined =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : undefined;

/**
 * A stored document, read leniently: a collection without a name or a known
 * type is dropped on its own, and every field this build does not know is
 * kept, so saving it back does not lose what a later build wrote.
 */
export function parseCollectionsDocument(raw: unknown): CollectionsDocument {
  const doc = typeof raw === "string" ? safeParse(raw) : raw;
  if (!isRecord(doc) || !Array.isArray(doc.collections)) {
    return { version: COLLECTIONS_DOCUMENT_VERSION, collections: [] };
  }
  const collections: Collection[] = [];
  for (const entry of doc.collections) {
    if (!isRecord(entry) || typeof entry.name !== "string" || entry.name.trim() === "") continue;
    if (entry.type !== "namespace" && entry.type !== "resource") continue;
    const collection: Collection = { ...entry, name: entry.name, type: entry.type };
    for (const field of ["kinds", "members", "patterns"] as const) {
      const list = strings(entry[field]);
      if (list === undefined) delete collection[field];
      else collection[field] = list;
    }
    if (typeof entry.description !== "string") delete collection.description;
    if (typeof entry.labelSelector !== "string") delete collection.labelSelector;
    if (typeof entry.lastUsedAt !== "number" || !Number.isFinite(entry.lastUsedAt)) delete collection.lastUsedAt;
    collections.push(collection);
  }
  return { ...doc, version: typeof doc.version === "number" ? doc.version : COLLECTIONS_DOCUMENT_VERSION, collections };
}

function safeParse(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

/**
 * Names pasted from somewhere else — a release ticket, a spreadsheet column —
 * split on commas, newlines and other whitespace, trimmed, and each kept once
 * in the order given.
 */
export function parseMemberList(text: string): string[] {
  const seen = new Set<string>();
  for (const part of text.split(/[\s,;]+/)) {
    const name = part.trim();
    if (name) seen.add(name);
  }
  return [...seen];
}

/** What is wrong with `collection`, empty when it can be saved. */
export async function validateCollection(
  collection: Collection,
  taken: string[],
  invoke: Invoker = invokeCapability,
): Promise<CollectionProblem[]> {
  const out = await invoke<{ problems: CollectionProblem[] }>("collections.validate", {
    collection: forBackend(collection),
    taken,
  });
  return out.problems;
}

/** One namespace a view can see, with the labels a selector reads. */
export interface NamespaceItem {
  name: string;
  labels?: Record<string, string>;
}

/**
 * The namespaces `collections` cover out of `namespaces`. `complete` is false
 * when the listing is not every namespace on the cluster.
 */
export async function resolveNamespaceCollections(
  collections: Collection[],
  namespaces: NamespaceItem[],
  complete: boolean,
  invoke: Invoker = invokeCapability,
): Promise<NamespaceResolution> {
  return invoke<NamespaceResolution>("collections.resolveNamespaces", {
    collections: collections.map(forBackend),
    namespaces: namespaces.map((ns) => ({ name: ns.name, labels: ns.labels ?? {} })),
    complete,
  });
}

/** The fields the backend reads; the rest stay in the frontend's document. */
function forBackend(c: Collection) {
  return {
    name: c.name,
    description: c.description ?? "",
    type: c.type,
    kinds: c.kinds ?? [],
    members: c.members ?? [],
    patterns: c.patterns ?? [],
    ...(c.labelSelector && c.labelSelector.trim() !== "" ? { labelSelector: c.labelSelector.trim() } : {}),
  };
}
