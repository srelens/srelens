import { beforeEach, describe, expect, it, vi } from "vitest";

const isTauri = vi.fn(() => true);
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  isTauri: () => isTauri(),
}));

const startLocalSession = vi.fn();
vi.mock("./sessions", () => ({
  startLocalSession: (...args: unknown[]) => startLocalSession(...args),
}));

const { canOpenClusterTerminal, openClusterTerminal, terminalNamespace } = await import(
  "./clusterTerminal"
);
const { activeRoute, currentWorkspace, setClusterPaused, setState, setTabNamespaces } = await import(
  "./tabsStore"
);
const { defaultState } = await import("./tabs");

const dev = {
  name: "dev-cluster", stableId: "dev", key: "dev", cluster: "c", server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
};
const other = { ...dev, name: "other-cluster", stableId: "other", key: "other" };

beforeEach(() => {
  isTauri.mockReturnValue(true);
  startLocalSession.mockReset().mockResolvedValue(1);
  setState(defaultState([dev, other]));
});

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
  it("starts a local shell for the cluster it was given, then shows it", async () => {
    await openClusterTerminal(dev);
    expect(startLocalSession).toHaveBeenCalledWith({ context: "dev-cluster", namespace: undefined });
    expect(activeRoute()).toBe("/terminals");
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

  it("shows the terminals screen only once the session exists", async () => {
    let started!: () => void;
    startLocalSession.mockReturnValue(new Promise<number>((resolve) => (started = () => resolve(1))));
    const before = activeRoute();
    const opening = openClusterTerminal(dev);
    expect(activeRoute()).toBe(before);
    started();
    await opening;
    expect(activeRoute()).toBe("/terminals");
  });
});
