import { useSyncExternalStore } from "react";
import { refreshSetting } from "@srelens/core";
import { getContexts, useContexts } from "./clusters";
import { getMark, useMark } from "./marks";
import type { Workspace } from "./tabs";
import { getStorageKey, loadTabsState } from "./tabsPersist";
import { getState, subscribe } from "./tabsStore";

/**
 * Protected workspaces (#101): whether a change to a cluster asks for its
 * name to be typed first.
 *
 * **Protection follows the cluster.** The flag is set on a workspace, and a
 * cluster is protected when ANY workspace holding it is, so acting on prod
 * from an unprotected "All" workspace still asks.
 *
 * **This window's workspaces, then the main window's save.** A context window
 * copies main's workspaces once, when it first opens, and keeps its own copy
 * from then on, so main's save is read as well: a context window can add
 * protection, never remove what main set. The save is read through
 * `loadTabsState`, which honours "Restore session": with it off, a save left
 * by an earlier session protects nothing.
 */
export interface Protection {
  /** What the reader types: the cluster's mark name, `getMark(stableId, contextName).name`. */
  name: string;
  /** The first protected workspace holding the cluster, for copy: "(workspace Production)". */
  workspace: string;
}

// ponytail: main's save is parsed on every call. In main it trails the store
// by the 300 ms save debounce, which errs only toward protected; a context
// window sees main's later saves when it is focused (followMainProtection).
// Cache by the raw string if this ever shows in a profile.
function workspaces(): Workspace[] {
  return [...getState().workspaces, ...(loadTabsState(undefined, undefined, "main")?.workspaces ?? [])];
}

/** Name of the first protected workspace holding the cluster (this window first, then main's save), or undefined. */
export function protectingWorkspace(stableId: string): string | undefined {
  return workspaces().find((w) => w.protected === true && w.clusters.includes(stableId))?.name;
}

/** stableId → protected? This window's store OR the main window's saved state. */
export function isClusterProtected(stableId: string): boolean {
  return protectingWorkspace(stableId) !== undefined;
}

/**
 * A cluster, named any way the host resolves one (`find_context`,
 * `crates/kube/src/context_resolve.rs`) → Protection, or null when unknown or
 * not protected. Accepted: the context's name, its pinned ID
 * (`srelens-context:…`, which app surfaces and a sidecar's consent request
 * name their cluster by), its stableId, its key, and the kubeconfig's own
 * name for it (`stableId` past `sourceFile#`; the display name may have
 * gained a `file/` prefix). Matching more can only ask more.
 *
 * Every context that matches is tried, not the first: two kubeconfig files
 * can name two clusters alike, and an unprotected one must not hide a
 * protected twin.
 */
export function protectionFor(contextName: string | null | undefined): Protection | null {
  if (!contextName) return null;
  for (const context of getContexts()) {
    const file = `${context.sourceFile}#`;
    const original = context.stableId.startsWith(file) ? context.stableId.slice(file.length) : undefined;
    const named = [context.name, context.pinnedId, context.stableId, context.key, original];
    if (!named.includes(contextName)) continue;
    const workspace = protectingWorkspace(context.stableId);
    if (workspace !== undefined) return { name: getMark(context.stableId, context.name).name, workspace };
  }
  return null;
}

/** How many times a focus has brought main's latest save into this window. */
let mainSaveReads = 0;
const mainSaveListeners = new Set<() => void>();

function onMainSaveRead(listener: () => void): () => void {
  mainSaveListeners.add(listener);
  return () => {
    mainSaveListeners.delete(listener);
  };
}

const mainSaveReadCount = () => mainSaveReads;

/**
 * In a context window, read main's saved workspaces again each time the
 * window is focused, then redraw what `useProtection` drew. A window reads its
 * settings once, at boot, so protection turned on in main afterwards would
 * otherwise not reach a window that is already open. A read that fails leaves
 * this window's copy in use, and is logged. The main window holds the
 * workspaces itself and listens to nothing. Returns the release.
 */
export function followMainProtection(windowLabel: string): () => void {
  if (windowLabel === "main") return () => {};
  const onFocus = () => {
    refreshSetting(getStorageKey("main")).then(
      () => {
        mainSaveReads += 1;
        for (const listener of mainSaveListeners) listener();
      },
      // Not readable again: this window's copy is the best there is. Said, not
      // swallowed, since it is the one way protection main set can miss this
      // window; the sibling read in `loadTabsState` logs the same way.
      (error) => console.warn("could not re-read main's protection", error),
    );
  };
  window.addEventListener("focus", onFocus);
  return () => window.removeEventListener("focus", onFocus);
}

/** Reactive protectionFor: re-renders on tabs-store, contexts-store, marks and main-save changes. */
export function useProtection(contextName: string | null | undefined): Protection | null {
  useSyncExternalStore(subscribe, getState, getState);
  useSyncExternalStore(onMainSaveRead, mainSaveReadCount, mainSaveReadCount);
  useContexts();
  // The marks store's "subscribe once", as `WorkspacesPane` and `Window` do.
  useMark("", "");
  return protectionFor(contextName);
}
