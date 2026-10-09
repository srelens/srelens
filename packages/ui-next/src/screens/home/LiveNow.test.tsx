import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { browsable, forwardAddress, type ActiveForward, type ClusterContext } from "@srelens/core";
import { resetContexts, setContexts } from "../../lib/clusters";
import { markLogStream } from "../../lib/liveLogStreams";
import { loadMarks } from "../../lib/marks";
import type { TerminalSessionRow } from "../../lib/sessions";
import { defaultState } from "../../lib/tabs";
import { activeCluster, activeRoute, currentWorkspace, openTab, setState, togglePin } from "../../lib/tabsStore";
import { logsRoute } from "../Logs";
import { LiveNow } from "./LiveNow";

const core = vi.hoisted(() => ({
  forwards: [] as ActiveForward[],
  stopPortForward: vi.fn(),
  openExternal: vi.fn(),
}));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  getForwards: () => core.forwards,
  subscribeForwards: () => () => {},
  stopPortForward: core.stopPortForward,
  openExternal: core.openExternal,
}));
const shells = vi.hoisted(() => ({ sessions: [] as TerminalSessionRow[], endSession: vi.fn() }));
vi.mock("../../lib/sessions", () => ({
  getSessions: () => shells.sessions,
  subscribeSessions: () => () => {},
  endSession: shells.endSession,
}));

const ctx = (stableId: string, name: string): ClusterContext => ({
  stableId, key: stableId, name, cluster: name, server: `https://${stableId}.example`, isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
});
const PROD = ctx("prod-id", "prod");
const STAGE = ctx("stage-id", "staging");

const forward = (over: Partial<ActiveForward>): ActiveForward => ({
  id: 1, context: "staging", namespace: "checkout", kind: "Service", name: "web", remotePort: 80, localPort: 18080,
  status: "active", bytesMoved: 0, startedAt: 0, ...over,
} as ActiveForward);
const shell = (over: Partial<TerminalSessionRow>): TerminalSessionRow => ({
  id: 7, kind: "pod", title: "web-7d4b · app", context: "staging", namespace: "checkout", state: "attached", startedAt: 0, lastOutputAt: 0, ...over,
});

beforeEach(() => {
  core.forwards = [];
  shells.sessions = [];
  core.stopPortForward.mockReset().mockResolvedValue(undefined);
  core.openExternal.mockReset().mockResolvedValue(undefined);
  shells.endSession.mockReset();
  localStorage.clear(); loadMarks(); resetContexts();
  setContexts([PROD, STAGE]);
  setState(defaultState([PROD, STAGE]));
});

describe("LiveNow", () => {
  it("is not drawn when nothing is live", () => {
    core.forwards = [forward({ status: "failed" })];
    shells.sessions = [shell({ state: "closed" })];
    const { container } = render(<LiveNow />);
    expect(container.innerHTML).toBe("");
  });

  it("opens, stops and jumps to a running port-forward on its cluster", async () => {
    core.forwards = [forward({})];
    render(<LiveNow />);
    expect(screen.getByRole("heading", { name: "Live now", level: 2 })).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Open svc/web in the browser" }));
    // Wherever this host says the tunnel is reachable from — localhost on the desktop, the /pf/ proxy on the web.
    expect(core.openExternal).toHaveBeenCalledWith(browsable(forwardAddress({ id: 1, localPort: 18080 })));
    await userEvent.click(screen.getByRole("button", { name: "Stop forward svc/web" }));
    expect(core.stopPortForward).toHaveBeenCalledWith(1);
    await userEvent.click(screen.getByRole("button", { name: "Go to forward svc/web on staging" }));
    expect(activeCluster()).toBe(STAGE.stableId);
    expect(activeRoute()).toBe("/forwards");
  });

  it("opens a desktop forward's bare localhost address as a URL", async () => {
    // The desktop's address is `localhost:<port>` — an authority, not a URL.
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    try {
      core.forwards = [forward({})];
      render(<LiveNow />);
      await userEvent.click(screen.getByRole("button", { name: "Open svc/web in the browser" }));
      expect(core.openExternal).toHaveBeenCalledWith("http://localhost:18080");
    } finally {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    }
  });

  it("says a stop that failed, rather than leaving the row looking stopped", async () => {
    core.forwards = [forward({})];
    core.stopPortForward.mockRejectedValue(new Error("connection refused"));
    render(<LiveNow />);
    await userEvent.click(screen.getByRole("button", { name: "Stop forward svc/web" }));
    expect(await screen.findByText("Could not stop svc/web")).toBeTruthy();
  });

  it("ends and jumps to an open shell", async () => {
    shells.sessions = [shell({})];
    render(<LiveNow />);
    await userEvent.click(screen.getByRole("button", { name: "End shell web-7d4b · app" }));
    expect(shells.endSession).toHaveBeenCalledWith(7);
    await userEvent.click(screen.getByRole("button", { name: "Go to shell web-7d4b · app on staging" }));
    expect(activeCluster()).toBe(STAGE.stableId);
    expect(activeRoute()).toBe("/terminals");
  });

  it("does not count an open logs tab as live until its stream is running", () => {
    openTab(logsRoute("Deployment", "checkout", "web"), { clusterName: "prod" });
    openTab("/");
    const { container } = render(<LiveNow />);
    expect(container.innerHTML).toBe("");
  });

  it("stops a pinned logs tab too, rather than leaving it streaming", async () => {
    openTab(logsRoute("Deployment", "checkout", "web"), { clusterName: "prod" });
    const logsTab = currentWorkspace().activeId;
    togglePin(logsTab);
    markLogStream(logsTab, true);
    openTab("/");
    render(<LiveNow />);
    await userEvent.click(screen.getByRole("button", { name: "Stop web · logs" }));
    expect(currentWorkspace().tabs.map((t) => t.route)).toEqual(["/"]);
  });

  it("jumps to and stops a logs tab that is following a stream", async () => {
    openTab(logsRoute("Deployment", "checkout", "web"), { clusterName: "prod" });
    markLogStream(currentWorkspace().activeId, true);
    openTab("/");
    render(<LiveNow />);
    await userEvent.click(screen.getByRole("button", { name: "Go to web · logs" }));
    expect(activeRoute()).toBe(logsRoute("Deployment", "checkout", "web"));
    await userEvent.click(screen.getByRole("button", { name: "Stop web · logs" }));
    expect(currentWorkspace().tabs.map((t) => t.route)).toEqual(["/"]);
  });
});
