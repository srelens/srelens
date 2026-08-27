import { useSyncExternalStore } from "react";
import type { ClusterContext } from "@srelens/core";
import { useActiveCluster } from "./tabsStore";

/**
 * The cluster contexts this window knows about.
 *
 * Workspaces hold `ClusterContext.stableId`s (#265); core's `list*`,
 * `watchResource` and `useNamespaceOptions` all take a context *name*. Screens
 * receive only `{ route }` and read their state from stores, so without this
 * there is nowhere for a screen to make that translation — `Window` held the
 * list in `useState` and passed it as props to the two components that needed
 * it.
 *
 * **Three writers, not one.** `Window` sets it at boot from the `listContexts`
 * call it already makes; `Connections.reload()` and `Connect.reload()` both
 * write their own listing back through {@link setContexts}, which is what makes
 * `Refresh all` and either door shared rather than private — the rail, the
 * status bar and every other screen see the list those screens are drawing.
 *
 * This comment said "Window is still the only writer" while both screens wrote
 * to it, and this branch was already bitten once by a screen depending on a
 * store invariant the store knew nothing about (`/connections` cleared its
 * in-flight facts only on its OWN reload, latent purely because Window's write
 * sits behind `if (!booted)`). So: any writer must assume another may write
 * next, and nothing here promises otherwise.
 */
let contexts: ClusterContext[] = [];
const listeners = new Set<() => void>();

/**
 * Whether the list above has been asked for yet, and what came back.
 *
 * An empty `contexts` is three different facts and the screens were reading it
 * as one. A user with `K8SM01-ADMIN` on every tab was told "No cluster in
 * focus — pick a cluster in the rail", because `listContexts` had refused and
 * `Window` deliberately keeps the saved cluster ids when it does (see the
 * comment at its `saved && outcome.error` branch): the cluster *is* in focus,
 * the rail cannot fix it, and srelens already knew the real reason. The same
 * screen also appeared for the ordinary race of a slow listing.
 *
 * So the store says which of the three it is, and the shared no-cluster screen
 * says only what is true: still listing, could not list, or genuinely nothing
 * picked.
 */
export type ContextsStatus = "loading" | "loaded" | "failed";

let status: ContextsStatus = "loading";
let failure = "";

export function getContexts(): ClusterContext[] {
  return contexts;
}

/** The store's own read of itself — a string, deliberately not a `{ status,
 *  error }` object. `useSyncExternalStore` re-reads its snapshot after every
 *  render and compares it by identity, so a getter that allocates per call
 *  never settles: "Maximum update depth exceeded", which this package has
 *  already shipped once. Two primitives, two getters, no allocation. */
export function getContextsStatus(): ContextsStatus {
  return status;
}

/** The raw message a failed listing came back with, for `describeError` to
 *  classify at the point it is shown. Empty unless {@link getContextsStatus}
 *  reads `failed`. */
export function getContextsError(): string {
  return failure;
}

/**
 * The result of one listing: what came back, and the reason if it refused.
 *
 * One call rather than a separate `setContextsError`, so the three states can
 * never be observed half-changed — a list installed before its error, or an
 * error left standing over a list that has since succeeded.
 */
export function setContexts(next: ClusterContext[], error = ""): void {
  contexts = next;
  status = error === "" ? "loaded" : "failed";
  failure = error;
  for (const listener of listeners) listener();
}

/**
 * The kubeconfig files the backend must know about before a client can be built
 * for a context that came from one of them. Read by every core call that takes
 * them.
 *
 * **Not resolved once at boot, which this said.** It is seeded at boot and
 * written again by `Connections.addFile()` and by `Connect.remember()` — a file
 * the reader has just picked has to be here before the listing that follows,
 * or the backend cannot build a client for any context in it. Deliberately not
 * reactive: every writer re-lists in the same breath, and the listing is what
 * re-renders the screens.
 */
let files: string[] = [];

export function setKubeconfigFiles(next: string[]): void {
  files = next;
}

export function getKubeconfigFiles(): string[] {
  return files;
}

/** Test-only: put the store back to how it boots — nothing listed, and no
 *  listing attempted yet. `setContexts([])` would say the opposite (a listing
 *  that answered with none), and a leftover `failed` from one test would
 *  otherwise render every later test's no-cluster screen as an error. */
export function resetContexts(): void {
  files = [];
  contexts = [];
  status = "loading";
  failure = "";
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useContexts(): ClusterContext[] {
  return useSyncExternalStore(subscribe, getContexts, getContexts);
}

/** Whether the contexts have been listed yet, and whether the listing worked. */
export function useContextsStatus(): ContextsStatus {
  return useSyncExternalStore(subscribe, getContextsStatus, getContextsStatus);
}

/** Why the listing refused, when it did. */
export function useContextsError(): string {
  return useSyncExternalStore(subscribe, getContextsError, getContextsError);
}

/** The context a workspace's cluster id stands for, if the kubeconfig still has it. */
export function contextFor(stableId: string | null | undefined): ClusterContext | undefined {
  if (!stableId) return undefined;
  return contexts.find((c) => c.stableId === stableId);
}

/**
 * The active cluster's context, re-rendering on a change to either the store or
 * the active cluster. A screen with no answer here renders its "no cluster"
 * state rather than calling core with an empty context name.
 *
 * `undefined` says only that the two halves did not meet — it does not say
 * which half was missing, and the three ways that happens want three different
 * sentences. {@link ContextsStatus} is how a screen tells them apart; see
 * `NoClusterScreen`.
 */
export function useActiveContext(): ClusterContext | undefined {
  const active = useActiveCluster();
  const all = useContexts();
  return active ? all.find((c) => c.stableId === active) : undefined;
}
