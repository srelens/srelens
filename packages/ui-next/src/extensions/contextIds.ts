import { useSyncExternalStore } from "react";
import { KUBECONFIG_FILES_CHANGED, listContexts, getLiveKubeconfigFiles } from "@srelens/core";

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
  /**
   * Names whose stable ID another listed context also carries (`a` + `b#c` and `a#b` + `c`
   * read the same). The host refuses such an ID for both, so neither is "found".
   */
  shared?: ReadonlySet<string>;
  /** Why the latest listing failed, when it did. */
  error?: string;
};
let state: ContextIds = {};
const listeners = new Set<() => void>();
let stop: (() => void) | undefined;
/** The latest listing started. Refreshes can overlap (a focus during a Retry), and an older one answering last must not replace a newer answer. */
let generation = 0;

const key = ({ ids, shared, error }: ContextIds) =>
  JSON.stringify([ids ? [...ids] : null, shared ? [...shared] : null, error ?? null]);

/** List the contexts again. */
export async function refreshContextIds() {
  const started = ++generation;
  let outcome: Awaited<ReturnType<typeof listContexts>> | undefined;
  try {
    outcome = await listContexts(getLiveKubeconfigFiles());
  } catch (e) {
    outcome = { error: String(e) };
  }
  if (!listeners.size || started !== generation) return;
  const listed = outcome?.contexts ?? (outcome?.error ? undefined : []);
  let next: ContextIds;
  if (listed) {
    const holders = new Map<string, number>();
    for (const context of listed) holders.set(context.stableId, (holders.get(context.stableId) ?? 0) + 1);
    const unique = listed.filter((context) => holders.get(context.stableId) === 1);
    next = {
      ids: new Map(unique.map((context) => [context.name, context.stableId])),
      shared: new Set(listed.filter((context) => holders.get(context.stableId)! > 1).map((context) => context.name)),
      error: outcome?.error || undefined,
    };
  } else {
    next = { ids: state.ids, shared: state.shared, error: outcome?.error || undefined };
  }
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
    const onFilesChanged = refresh;
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
      ++generation;
    }
  };
}

const getState = () => state;

/** The stable ID of the context named `name`, or undefined until the contexts are listed. */
export function useContextId(name: string): string | undefined {
  return useSyncExternalStore(subscribe, getState, getState).ids?.get(name);
}

/** What a page can say when a limited app's cluster cannot be checked, by lookup status. */
export const SHARED_CONTEXT_ID_MESSAGE =
  "This cluster shares its ID with another context, so apps limited to it cannot be checked. Rename one of them in your kubeconfig files.";

/**
 * The context named `name`: found; still being listed; not found because the listing
 * failed; sharing its stable ID with another context; or missing from a listing that worked.
 */
export type ContextLookup =
  | { status: "found"; id: string }
  | { status: "loading" }
  | { status: "failed"; error: string }
  | { status: "shared" }
  | { status: "missing" };

export function useContextLookup(name: string): ContextLookup {
  const { ids, shared, error } = useSyncExternalStore(subscribe, getState, getState);
  const id = ids?.get(name);
  if (id !== undefined) return { status: "found", id };
  if (shared?.has(name)) return { status: "shared" };
  if (error) return { status: "failed", error };
  return ids ? { status: "missing" } : { status: "loading" };
}
