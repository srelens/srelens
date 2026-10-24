import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

// Whether a cluster is protected is `./protection`'s to say, and tested there.
const protectionFor = vi.fn(
  (_context: string | null | undefined): { name: string; workspace: string } | null => null,
);
vi.mock("./protection", () => ({
  protectionFor: (context: string | null | undefined) => protectionFor(context),
}));

const {
  answerClusterTerminal,
  canOpenClusterTerminal,
  localShellCanBeReadOnly,
  openClusterTerminal,
  pendingClusterTerminal,
  terminalNamespace,
} = await import("./clusterTerminal");
const { activeRoute, currentWorkspace, openTab, setClusterPaused, setState, setTabNamespaces } = await import(
  "./tabsStore"
);
const { defaultState } = await import("./tabs");
const { __resetTerminalPanelsForTests, closeTerminalPanel, setTerminalPanelMinimized, terminalPanel } = await import("./terminalPanel");

const dev = {
  name: "dev-cluster", stableId: "dev", key: "dev", cluster: "c", server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
};
const other = { ...dev, name: "other-cluster", stableId: "other", key: "other" };

const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
let agent: { mockRestore(): void } | undefined;

/** The platform the window runs on, as its WebView reports it. */
function onPlatform(userAgent: string) {
  agent = vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue(userAgent);
}

beforeEach(async () => {
  await answerClusterTerminal(false);
  isTauri.mockReturnValue(true);
  startLocalSession.mockReset().mockResolvedValue(1);
  protectionFor.mockReset().mockReturnValue(null);
  setState(defaultState([dev, other]));
  sessions = [];
  __resetTerminalPanelsForTests();
});

afterEach(() => {
  agent?.mockRestore();
  agent = undefined;
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
    expect(terminalPanel(tab)).toEqual({ shellId: 1, open: true, maximized: false, minimized: false });
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

  it("brings a minimized panel back up rather than closing it", async () => {
    const tab = onPods();
    await openClusterTerminal(dev);
    sessions = [{ id: 1, context: "dev-cluster", state: "attached" }];
    setTerminalPanelMinimized(tab, true);
    await openClusterTerminal(dev);
    expect(terminalPanel(tab)).toEqual({ shellId: 1, open: true, maximized: false, minimized: false });
  });

  it("opens the asked-for cluster's shell over an open panel on another cluster, or one that ended", async () => {
    const tab = onPods();
    await openClusterTerminal(dev);
    sessions = [{ id: 1, context: "dev-cluster", state: "attached" }];
    startLocalSession.mockResolvedValue(2);
    // Open, and minimized: neither is the reader's to toggle on another cluster.
    setTerminalPanelMinimized(tab, true);
    await openClusterTerminal(other);
    expect(startLocalSession).toHaveBeenLastCalledWith({ context: "other-cluster", namespace: undefined });
    expect(terminalPanel(tab)).toEqual({ shellId: 2, open: true, maximized: false, minimized: false });

    // An open panel whose shell has ended is replaced, not closed.
    sessions = [{ id: 2, context: "other-cluster", state: "closed" }];
    startLocalSession.mockResolvedValue(3);
    await openClusterTerminal(other);
    expect(terminalPanel(tab)).toEqual({ shellId: 3, open: true, maximized: false, minimized: false });
  });

  it("opens again on the shell it last showed, rather than starting a second", async () => {
    const tab = onPods();
    await openClusterTerminal(dev);
    sessions = [{ id: 1, context: "dev-cluster", state: "idle" }];
    closeTerminalPanel(tab);
    await openClusterTerminal(dev);
    expect(terminalPanel(tab)).toEqual({ shellId: 1, open: true, maximized: false, minimized: false });
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

describe("localShellCanBeReadOnly", () => {
  it("is true where the host writes its guards", () => {
    onPlatform(MAC);
    expect(localShellCanBeReadOnly()).toBe(true);
  });

  it("is false on Windows, which has none", () => {
    onPlatform(WINDOWS);
    expect(localShellCanBeReadOnly()).toBe(false);
  });
});

describe("openClusterTerminal on a protected cluster (#101)", () => {
  beforeEach(() => {
    protectionFor.mockImplementation((context) =>
      context === "dev-cluster" ? { name: "Dev", workspace: "Production" } : null,
    );
  });

  it("opens at once where the host makes the shell read-only instead", async () => {
    onPlatform(MAC);
    const tab = onPods();
    await openClusterTerminal(dev);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "dev-cluster", namespace: undefined });
    expect(pendingClusterTerminal()).toBeNull();
    expect(terminalPanel(tab)?.open).toBe(true);
  });

  it("on Windows, holds the shell for the reader's typed name rather than opening it", async () => {
    onPlatform(WINDOWS);
    const tab = onPods();
    await openClusterTerminal(dev);
    expect(startLocalSession).not.toHaveBeenCalled();
    expect(activeRoute()).toBe("/pods");
    expect(terminalPanel(tab)).toBeUndefined();
    expect(pendingClusterTerminal()).toEqual({ cluster: dev, namespace: undefined, tab, inPanel: true });
  });

  it("on Windows, opens a cluster no workspace protects at once", async () => {
    onPlatform(WINDOWS);
    await openClusterTerminal(other);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "other-cluster", namespace: undefined });
    expect(pendingClusterTerminal()).toBeNull();
  });

  it("opens what it held once the reader says yes, in the namespace read when they asked", async () => {
    onPlatform(WINDOWS);
    const tab = onPods();
    setTabNamespaces(tab, dev.stableId, ["payments"]);
    await openClusterTerminal(dev);
    setTabNamespaces(tab, dev.stableId, ["billing"]);
    await answerClusterTerminal(true);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "dev-cluster", namespace: "payments" });
    expect(terminalPanel(tab)).toEqual({ shellId: 1, open: true, maximized: false, minimized: false });
    expect(pendingClusterTerminal()).toBeNull();
  });

  it("opens what it held on the Terminals tab when that is where it was asked for", async () => {
    onPlatform(WINDOWS);
    openTab("/terminals");
    const tab = currentWorkspace().activeId;
    await openClusterTerminal(dev);
    await answerClusterTerminal(true);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "dev-cluster", namespace: undefined });
    expect(activeRoute()).toBe("/terminals");
    expect(terminalPanel(tab)).toBeUndefined();
  });

  it("opens nothing when the reader says no", async () => {
    onPlatform(WINDOWS);
    await openClusterTerminal(dev);
    await answerClusterTerminal(false);
    expect(startLocalSession).not.toHaveBeenCalled();
    expect(pendingClusterTerminal()).toBeNull();
  });

  it("opens nothing for a cluster paused while the question was up", async () => {
    onPlatform(WINDOWS);
    await openClusterTerminal(dev);
    setClusterPaused(currentWorkspace().id, dev.stableId, true);
    await answerClusterTerminal(true);
    expect(startLocalSession).not.toHaveBeenCalled();
  });

  it("on Windows, toggles a protected cluster's live shell without asking again", async () => {
    onPlatform(WINDOWS);
    const tab = onPods();
    await openClusterTerminal(dev);
    await answerClusterTerminal(true);
    sessions = [{ id: 1, context: "dev-cluster", state: "attached" }];
    // The shell was confirmed when it started; closing its panel asks nothing.
    await openClusterTerminal(dev);
    expect(terminalPanel(tab)?.open).toBe(false);
    expect(pendingClusterTerminal()).toBeNull();
    expect(startLocalSession).toHaveBeenCalledTimes(1);
  });
});
