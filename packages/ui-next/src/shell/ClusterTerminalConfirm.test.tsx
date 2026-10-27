import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";

const core = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

/** The session store is the boundary: this file is about the question, not the shell. */
const sessions = vi.hoisted(() => ({ startLocalSession: vi.fn(), getSessions: vi.fn(() => []) }));
vi.mock("../lib/sessions", () => sessions);

import { answerClusterTerminal, openClusterTerminal, pendingClusterTerminal } from "../lib/clusterTerminal";
import { resetContexts, setContexts } from "../lib/clusters";
import { defaultState } from "../lib/tabs";
import * as tabs from "../lib/tabsStore";
import { __resetTerminalPanelsForTests, terminalPanel } from "../lib/terminalPanel";
import { ClusterTerminalConfirm } from "./ClusterTerminalConfirm";

const PROD: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  key: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
let agent: { mockRestore(): void };

beforeEach(() => {
  core.isTauri.mockReturnValue(true);
  sessions.startLocalSession.mockReset().mockResolvedValue(1);
  agent = vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue(WINDOWS);
  setContexts([PROD]);
  tabs.setState(defaultState([PROD]));
  tabs.setWorkspaceProtected(tabs.currentWorkspace().id, true);
});

afterEach(async () => {
  cleanup();
  agent.mockRestore();
  await answerClusterTerminal(false);
  resetContexts();
  tabs.setState(defaultState([]));
  __resetTerminalPanelsForTests();
});

describe("ClusterTerminalConfirm", () => {
  it("draws nothing while no shell waits on it", () => {
    const { container } = render(<ClusterTerminalConfirm />);
    expect(container.innerHTML).toBe("");
  });

  it("on Windows, opens a protected cluster's shell only once its name is typed", async () => {
    const user = userEvent.setup();
    render(<ClusterTerminalConfirm />);
    await act(() => openClusterTerminal(PROD));

    expect(
      screen.getByText("On Windows this terminal can't be made read-only. kubectl in it can change prod-eu."),
    ).toBeTruthy();
    expect(screen.getByText("prod-eu is protected (workspace Default).")).toBeTruthy();
    const open = () => screen.getByRole("button", { name: "Open terminal" }) as HTMLButtonElement;
    expect(open().disabled).toBe(true);
    expect(sessions.startLocalSession).not.toHaveBeenCalled();

    await user.type(screen.getByRole("textbox", { name: "Type prod-eu to confirm" }), "prod-eu");
    expect(open().disabled).toBe(false);
    await user.click(open());
    await waitFor(() =>
      expect(sessions.startLocalSession).toHaveBeenCalledWith({ context: "prod-eu", namespace: undefined }),
    );
    // The default tab is Home, not Terminals: the shell shows under it (#942).
    await waitFor(() => expect(terminalPanel(tabs.currentWorkspace().activeId)?.open).toBe(true));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("opens nothing when the reader cancels", async () => {
    const user = userEvent.setup();
    render(<ClusterTerminalConfirm />);
    await act(() => openClusterTerminal(PROD));
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(pendingClusterTerminal()).toBeNull();
    expect(sessions.startLocalSession).not.toHaveBeenCalled();
  });
});
