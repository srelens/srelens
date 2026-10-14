import { beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ClusterContext } from "@srelens/core";

// The one backend read this module makes: main's saved workspaces, read again
// when a context window is focused. A stand-in, so each test says what it brings in.
const refreshSetting = vi.hoisted(() => vi.fn<(key: string) => Promise<void>>());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), refreshSetting }));

import { resetContexts, setContexts } from "./clusters";
import { defaultMark, loadMarks, setMark } from "./marks";
import {
  followMainProtection,
  isClusterProtected,
  protectingWorkspace,
  protectionFor,
  useProtection,
} from "./protection";
import { defaultState } from "./tabs";
import { getStorageKey, saveTabsState } from "./tabsPersist";
import { createWorkspace, getState, setState, setWorkspaceProtected, switchWorkspace } from "./tabsStore";

/**
 * A stableId that is not the name. Workspaces hold stableIds (#265) and
 * callers hand a context NAME, so a fixture where the two are one string
 * could not tell a lookup by the wrong one.
 */
const ctx = (name: string): ClusterContext => ({
  name, stableId: `${name}-id`, key: `${name}-id`, cluster: name, server: `https://${name}`,
  sourceFile: "/config", authKind: "token", isCurrent: false,
});
const prod = ctx("prod-eu");
const staging = ctx("staging");

/** A protected workspace in THIS window's store. */
function protect(name: string, clusters: string[]): string {
  const id = createWorkspace(name, clusters);
  setWorkspaceProtected(id, true);
  return id;
}

/** The main window's save: one protected workspace, where `loadTabsState(…, "main")` reads it. */
function protectInMain(name: string, clusters: ClusterContext[]): void {
  const main = defaultState(clusters);
  main.workspaces[0] = { ...main.workspaces[0], name, protected: true };
  saveTabsState(main, undefined, "main");
}

beforeEach(() => {
  // `settingsStorage` is plain `localStorage` in a test, and the shared setup
  // clears it after each one: no marks, and no save from main.
  loadMarks();
  resetContexts();
  setState(defaultState([prod, staging])); // Default: both clusters, not protected
  setContexts([prod, staging]);
  refreshSetting.mockReset().mockResolvedValue(undefined);
});

describe("isClusterProtected and protectingWorkspace", () => {
  it("protects a cluster a protected workspace holds, whichever workspace is current", () => {
    protect("Production", [prod.stableId]);
    switchWorkspace(getState().workspaces[0].id); // back to Default, which is not protected
    expect(isClusterProtected(prod.stableId)).toBe(true);
    expect(protectingWorkspace(prod.stableId)).toBe("Production");
  });

  it("does not protect a cluster only unprotected workspaces hold", () => {
    protect("Production", [prod.stableId]);
    createWorkspace("Staging", [staging.stableId]);
    expect(isClusterProtected(staging.stableId)).toBe(false);
    expect(protectingWorkspace(staging.stableId)).toBeUndefined();
  });

  it("protects a cluster only the main window's saved state protects", () => {
    // A context window: its own store protects nothing; main's save does.
    protectInMain("Production", [prod]);
    expect(isClusterProtected(prod.stableId)).toBe(true);
    expect(protectingWorkspace(prod.stableId)).toBe("Production");
    expect(isClusterProtected(staging.stableId)).toBe(false);
  });

  it("names this window's protecting workspace before the main window's", () => {
    protectInMain("Main's copy", [prod]);
    protect("Production", [prod.stableId]);
    expect(protectingWorkspace(prod.stableId)).toBe("Production");
  });
});

describe("protectionFor", () => {
  it("answers with the cluster's mark name and the workspace protecting it", () => {
    protect("Production", [prod.stableId]);
    expect(protectionFor("prod-eu")).toEqual({ name: "prod-eu", workspace: "Production" });
    setMark(prod.stableId, { ...defaultMark("prod-eu"), name: "Production EU" });
    expect(protectionFor("prod-eu")).toEqual({ name: "Production EU", workspace: "Production" });
  });

  it("is null for an unprotected cluster, an unknown name, a stableId, or no name", () => {
    protect("Production", [prod.stableId]);
    expect(protectionFor("staging")).toBeNull();
    expect(protectionFor("nope")).toBeNull();
    expect(protectionFor(prod.stableId)).toBeNull();
    expect(protectionFor("")).toBeNull();
    expect(protectionFor(null)).toBeNull();
    expect(protectionFor(undefined)).toBeNull();
  });

  it("finds a protected cluster behind an unprotected one of the same name", () => {
    // Two kubeconfig files can name two different clusters alike.
    const twin: ClusterContext = { ...prod, stableId: "twin-id", key: "twin-id", sourceFile: "/other" };
    setContexts([prod, twin, staging]);
    protect("Production", [twin.stableId]);
    expect(protectionFor("prod-eu")).toEqual({ name: "prod-eu", workspace: "Production" });
  });

  it("reads a context named by its pinned ID, as an app page names it", () => {
    const pinned: ClusterContext = { ...prod, pinnedId: "srelens-context:/config#prod-eu" };
    const other: ClusterContext = { ...staging, pinnedId: "srelens-context:/config#staging" };
    setContexts([pinned, other]);
    protect("Production", [prod.stableId]);
    expect(protectionFor("srelens-context:/config#prod-eu")).toEqual({ name: "prod-eu", workspace: "Production" });
    expect(protectionFor("srelens-context:/config#staging")).toBeNull();
  });
});

