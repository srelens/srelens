import { useSyncExternalStore } from "react";
import { KUBECONFIG_FILES_CHANGED, listContexts, getLiveKubeconfigFiles } from "@srelens/core";

/**
 * The key of each kubeconfig context, by display name or pinned ID, for per-cluster app scope.
 *
 * A context's name is presentation only: it gains a `file/` prefix as soon as another
 * kubeconfig declares the same name, so anything kept per context keys on the context key
 * (#265). App surfaces are handed only the string they ask the host by: a display name in the
 * Inspector and the classic design, a pinned ID on the new design's app pages (#695). They
 * look the key up here rather than each threading it through. One listing per window,
 * refreshed on focus. The host enforces scope on every read and action; this only decides
 * what to show.
 *
 * A listing that fails keeps the IDs already known and records why, so a caller can tell
 * "not enabled for this cluster" from "the clusters could not be listed".
 */
type Listed = { name: string; key: string; pinnedId?: string };
type ContextIds = {
  /** Undefined until a listing has answered. */
  contexts?: readonly Listed[];
  /** Why the latest listing failed, when it did. */
  error?: string;
};
let state: ContextIds = {};
const listeners = new Set<() => void>();
let stop: (() => void) | undefined;
/** The latest listing started. Refreshes can overlap (a focus during a Retry), and an older one answering last must not replace a newer answer. */
let generation = 0;

const snapshotKey = ({ contexts, error }: ContextIds) => JSON.stringify([contexts ?? null, error ?? null]);

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
  const next: ContextIds = {
    contexts: listed ? listed.map(({ name, key, pinnedId }) => ({ name, key, pinnedId })) : state.contexts,
    error: outcome?.error || undefined,
  };
  if (snapshotKey(next) !== snapshotKey(state)) {
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

/**
 * The context `context` names: by display name, or by the pinned ID an app page asks the host
 * by. Never by stable ID, which two contexts can share (#623). A string that is one context's
 * name and another's pinned ID names neither, as the host's `find_context` finds neither.
 */
function lookUp(contexts: readonly Listed[], context: string): ContextLookup {
  const named = contexts.find((listed) => listed.name === context);
  const pinned = contexts.find((listed) => listed.pinnedId === context);
  if (named && pinned && named.key !== pinned.key) return { status: "ambiguous" };
  const found = named ?? pinned;
  return found ? { status: "found", id: found.key } : { status: "missing" };
}

/** The key of the context `context` names, or undefined until the contexts are listed. */
export function useContextId(context: string): string | undefined {
  const { contexts } = useSyncExternalStore(subscribe, getState, getState);
  const lookup = contexts && lookUp(contexts, context);
  return lookup?.status === "found" ? lookup.id : undefined;
}

/**
 * What an app page says when its route, from a tab opened before #695, names a stable ID two
 * contexts share: the route cannot say which. Opened again, it names the context by key.
 */
export const SHARED_CONTEXT_ID_MESSAGE =
  "This tab names its cluster by an ID that two contexts share, so it cannot tell which one it is for. Open the page again from that cluster's sidebar.";

/** What an app surface says when the host lists its cluster without a pinned ID to ask it by. */
export const NO_PINNED_ID_MESSAGE =
  "This cluster's kubeconfig path cannot be made absolute, so its apps cannot be opened here. Manage your kubeconfig files in Settings → Contexts.";

/**
 * The context `context` names: found; still being listed; not found because the listing
 * failed; missing from a listing that worked; or ambiguous, one context's name and another's
 * pinned ID, so neither.
 */
export type ContextLookup =
  | { status: "found"; id: string }
  | { status: "loading" }
  | { status: "failed"; error: string }
  | { status: "missing" }
  | { status: "ambiguous" };

export function useContextLookup(context: string): ContextLookup {
  const { contexts, error } = useSyncExternalStore(subscribe, getState, getState);
  const lookup = contexts && lookUp(contexts, context);
  if (lookup && lookup.status !== "missing") return lookup;
  if (error) return { status: "failed", error };
  return lookup ?? { status: "loading" };
}

/** What an app surface says when its cluster's name is also another context's pinned ID. */
export const AMBIGUOUS_CONTEXT_MESSAGE =
  "This cluster's name is also another context's ID, so this page cannot tell which one it is for. Rename one of them in your kubeconfig files.";
