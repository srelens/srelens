import { useEffect, useMemo, useSyncExternalStore } from "react";
import { loadContextOrder, orderContexts, saveContextOrder, migrateOrder, projectOrderToNames, mergeOrderFromNames, settingsStorage, unprefixedName, type ContextIdentity } from "@srelens/core";
import { getContextsStatus, useContextsStatus } from "./clusters";
const AMBIGUOUS_ORDER_KEY = "srelens.next.ambiguousContextOrder";

function orderMigration(order: string[], contexts: readonly ContextIdentity[], complete: boolean) {
  let saved: string[] = [];
  try {
    const raw: unknown = JSON.parse(settingsStorage.getItem(AMBIGUOUS_ORDER_KEY) ?? "[]");
    if (Array.isArray(raw)) saved = raw.filter((key): key is string => typeof key === "string");
  } catch { /* A missing or unreadable record does not prevent ordering. */ }
  const ambiguous = new Set(saved);
  const ids = new Set(contexts.map(context => context.stableId));
  const migrated = order.map(key => {
    if (ids.has(key)) return key;
    const exact = contexts.filter(context => context.name === key);
    const candidates = exact.length ? exact : contexts.filter(context => unprefixedName(context.name) === key);
    if (candidates.length > 1) ambiguous.add(key);
    return !complete || ambiguous.has(key) ? key : migrateOrder([key], contexts).migrated[0];
  });
  return { migrated, changed: migrated.some((key, index) => key !== order[index]),
    ambiguous: [...ambiguous], ambiguityChanged: ambiguous.size !== saved.length };
}
function persistMigration(migration: ReturnType<typeof orderMigration>) {
  if (migration.ambiguityChanged) {
    try { settingsStorage.setItem(AMBIGUOUS_ORDER_KEY, JSON.stringify(migration.ambiguous)); }
    catch (error) { console.error("could not persist ambiguous context order keys", error); }
  }
  if (migration.changed) saveContextOrder(migration.migrated);
}
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const read = () => JSON.stringify(loadContextOrder());
export function useOrderedContexts<T extends ContextIdentity>(
  contexts: readonly T[],
  migrationContexts: readonly ContextIdentity[] = contexts,
): T[] {
  const order = useSyncExternalStore(subscribe, read, read);
  const status = useContextsStatus();
  const migration = useMemo(() => orderMigration(JSON.parse(order), migrationContexts, status !== "failed"), [migrationContexts, order, status]);
  useEffect(() => {
    persistMigration(migration);
  }, [migration]);
  return useMemo(() => orderContexts([...contexts], projectOrderToNames(migration.migrated, contexts)), [contexts, migration]);
}
/**
 * Fold a reordered SUBSET back into the shared order, slot for slot.
 *
 * `mergeOrderFromNames` is for a caller holding every connected context: it
 * writes the reordered list first and re-appends whatever it was not told
 * about, on the understanding that those are offline. A workspace's rail holds
 * only its own clusters, and to it every other context — connected, in another
 * workspace — looked offline too: reordering two of a workspace's clusters
 * sent each context between them to the end of the shared order, rearranging
 * a list the reader was not looking at.
 *
 * Here the places the subset occupied are the only places written. Walking
 * the old order, each slot that held one of the subset's contexts takes the
 * next of them in the new order; every other slot is left as it was. A subset
 * context the old order never held goes on the end, where an unsaved context
 * already sorts.
 */
function mergeSubsetOrder(previous: readonly string[], names: readonly string[], contexts: readonly ContextIdentity[]): string[] {
  const idOf = new Map(contexts.map((context) => [context.name, context.stableId]));
  const reordered = names.map((name) => idOf.get(name)).filter((id): id is string => !!id);
  const subset = new Set(contexts.map((context) => context.stableId));
  let next = 0;
  const merged = previous.map((id) => (subset.has(id) ? reordered[next++] : id)).filter((id): id is string => !!id);
  return [...merged, ...reordered.slice(next)];
}

/**
 * Move before the target; unlisted contexts keep their place in the shared preference.
 *
 * `subset` says `contexts` is PART of what is connected — a workspace's own
 * clusters — so that the rest are left exactly where they are rather than
 * treated as offline. See {@link mergeSubsetOrder}.
 */
export function moveContext(contexts: readonly ContextIdentity[], name: string, before: string | null, subset = false): void {
  if (name === before || !contexts.some(c => c.name === name) || (before !== null && !contexts.some(c => c.name === before))) return;
  const migration = orderMigration(loadContextOrder(), contexts, getContextsStatus() !== "failed");
  persistMigration(migration);
  const previous = migration.migrated;
  const names = orderContexts([...contexts], projectOrderToNames(previous, contexts)).map(c => c.name).filter(n => n !== name);
  names.splice(before === null ? names.length : names.indexOf(before), 0, name);
  saveContextOrder(subset ? mergeSubsetOrder(previous, names, contexts) : mergeOrderFromNames(previous, names, contexts));
  listeners.forEach(listener => listener());
}
/** Keyboard moves can place a context after the last row as well as before one. */
export function moveContextBy(contexts: readonly ContextIdentity[], name: string, delta: -1 | 1, subset = false): void {
  const migration = orderMigration(loadContextOrder(), contexts, getContextsStatus() !== "failed");
  persistMigration(migration);
  const order = migration.migrated;
  const ordered = orderContexts([...contexts], projectOrderToNames(order, contexts));
  const index = ordered.findIndex(c => c.name === name);
  if (index < 0) return;
  const target = ordered[index + delta];
  if (!target) return;
  if (delta === -1) moveContext(ordered, name, target.name, subset);
  else moveContext(ordered, target.name, name, subset);
}
/** A confirmed deletion is different from an offline context: forget its saved slot. */
export function removeContextFromOrder(stableId: string): void {
  const previous = loadContextOrder();
  const next = previous.filter(id => id !== stableId);
  if (next.length === previous.length) return;
  saveContextOrder(next);
  listeners.forEach(listener => listener());
}
