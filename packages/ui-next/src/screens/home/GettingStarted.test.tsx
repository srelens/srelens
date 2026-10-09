import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage, type AgentInfo, type ClusterContext, type InstalledExtension } from "@srelens/core";
import { resetContexts, setContexts } from "../../lib/clusters";
import { takeSettingsRequest } from "../../lib/settingsRequest";
import { defaultState } from "../../lib/tabs";
import { activeRoute, setState } from "../../lib/tabsStore";
import { __setKnownVaultMode, resetLock } from "../../shell/LockGate";
import { CHECKLIST_DISMISSED_KEY, GettingStarted } from "./GettingStarted";

const core = vi.hoisted(() => ({ isTauri: vi.fn(() => true), listAgents: vi.fn() }));
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), ...core }));
const inventory = vi.hoisted(() => ({ snapshot: { status: "loading" } as Record<string, unknown> }));
vi.mock("../../extensions/inventoryStore", async (original) => ({
  ...(await original<typeof import("../../extensions/inventoryStore")>()),
  useExtensions: () => ({ ...inventory.snapshot, reload: () => {} }),
}));

const PROD: ClusterContext = {
  stableId: "prod-id", key: "prod-id", name: "prod", cluster: "prod", server: "https://prod.example", isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
};
const CLAUDE = { kind: "claude", label: "Claude", available: true, gated: false, path: "/c", version: "1", installUrl: "" } as AgentInfo;
const apps = (...plugins: unknown[]) => ({ status: "ready", data: { schemaVersion: 1, nextRevision: 1, plugins: plugins as InstalledExtension[] } });
const item = (name: string) => screen.getByText(name).closest("li") as HTMLElement;

beforeEach(() => {
  core.isTauri.mockReturnValue(true);
  core.listAgents.mockReset().mockResolvedValue([CLAUDE]);
  inventory.snapshot = apps();
  localStorage.clear();
  resetLock();
  act(() => __setKnownVaultMode("unlocked"));
  resetContexts();
  setContexts([PROD]);
  setState(defaultState([PROD]));
  takeSettingsRequest();
});

describe("GettingStarted", () => {
  it("ticks each item from what is really set up, and offers the way to the rest", async () => {
    core.listAgents.mockResolvedValue([{ ...CLAUDE, available: false }]);
    render(<GettingStarted />);
    expect(await screen.findByRole("heading", { name: /^Getting started/, level: 2 })).toBeTruthy();
    expect(within(item("Connect a cluster")).getByText("Done")).toBeTruthy();
    expect(within(item("Protect the workspace")).getByText("Done")).toBeTruthy();
    await userEvent.click(within(item("Set up the assistant")).getByRole("button", { name: "Set up" }));
    expect(activeRoute()).toBe("/settings");
    expect(takeSettingsRequest()).toEqual({ section: "agent" });
    await userEvent.click(within(item("Install an app")).getByRole("button", { name: "Browse apps" }));
    expect(takeSettingsRequest()).toEqual({ section: "extensions", tab: "catalog" });
  });

  it("opens Connect for a reader with no cluster yet", async () => {
    setContexts([]);
    render(<GettingStarted />);
    await screen.findByText("Connect a cluster");
    await userEvent.click(within(item("Connect a cluster")).getByRole("button", { name: "Connect" }));
    expect(activeRoute()).toBe("/connect");
  });

  it("is gone once every item is done", async () => {
    inventory.snapshot = apps({ manifest: { id: "io.a", name: "A" } });
    render(<GettingStarted />);
    await act(async () => { await Promise.resolve(); });
    expect(screen.queryByRole("heading", { name: /^Getting started/ })).toBeNull();
  });

  it("stays dismissed, through the settings storage", async () => {
    const first = render(<GettingStarted />);
    await userEvent.click(await screen.findByRole("button", { name: "Dismiss getting started" }));
    expect(screen.queryByRole("heading", { name: /^Getting started/ })).toBeNull();
    expect(settingsStorage.getItem(CHECKLIST_DISMISSED_KEY)).toBe("true");
    first.unmount();
    render(<GettingStarted />);
    await act(async () => { await Promise.resolve(); });
    expect(screen.queryByRole("heading", { name: /^Getting started/ })).toBeNull();
  });

  it("does not tick the cluster step when the listing failed, and stays up for it", async () => {
    setContexts([PROD], "another kubeconfig is unreadable");
    inventory.snapshot = apps({ manifest: { id: "io.a", name: "A" } });
    render(<GettingStarted />);
    await screen.findByText("Connect a cluster");
    expect(within(item("Connect a cluster")).getByText("Could not check")).toBeTruthy();
    expect(screen.getByRole("heading", { name: /^Getting started/ })).toBeTruthy();
  });

  it("does not tick the assistant when its check failed", async () => {
    core.listAgents.mockRejectedValue(new Error("agent_list: no such command"));
    inventory.snapshot = apps({ manifest: { id: "io.a", name: "A" } });
    render(<GettingStarted />);
    await screen.findByText("Set up the assistant");
    expect(within(item("Set up the assistant")).getByText("Could not check")).toBeTruthy();
    expect(screen.getByRole("heading", { name: /^Getting started/ })).toBeTruthy();
  });

  it("draws nothing while a step is still being checked, so it never flashes up", async () => {
    core.listAgents.mockReturnValue(new Promise(() => {}));
    render(<GettingStarted />);
    await act(async () => { await Promise.resolve(); });
    expect(screen.queryByRole("heading", { name: /^Getting started/ })).toBeNull();
  });

  it("does not tick apps when the inventory could not be read", async () => {
    inventory.snapshot = { status: "error", error: "connection refused" };
    render(<GettingStarted />);
    await screen.findByText("Install an app");
    expect(within(item("Install an app")).getByText("Could not check")).toBeTruthy();
  });

  it("leaves out the vault and the assistant on the web host", async () => {
    core.isTauri.mockReturnValue(false);
    render(<GettingStarted />);
    expect(await screen.findByText("Install an app")).toBeTruthy();
    expect(screen.queryByText("Protect the workspace")).toBeNull();
    expect(screen.queryByText("Set up the assistant")).toBeNull();
    expect(core.listAgents).not.toHaveBeenCalled();
  });
});
