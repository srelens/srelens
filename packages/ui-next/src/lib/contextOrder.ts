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
/** Move before the target; unlisted contexts keep their place in the shared preference. */
export function moveContext(contexts: readonly ContextIdentity[], name: string, before: string | null): void {
  if (name === before || !contexts.some(c => c.name === name) || (before !== null && !contexts.some(c => c.name === before))) return;
  const migration = orderMigration(loadContextOrder(), contexts, getContextsStatus() !== "failed");
  persistMigration(migration);
  const previous = migration.migrated;
  const names = orderContexts([...contexts], projectOrderToNames(previous, contexts)).map(c => c.name).filter(n => n !== name);
  names.splice(before === null ? names.length : names.indexOf(before), 0, name);
  saveContextOrder(mergeOrderFromNames(previous, names, contexts));
  listeners.forEach(listener => listener());
}
/** Keyboard moves can place a context after the last row as well as before one. */
export function moveContextBy(contexts: readonly ContextIdentity[], name: string, delta: -1 | 1): void {
  const migration = orderMigration(loadContextOrder(), contexts, getContextsStatus() !== "failed");
  persistMigration(migration);
  const order = migration.migrated;
  const ordered = orderContexts([...contexts], projectOrderToNames(order, contexts));
  const index = ordered.findIndex(c => c.name === name);
  if (index < 0) return;
  const target = ordered[index + delta];
  if (!target) return;
  if (delta === -1) moveContext(ordered, name, target.name);
  else moveContext(ordered, target.name, name);
}
/** A confirmed deletion is different from an offline context: forget its saved slot. */
export function removeContextFromOrder(stableId: string): void {
  const previous = loadContextOrder();
  const next = previous.filter(id => id !== stableId);
  if (next.length === previous.length) return;
  saveContextOrder(next);
  listeners.forEach(listener => listener());
}