/**
 * Each case changes ONE store, so each subscription is the only thing that
 * can redraw it: `setWorkspaceProtected` wakes only the tabs store,
 * `setMark` only the marks, and `setContexts` here only the contexts — its
 * reconcile finds nothing to change and its mark migration has nothing to move.
 */
describe("useProtection", () => {
  it("follows the workspace being protected and unprotected", () => {
    const id = createWorkspace("Production", [prod.stableId]);
    const { result } = renderHook(() => useProtection("prod-eu"));
    expect(result.current).toBeNull();
    act(() => setWorkspaceProtected(id, true));
    expect(result.current).toEqual({ name: "prod-eu", workspace: "Production" });
    act(() => setWorkspaceProtected(id, false));
    expect(result.current).toBeNull();
  });

  it("follows the contexts being listed", () => {
    resetContexts();
    protect("Production", [prod.stableId]);
    const { result } = renderHook(() => useProtection("prod-eu"));
    expect(result.current).toBeNull();
    act(() => setContexts([prod, staging]));
    expect(result.current).toEqual({ name: "prod-eu", workspace: "Production" });
  });

  it("follows the cluster's mark being renamed", () => {
    protect("Production", [prod.stableId]);
    const { result } = renderHook(() => useProtection("prod-eu"));
    expect(result.current?.name).toBe("prod-eu");
    act(() => setMark(prod.stableId, { ...defaultMark("prod-eu"), name: "Production EU" }));
    expect(result.current?.name).toBe("Production EU");
  });
});

/**
 * A context window copied main's workspaces when it opened, and its settings
 * were read once at boot. Main may protect a cluster later; the window reads
 * main's save again each time it is focused, before the reader acts in it.
 */
describe("followMainProtection", () => {
  const focus = () => window.dispatchEvent(new Event("focus"));

  it("re-reads main's save when a context window is focused, and redraws", async () => {
    onTestFinished(followMainProtection("ctx-prod"));
    // What the refresh brings in: main has protected prod-eu since this window opened.
    refreshSetting.mockImplementation(async () => protectInMain("Production", [prod]));
    const { result } = renderHook(() => useProtection("prod-eu"));
    expect(result.current).toBeNull();

    await act(async () => focus());

    expect(refreshSetting).toHaveBeenCalledWith(getStorageKey("main"));
    await waitFor(() => expect(result.current).toEqual({ name: "prod-eu", workspace: "Production" }));
  });

  it("asks nothing in the main window, whose store is the workspaces", () => {
    onTestFinished(followMainProtection("main"));
    act(() => focus());
    expect(refreshSetting).not.toHaveBeenCalled();
  });

  it("stops listening when released", () => {
    followMainProtection("ctx-prod")();
    act(() => focus());
    expect(refreshSetting).not.toHaveBeenCalled();
  });

  it("keeps this window's copy, and says so, when main's save cannot be read again", async () => {
    onTestFinished(followMainProtection("ctx-prod"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    onTestFinished(() => warn.mockRestore());
    const failure = new Error("backend gone");
    refreshSetting.mockRejectedValue(failure);
    // This window's own store protects prod-eu: that is what must survive a
    // refresh that fails. An unprotected window would read null before and after.
    protect("Production", [prod.stableId]);
    const production = { name: "prod-eu", workspace: "Production" };
    const { result } = renderHook(() => useProtection("prod-eu"));
    expect(result.current).toEqual(production);

    await act(async () => focus());

    expect(refreshSetting).toHaveBeenCalledTimes(1);
    expect(result.current).toEqual(production);
    expect(protectionFor("prod-eu")).toEqual(production);
    // The one place protection can fail open: the reader is told why it may be stale.
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledWith("could not re-read main's protection", failure);
  });
});
