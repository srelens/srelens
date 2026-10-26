import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { ASSISTANT_KEY, readAssistantSetting, type ClusterContext } from "@srelens/core";

// The one backend read this module makes: the setting, read again when a
// window is focused. A stand-in, so each test says what the backend holds.
// And the write: whether it reached the backend.
const refreshSetting = vi.hoisted(() => vi.fn<(key: string) => Promise<void>>());
const flushSettingsWrites = vi.hoisted(() => vi.fn<(options?: { throwOnError?: boolean }) => Promise<void>>());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), refreshSetting, flushSettingsWrites }));

// The run store, reduced to what this module asks of it: which run is busy,
// what it is about, and the Stop.
const runs = vi.hoisted(() => ({
  busyKey: null as string | null,
  cluster: "",
  stopAgentRun: vi.fn(),
}));
vi.mock("./agentRun", () => ({
  getRunSummaries: () => (runs.busyKey ? [{ key: runs.busyKey }] : []),
  getRun: (key: string | null) => ({ busy: key !== null && key === runs.busyKey }),
  getRunSubject: (key: string | null) => (key === runs.busyKey ? { about: { cluster: runs.cluster }, route: "/" } : undefined),
  stopAgentRun: runs.stopAgentRun,
}));

import { resetContexts, setContexts } from "./clusters";
import {
  __resetAssistantForTests,
  assistantSaved,
  followAssistantSetting,
  getAssistant,
  getAssistantSaveError,
  isAssistantAllowed,
  setAssistantEnabled,
  setClusterAssistant,
  useAssistantAllowed,
} from "./assistant";

const ctx = (name: string): ClusterContext => ({
  name, stableId: `/config#${name}`, key: `/config#${name}`, cluster: name, server: `https://${name}`,
  sourceFile: "/config", authKind: "token", isCurrent: false,
});
const prod = ctx("prod");
const dev = ctx("dev");

beforeEach(async () => {
  // `settingsStorage` is plain `localStorage` in a test. A save queued by the
  // last test lands first, then the storage is cleared under this one.
  await assistantSaved();
  __resetAssistantForTests();
  localStorage.clear();
  resetContexts();
  setContexts([prod, dev]);
  refreshSetting.mockReset().mockResolvedValue(undefined);
  flushSettingsWrites.mockReset().mockResolvedValue(undefined);
  runs.busyKey = null;
  runs.cluster = "";
  runs.stopAgentRun.mockReset();
});

describe("the assistant store", () => {
  it("is on for everything until someone turns it off", () => {
    expect(getAssistant()).toEqual({ enabled: true, disabledClusters: [] });
    expect(isAssistantAllowed("prod")).toBe(true);
  });

  it("holds a switched-off cluster once, and lets it back on", () => {
    setClusterAssistant(prod.stableId, false);
    setClusterAssistant(prod.stableId, false);
    expect(getAssistant().disabledClusters).toEqual([prod.stableId]);
    expect(isAssistantAllowed("prod")).toBe(false);
    setClusterAssistant(prod.stableId, true);
    expect(getAssistant().disabledClusters).toEqual([]);
  });

  it("keeps the cluster list while global is off", () => {
    setClusterAssistant(prod.stableId, false);
    setAssistantEnabled(false);
    expect(getAssistant()).toEqual({ enabled: false, disabledClusters: [prod.stableId] });
    expect(isAssistantAllowed("dev")).toBe(false);
  });

  it("redraws a reader when its cluster or the whole assistant is switched off", () => {
    const { result } = renderHook(() => useAssistantAllowed("dev"));
    expect(result.current).toBe(true);
    act(() => { setClusterAssistant(prod.stableId, false); });
    expect(result.current).toBe(true);
    act(() => { setAssistantEnabled(false); });
    expect(result.current).toBe(false);
  });

  it("picks up a switch flipped in another window when this one is focused", async () => {
    const release = followAssistantSetting();
    const { result } = renderHook(() => useAssistantAllowed("prod"));
    expect(result.current).toBe(true);
    // The backend now holds main's write; the re-read lands it here.
    refreshSetting.mockImplementation(async (key) => {
      localStorage.setItem(key, JSON.stringify({ enabled: true, disabledClusters: [prod.stableId] }));
    });
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(refreshSetting).toHaveBeenCalledWith(ASSISTANT_KEY);
    expect(result.current).toBe(false);
    release();
  });

  // Review finding: the switch must never show a state the file — the only
  // thing the MCP gate reads — does not hold.
  it("writes against the setting as another window left it", async () => {
    // Main switched dev off after this window loaded its settings.
    refreshSetting.mockImplementation(async (key) => {
      if (!localStorage.getItem(key)?.includes(dev.stableId)) {
        localStorage.setItem(key, JSON.stringify({ enabled: true, disabledClusters: [dev.stableId] }));
      }
    });
    setClusterAssistant(prod.stableId, false);
    await assistantSaved();
    expect(readAssistantSetting().disabledClusters.sort()).toEqual([dev.stableId, prod.stableId].sort());
    expect(getAssistantSaveError()).toBeNull();
  });

  it("shows what was saved, not what was asked for, when the save fails", async () => {
    const backend = JSON.stringify({ enabled: true, disabledClusters: [] });
    refreshSetting.mockImplementation(async (key) => localStorage.setItem(key, backend));
    flushSettingsWrites.mockRejectedValue(new Error("settings.json is locked"));
    const { result } = renderHook(() => useAssistantAllowed("dev"));

    act(() => { setAssistantEnabled(false); });
    expect(result.current).toBe(false); // shown at once
    await act(async () => {
      await assistantSaved();
    });

    expect(result.current).toBe(true); // back to what the file holds
    expect(getAssistant().enabled).toBe(true);
    expect(String(getAssistantSaveError())).toContain("settings.json is locked");
  });

  // Review (PR #984): a failed save whose re-read also fails must not leave
  // the unconfirmed value in memory.
  it("goes back to the last confirmed value when the save and the read after it both fail", async () => {
    let reads = 0;
    refreshSetting.mockImplementation(async (key) => {
      reads += 1;
      if (reads === 1) {
        localStorage.setItem(key, JSON.stringify({ enabled: true, disabledClusters: [] }));
        return;
      }
      throw new Error("backend unreachable");
    });
    flushSettingsWrites.mockRejectedValue(new Error("settings.json is locked"));
    vi.spyOn(console, "warn").mockImplementation(() => {});

    setAssistantEnabled(false);
    await assistantSaved();

    expect(getAssistant().enabled).toBe(true);
    expect(String(getAssistantSaveError())).toContain("settings.json is locked");
  });

  it("stops the turn in flight when its cluster is switched off, and only then", () => {
    runs.busyKey = "prod|/overview";
    runs.cluster = "prod";
    setClusterAssistant(dev.stableId, false);
    expect(runs.stopAgentRun).not.toHaveBeenCalled();
    setClusterAssistant(prod.stableId, false);
    expect(runs.stopAgentRun).toHaveBeenCalledTimes(1);
  });
});
