import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { ASSISTANT_KEY, type ClusterContext } from "@srelens/core";

// The one backend read this module makes: the setting, read again when a
// window is focused. A stand-in, so each test says what the backend holds.
const refreshSetting = vi.hoisted(() => vi.fn<(key: string) => Promise<void>>());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), refreshSetting }));

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
  followAssistantSetting,
  getAssistant,
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

beforeEach(() => {
  // `settingsStorage` is plain `localStorage` in a test, cleared after each one.
  resetContexts();
  setContexts([prod, dev]);
  refreshSetting.mockReset().mockResolvedValue(undefined);
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
    act(() => setClusterAssistant(prod.stableId, false));
    expect(result.current).toBe(true);
    act(() => setAssistantEnabled(false));
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

  it("stops the turn in flight when its cluster is switched off, and only then", () => {
    runs.busyKey = "prod|/overview";
    runs.cluster = "prod";
    setClusterAssistant(dev.stableId, false);
    expect(runs.stopAgentRun).not.toHaveBeenCalled();
    setClusterAssistant(prod.stableId, false);
    expect(runs.stopAgentRun).toHaveBeenCalledTimes(1);
  });
});
