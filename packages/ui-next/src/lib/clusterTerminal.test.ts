import { beforeEach, describe, expect, it, vi } from "vitest";

const isTauri = vi.fn(() => true);
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  isTauri: () => isTauri(),
}));

const startLocalSession = vi.fn();
let sessions: Array<{ id: number; context: string; state: string }> = [];
vi.mock("./sessions", () => ({
  startLocalSession: (...args: unknown[]) => startLocalSession(...args),
  getSessions: () => sessions,
}));

const { canOpenClusterTerminal, openClusterTerminal, terminalNamespace } = await import(
  "./clusterTerminal"
);
const { activeRoute, currentWorkspace, openTab, setClusterPaused, setState, setTabNamespaces } = await import(
  "./tabsStore"
);
const { defaultState } = await import("./tabs");
const { __resetTerminalPanelsForTests, closeTerminalPanel, terminalPanel } = await import("./terminalPanel");

const dev = {
  name: "dev-cluster", stableId: "dev", key: "dev", cluster: "c", server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
};
const other = { ...dev, name: "other-cluster", stableId: "other", key: "other" };

beforeEach(() => {
  isTauri.mockReturnValue(true);
  startLocalSession.mockReset().mockResolvedValue(1);
  setState(defaultState([dev, other]));
  sessions = [];
  __resetTerminalPanelsForTests();
});

/** Put the active tab on a page other than Terminals, as the reader would be. */
function onPods(): string {
  openTab("/pods");
  return currentWorkspace().activeId;
}

describe("terminalNamespace", () => {
  it("is the tab's namespace when the tab is looking at exactly one", () => {
    expect(terminalNamespace(["payments"])).toBe("payments");
  });

  it("is none when the tab is looking at several, at all, or has not chosen", () => {
    // Any one of several would be a guess `kubectl get pods` then answers.
    expect(terminalNamespace(["payments", "billing"])).toBeUndefined();
    expect(terminalNamespace([])).toBeUndefined();
    expect(terminalNamespace(undefined)).toBeUndefined();
  });
});

describe("canOpenClusterTerminal", () => {
  it("is true for a cluster on the desktop", () => {
    expect(canOpenClusterTerminal(dev)).toBe(true);
  });

  it("is false with no cluster", () => {
    expect(canOpenClusterTerminal(undefined)).toBe(false);
  });

  it("is false on the web, which has no local shell", () => {
    isTauri.mockReturnValue(false);
    expect(canOpenClusterTerminal(dev)).toBe(false);
  });

  it("is false for a cluster this window has paused", () => {
    setClusterPaused(currentWorkspace().id, dev.stableId, true);
    expect(canOpenClusterTerminal(dev)).toBe(false);
    expect(canOpenClusterTerminal(other)).toBe(true);
  });
});

describe("openClusterTerminal", () => {
  it("starts a local shell for the cluster it was given, under the page it was opened on (#942)", async () => {
    const tab = onPods();
    await openClusterTerminal(dev);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "dev-cluster", namespace: undefined });
    // The reader stays where they were; the shell is in that tab's panel.
    expect(activeRoute()).toBe("/pods");
    expect(terminalPanel(tab)).toEqual({ shellId: 1, open: true, maximized: false });
  });

  it("shows the session in the Terminals tab when that is where it was asked for", async () => {
    openTab("/terminals");
    await openClusterTerminal(dev);
    expect(activeRoute()).toBe("/terminals");
    expect(terminalPanel(currentWorkspace().activeId)).toBeUndefined();
  });

  it("starts in the active tab's namespace for that cluster when it has exactly one", async () => {
    setTabNamespaces(currentWorkspace().activeId, dev.stableId, ["payments"]);
    await openClusterTerminal(dev);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "dev-cluster", namespace: "payments" });
  });

  it("does not borrow another cluster's namespace selection", async () => {
    setTabNamespaces(currentWorkspace().activeId, other.stableId, ["payments"]);
    await openClusterTerminal(dev);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "dev-cluster", namespace: undefined });
  });

  it("opens the panel only once the session exists", async () => {
    const tab = onPods();
    let started!: () => void;
    startLocalSession.mockReturnValue(new Promise<number>((resolve) => (started = () => resolve(1))));
    const opening = openClusterTerminal(dev);
    expect(terminalPanel(tab)).toBeUndefined();
    started();
    await opening;
    expect(terminalPanel(tab)?.open).toBe(true);
  });

  it("closes an open panel, leaving its shell running", async () => {
    const tab = onPods();
    await openClusterTerminal(dev);
    sessions = [{ id: 1, context: "dev-cluster", state: "attached" }];
    await openClusterTerminal(dev);
    expect(terminalPanel(tab)?.open).toBe(false);
    expect(startLocalSession).toHaveBeenCalledTimes(1);
  });

  it("opens again on the shell it last showed, rather than starting a second", async () => {
    const tab = onPods();
    await openClusterTerminal(dev);
    sessions = [{ id: 1, context: "dev-cluster", state: "idle" }];
    closeTerminalPanel(tab);
    await openClusterTerminal(dev);
    expect(terminalPanel(tab)).toEqual({ shellId: 1, open: true, maximized: false });
    expect(startLocalSession).toHaveBeenCalledTimes(1);
  });

  it("starts a new shell when the one it last showed has ended, or is on another cluster", async () => {
    const tab = onPods();
    await openClusterTerminal(dev);
    sessions = [{ id: 1, context: "dev-cluster", state: "closed" }];
    closeTerminalPanel(tab);
    startLocalSession.mockResolvedValue(2);
    await openClusterTerminal(dev);
    expect(terminalPanel(tab)?.shellId).toBe(2);

    sessions = [{ id: 2, context: "dev-cluster", state: "attached" }];
    closeTerminalPanel(tab);
    startLocalSession.mockResolvedValue(3);
    await openClusterTerminal(other);
    expect(startLocalSession).toHaveBeenLastCalledWith({ context: "other-cluster", namespace: undefined });
    expect(terminalPanel(tab)?.shellId).toBe(3);
  });
});
