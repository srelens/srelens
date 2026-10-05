import { useSyncExternalStore } from "react";

/**
 * A signal that what `listAgents` answers has changed, for the surfaces that
 * read it once and then stay mounted (#396).
 *
 * The dock's agent picker reads the inventory at mount, and the dock is never
 * unmounted by a tab switch — so a provider key configured in Settings ›
 * Agent & MCP never reached a dock that was already open: the native agent
 * stayed missing from the picker, and one whose key was cleared stayed
 * offered. The writers are known and few, so they say so here rather than the
 * readers polling a question that has an exact answer.
 *
 * Only what srelens itself writes can fire it. A CLI installed or removed
 * outside the app changes the inventory too, and nothing here can know.
 */
let version = 0;
const listeners = new Set<() => void>();

/** Called by a writer once its change has LANDED — never for a failed write,
 *  which changed nothing worth re-reading. */
export function invalidateAgentInventory(): void {
  version += 1;
  for (const l of listeners) l();
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

/** A number that changes whenever the inventory may have — for a reader to
 *  hang its read off, as an effect dependency. */
export function useAgentInventoryVersion(): number {
  return useSyncExternalStore(subscribe, () => version);
}
