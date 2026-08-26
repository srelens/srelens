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

const ctx = (stableId: string, name = stableId): ClusterContext => ({
  name, stableId, cluster: name, server: "", isCurrent: false,
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
