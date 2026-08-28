import { describe, it, expect, beforeEach } from "vitest";
import type { ClusterContext } from "@srelens/core";
import {
  contextFor,
  getContexts,
  getContextsError,
  getContextsStatus,
  resetContexts,
  setContexts,
} from "./clusters";
import { defaultState } from "./tabs";
import * as store from "./tabsStore";

const ctx = (stableId: string, name = stableId): ClusterContext => ({
  name, stableId, cluster: name, server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
});

describe("contexts store", () => {
  beforeEach(resetContexts);

  it("resolves a stableId to the context whose name core's calls take", () => {
    setContexts([ctx("prod-1", "prod"), ctx("dev-1", "dev")]);
    expect(contextFor("prod-1")?.name).toBe("prod");
  });

  it("answers undefined for a cluster the kubeconfig no longer declares", () => {
    setContexts([ctx("prod-1", "prod")]);
    expect(contextFor("gone")).toBeUndefined();
    expect(contextFor(null)).toBeUndefined();
  });

  it("hands back the same array until it is replaced, so a subscriber cannot tear", () => {
    setContexts([ctx("prod-1")]);
    expect(getContexts()).toBe(getContexts());
  });
});

/**
 * An empty list is three different facts, and the screens were reading it as
 * one: nothing selected, nothing listed yet, and a listing that failed all
 * arrived as `[]`. The store is the only place that knows which — `Window` is
 * where the difference is observed, and every screen reads it back from here.
 */
describe("contexts store — listed, not listed yet, or refused", () => {
  beforeEach(resetContexts);

  it("starts out not yet loaded, because an empty list before boot is evidence of nothing", () => {
    expect(getContextsStatus()).toBe("loading");
    expect(getContextsError()).toBe("");
    expect(getContexts()).toEqual([]);
  });

  it("is loaded once a listing has answered, even when the answer is none", () => {
    setContexts([]);
    expect(getContextsStatus()).toBe("loaded");
    expect(getContextsError()).toBe("");
  });

  it("is failed, with the backend's reason kept, when the listing could not be made", () => {
    setContexts([], "kubeconfig unreadable");
    expect(getContextsStatus()).toBe("failed");
    expect(getContextsError()).toBe("kubeconfig unreadable");
  });

  it("holds a partial list and its reason together, since some contexts can arrive alongside a refusal", () => {
    setContexts([ctx("prod-1")], "kubeconfig unreadable");
    expect(getContexts()).toHaveLength(1);
    expect(getContextsStatus()).toBe("failed");
  });

  it("clears an earlier failure when a later listing succeeds", () => {
    setContexts([], "kubeconfig unreadable");
    setContexts([ctx("prod-1")]);
    expect(getContextsStatus()).toBe("loaded");
    expect(getContextsError()).toBe("");
  });

  it("resets the status and the reason too, so one test's failure cannot leak into the next", () => {
    setContexts([ctx("prod-1")], "kubeconfig unreadable");
    resetContexts();
    expect(getContextsStatus()).toBe("loading");
    expect(getContextsError()).toBe("");
    expect(getContexts()).toEqual([]);
  });
});

/**
 * **A listing is not only a list — it can invalidate the workspace, and the
 * store is what has to say so.**
 *
 * `Window` already knew this and reconciled its boot listing against the
 * restored workspaces. Neither connection screen did: a reader who removed or
 * renamed the ACTIVE context and pressed `Refresh all` got a replaced context
 * list and a workspace still pointing at the cluster that had gone, so
 * `useActiveContext()` answered `undefined` and every cluster-scoped screen
 * fell to its no-cluster state — while other clusters were sitting right there
 * in the table. `/connect`'s own `reload` had the same gap.
 *
 * So it happens HERE, in the one write both screens and `Window` go through,
 * rather than three times in three callers. Same lesson as this branch's
 * loaded/failed signal, which exists because an empty list was three facts
 * being read as one: a store write that invalidates other state has to say so.
 */
describe("contexts store — a listing that invalidates the workspace", () => {
  const PROD = ctx("prod-1", "prod");
  const DEV = ctx("dev-1", "dev");

  beforeEach(() => {
    resetContexts();
    store.setState(defaultState([PROD, DEV]));
    setContexts([PROD, DEV]);
  });

  it("moves the focus onto a surviving cluster when a listing drops the active one", () => {
    expect(store.activeCluster()).toBe("prod-1");
    setContexts([DEV]);
    expect(store.activeCluster()).toBe("dev-1");
    expect(store.currentWorkspace().clusters).toEqual(["dev-1"]);
  });

  it("leaves the focus where it is when the active cluster survives", () => {
    setContexts([DEV, PROD]);
    expect(store.activeCluster()).toBe("prod-1");
  });

  /**
   * The workspace is not rewritten when nothing about it changed — identity is
   * the signal in that store, and one of its subscribers writes a file. A
   * refresh that answers with the same clusters must not schedule a save.
   */
  it("does not touch the workspace when the listing changes nothing about it", () => {
    const before = store.getState();
    setContexts([PROD, DEV]);
    expect(store.getState()).toBe(before);
  });

  /**
   * **A listing that REFUSED takes nothing away, and this is the case that
   * makes the guard load-bearing.**
   *
   * `Window` deliberately keeps the restored cluster ids when `listContexts`
   * refuses — reconciling against nothing would strip every workspace's
   * clusters and the next change would persist that emptied state. It calls
   * `setContexts(found, failure)` with `found` empty on that path, so
   * reconciling on a failed listing here would undo the very thing that branch
   * exists to protect.
   */
  it("strips nothing from the workspace when the listing refused", () => {
    setContexts([], "kubeconfig unreadable");
    expect(store.activeCluster()).toBe("prod-1");
    expect(store.currentWorkspace().clusters).toEqual(["prod-1", "dev-1"]);
  });

  /**
   * A PARTIAL listing is a refusal too. `listContexts` can answer with some
   * contexts and an error — one of several merged kubeconfigs was unreadable —
   * and a cluster missing for that reason has not gone anywhere. The conservative
   * read is the same one `Window` takes: trust what is on disk until a listing
   * answers cleanly.
   */
  it("strips nothing when a listing came back short with a reason", () => {
    setContexts([DEV], "one kubeconfig was unreadable");
    expect(store.activeCluster()).toBe("prod-1");
    expect(store.currentWorkspace().clusters).toEqual(["prod-1", "dev-1"]);
  });
});
