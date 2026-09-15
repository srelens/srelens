import { useSyncExternalStore } from "react";
import { listContexts, loadKubeconfigFiles } from "@srelens/core";

/**
 * The stable ID of each kubeconfig context, by display name, for per-cluster app scope.
 *
 * A context's name is presentation only: it gains a `file/` prefix as soon as another
 * kubeconfig declares the same name, so anything kept per context keys on the stable ID
 * (#265). Both desktop designs render app surfaces with only a context name, so they look
 * the ID up here rather than each threading it through. One listing per window, refreshed
 * on focus. The host enforces scope on every read and action; this only decides what to show.
 */
let ids: ReadonlyMap<string, string> | undefined;
const listeners = new Set<() => void>();
let stop: (() => void) | undefined;

async function refresh() {
  try {
    const outcome = await listContexts(loadKubeconfigFiles());
    const next = new Map((outcome?.contexts ?? []).map((context) => [context.name, context.stableId]));
    if (!ids || JSON.stringify([...ids]) !== JSON.stringify([...next])) {
      ids = next;
      for (const listener of listeners) listener();
    }
  } catch {
    // An unlisted context has no ID, so an app limited to chosen clusters stays hidden.
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    void refresh();
    stop = () => window.removeEventListener("focus", onFocus);
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      stop?.();
      stop = undefined;
      ids = undefined;
    }
  };
}

const getIds = () => ids;

/** The stable ID of the context named `name`, or undefined until the contexts are listed. */
export function useContextId(name: string): string | undefined {
  return useSyncExternalStore(subscribe, getIds, getIds)?.get(name);
}
