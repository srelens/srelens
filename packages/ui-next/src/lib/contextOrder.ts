import { useMemo, useSyncExternalStore } from "react";
import { loadContextOrder, orderContexts, saveContextOrder, migrateOrder, projectOrderToNames, mergeOrderFromNames, type ContextIdentity } from "@srelens/core";
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const read = () => JSON.stringify(loadContextOrder());
export function useOrderedContexts<T extends ContextIdentity>(contexts: readonly T[]): T[] {
  const order = useSyncExternalStore(subscribe, read, read);
  return useMemo(() => orderContexts([...contexts], projectOrderToNames(migrateOrder(JSON.parse(order), contexts).migrated, contexts)), [contexts, order]);
}
/** Move before the target; unlisted contexts keep their place in the shared preference. */
export function moveContext(contexts: readonly ContextIdentity[], name: string, before: string | null): void {
  if (name === before || !contexts.some(c => c.name === name) || (before !== null && !contexts.some(c => c.name === before))) return;
  const previous = migrateOrder(loadContextOrder(), contexts).migrated;
  const names = orderContexts([...contexts], projectOrderToNames(previous, contexts)).map(c => c.name).filter(n => n !== name);
  names.splice(before === null ? names.length : names.indexOf(before), 0, name);
  saveContextOrder(mergeOrderFromNames(previous, names, contexts));
  listeners.forEach(listener => listener());
}
/** Keyboard moves can place a context after the last row as well as before one. */
export function moveContextBy(contexts: readonly ContextIdentity[], name: string, delta: -1 | 1): void {
  const order = migrateOrder(loadContextOrder(), contexts).migrated;
  const ordered = orderContexts([...contexts], projectOrderToNames(order, contexts));
  const index = ordered.findIndex(c => c.name === name);
  if (index < 0) return;
  const target = ordered[index + delta];
  if (!target) return;
  if (delta === -1) moveContext(ordered, name, target.name);
  else moveContext(ordered, target.name, name);
}
