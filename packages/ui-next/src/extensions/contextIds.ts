import { useSyncExternalStore } from "react";
import { KUBECONFIG_FILES_CHANGED, listContexts, loadKubeconfigFiles } from "@srelens/core";

/**
 * The stable ID of each kubeconfig context, by display name, for per-cluster app scope.
 *
 * A context's name is presentation only: it gains a `file/` prefix as soon as another
 * kubeconfig declares the same name, so anything kept per context keys on the stable ID
 * (#265). Both desktop designs render app surfaces with only a context name, so they look
 * the ID up here rather than each threading it through. One listing per window, refreshed
 * on focus. The host enforces scope on every read and action; this only decides what to show.
 *
 * A listing that fails keeps the IDs already known and records why, so a caller can tell
 * "not enabled for this cluster" from "the clusters could not be listed".
 */
type ContextIds = {
  /** Undefined until a listing has answered. */
  ids?: ReadonlyMap<string, string>;
  /** Why the latest listing failed, when it did. */
  error?: string;
};
let state: ContextIds = {};
const listeners = new Set<() => void>();
let stop: (() => void) | undefined;
/** The latest listing started. Refreshes can overlap (a focus during a Retry), and an older one answering last must not replace a newer answer. */
let generation = 0;
/** The kubeconfig files in use, once a change has been published; storage may not hold them (a refused save). */
let files: string[] | undefined;

const key = ({ ids, error }: ContextIds) => JSON.stringify([ids ? [...ids] : null, error ?? null]);

/** List the contexts again. */
export async function refreshContextIds() {
  const started = ++generation;
  let outcome: Awaited<ReturnType<typeof listContexts>> | undefined;
  try {
    outcome = await listContexts(files ?? loadKubeconfigFiles());
  } catch (e) {
    outcome = { error: String(e) };
  }
  if (!listeners.size || started !== generation) return;
  const listed = outcome?.contexts ?? (outcome?.error ? undefined : []);
  const next: ContextIds = {
    ids: listed ? new Map(listed.map((context) => [context.name, context.stableId])) : state.ids,
    error: outcome?.error || undefined,
  };
  if (key(next) !== key(state)) {
    state = next;
    for (const listener of listeners) listener();
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    // Kubeconfig files can change while the window keeps focus (Settings → Contexts), and
    // that can add a context or rename one, so list again then as well as on focus.
    const refresh = () => void refreshContextIds();
    const onFilesChanged = (event: Event) => {
      const detail = (event as CustomEvent<string[]>).detail;
      if (Array.isArray(detail)) files = detail;
      refresh();
    };
    window.addEventListener("focus", refresh);
    window.addEventListener(KUBECONFIG_FILES_CHANGED, onFilesChanged);
    void refreshContextIds();
    stop = () => {
      window.removeEventListener("focus", refresh);
      window.removeEventListener(KUBECONFIG_FILES_CHANGED, onFilesChanged);
    };
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      stop?.();
      stop = undefined;
      state = {};
      files = undefined;
    }
  };
}

const getState = () => state;

/** The stable ID of the context named `name`, or undefined until the contexts are listed. */
export function useContextId(name: string): string | undefined {
  return useSyncExternalStore(subscribe, getState, getState).ids?.get(name);
}

/** The context named `name`: found, still being listed, or not found because the listing failed or lacks it. */
export type ContextLookup =
  | { status: "found"; id: string }
  | { status: "loading" }
  | { status: "failed"; error: string }
  | { status: "missing" };

export function useContextLookup(name: string): ContextLookup {
  const { ids, error } = useSyncExternalStore(subscribe, getState, getState);
  const id = ids?.get(name);
  if (id !== undefined) return { status: "found", id };
  if (error) return { status: "failed", error };
  return ids ? { status: "missing" } : { status: "loading" };
}
