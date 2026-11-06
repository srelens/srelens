import { afterEach, describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import React from "react";

const { mcp } = vi.hoisted(() => ({
  mcp: {
    startMcpHttp: vi.fn(),
    stopMcpHttp: vi.fn(),
    mcpHttpSnapshot: vi.fn(),
    installSrelensCli: vi.fn(),
    srelensCliStatus: vi.fn(),
  },
}));
vi.mock("@srelens/core/lib/mcp", () => mcp);
vi.mock("@srelens/core/lib/notify", () => ({ notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

const { mcpSecurity } = vi.hoisted(() => ({
  mcpSecurity: {
    getMcpToken: vi.fn(),
    getMcpTokenStorage: vi.fn(),
    rotateMcpToken: vi.fn(),
    revokeMcpToken: vi.fn(),
    auditTail: vi.fn(),
    promptIssues: vi.fn(),
    vaultBiometricStatus: vi.fn(),
    vaultBiometricEnable: vi.fn(),
    vaultBiometricDisable: vi.fn(),
    vaultBiometricUnlock: vi.fn(),
  },
}));
vi.mock("@srelens/core/lib/mcpSecurity", () => mcpSecurity);

import { McpSettingsSection } from "./McpSettingsSection";

const PLATFORM = navigator.platform;
afterEach(() => Object.defineProperty(navigator, "platform", { configurable: true, value: PLATFORM }));

beforeEach(() => {
  localStorage.clear();
  Object.values(mcp).forEach((m) => m.mockReset());
  mcp.mcpHttpSnapshot.mockResolvedValue({ running: false, url: null, clients: [] });
  mcp.srelensCliStatus.mockResolvedValue({
    installed: false,
    path: "/home/u/.local/bin/srelens",
    links_to: null,
    on_path: true,
  });
  Object.values(mcpSecurity).forEach((m) => m.mockReset());
  mcpSecurity.getMcpToken.mockResolvedValue(null);
  mcpSecurity.getMcpTokenStorage.mockResolvedValue("keychain");
  mcpSecurity.auditTail.mockResolvedValue([]);
  mcpSecurity.promptIssues.mockResolvedValue([]);
  // Default: no biometric sensor — the Touch ID control stays hidden.
  mcpSecurity.vaultBiometricStatus.mockResolvedValue({ available: false, enabled: false, unlocked: true });
  mcpSecurity.vaultBiometricEnable.mockResolvedValue(undefined);
  mcpSecurity.vaultBiometricDisable.mockResolvedValue(undefined);
  mcpSecurity.vaultBiometricUnlock.mockResolvedValue(undefined);
});

describe("McpSettingsSection", () => {
  // The trail carries this app's own changes as well as an agent's since
  // #555, so a heading naming only agents would have a reader attribute
  // their own Sync to something else.
  it("names the activity section for capabilities, not for agents", async () => {
    render(<McpSettingsSection />);
    expect(await screen.findByRole("heading", { name: "Recent capability activity" })).toBeDefined();
    expect(screen.queryByText("Recent agent activity")).toBeNull();
  });

  it("starts the MCP HTTP server when toggled on and shows the URL", async () => {
    mcp.startMcpHttp.mockImplementation(async () => {
      mcp.mcpHttpSnapshot.mockResolvedValue({
        running: true,
        url: "http://127.0.0.1:8765/mcp",
        clients: [],
      });
      return "http://127.0.0.1:8765/mcp";
    });
    render(<McpSettingsSection />);
    fireEvent.click(screen.getByLabelText("Run MCP HTTP server"));
    await waitFor(() => expect(mcp.startMcpHttp).toHaveBeenCalledWith(8765));
    expect(await screen.findByText("http://127.0.0.1:8765/mcp")).toBeDefined();
  });

  it("does not let a snapshot started before Start undo the completed action", async () => {
    const url = "http://127.0.0.1:8765/mcp";
    let answerSnapshot!: (snapshot: { running: false; url: null; clients: never[] }) => void;
    let answerStart!: (url: string) => void;
    mcp.mcpHttpSnapshot
      .mockResolvedValue({ running: true, url, clients: [] })
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            answerSnapshot = resolve;
          }),
      );
    mcp.startMcpHttp.mockImplementation(
      () =>
        new Promise((resolve) => {
          answerStart = resolve;
        }),
    );

    render(<McpSettingsSection />);
    await waitFor(() => expect(mcp.mcpHttpSnapshot).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByLabelText("Run MCP HTTP server"));
    await waitFor(() => expect(mcp.startMcpHttp).toHaveBeenCalledWith(8765));

    await act(async () => {
      answerStart(url);
      answerSnapshot({ running: false, url: null, clients: [] });
      await Promise.resolve();
    });

    expect(await screen.findByText("Listening at")).toBeDefined();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
  });

  it("updates the whole server status when the atomic session snapshot observes a stop", async () => {
    mcp.mcpHttpSnapshot.mockResolvedValue({
      running: true,
      url: "http://127.0.0.1:8765/mcp",
      clients: [],
    });
    render(<McpSettingsSection />);

    expect(await screen.findByRole("heading", { name: "HTTP client sessions" })).toBeDefined();
    expect(await screen.findByText("Listening at")).toBeDefined();

    mcp.mcpHttpSnapshot.mockResolvedValue({ running: false, url: null, clients: [] });
    fireEvent.click(screen.getByRole("button", { name: "Refresh HTTP client sessions" }));

    expect(await screen.findByText(/server is stopped/i)).toBeDefined();
    expect(screen.queryByText("Listening at")).toBeNull();
  });

  it("surfaces a bind error and leaves the toggle off", async () => {
    mcp.startMcpHttp.mockRejectedValue("Could not bind 127.0.0.1:8765: address in use");
    render(<McpSettingsSection />);
    fireEvent.click(screen.getByLabelText("Run MCP HTTP server"));
    expect(await screen.findByText(/address in use/)).toBeDefined();
    expect((screen.getByLabelText("Run MCP HTTP server") as HTMLInputElement).checked).toBe(false);
  });

  it("installs the srelens CLI", async () => {
    mcp.installSrelensCli.mockResolvedValue("/home/u/.local/bin/srelens");
    mcp.srelensCliStatus.mockResolvedValue({
      installed: true,
      path: "/home/u/.local/bin/srelens",
      links_to: "/x",
      on_path: true,
    });
    render(<McpSettingsSection />);
    const install = screen.getByRole("button", { name: /Install srelens CLI/ });
    await waitFor(() => expect((install as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(install);
    await waitFor(() => expect(mcp.installSrelensCli).toHaveBeenCalled());
    // After install the button relabels to Reinstall and the path is shown.
    expect(await screen.findByRole("button", { name: /Reinstall srelens CLI/ })).toBeDefined();
    expect(screen.getAllByText(/Installed at/).length).toBeGreaterThanOrEqual(1);
  });

  it("warns when the install directory is not on PATH", async () => {
    mcp.srelensCliStatus.mockResolvedValue({
      installed: true,
      path: "/home/u/.local/bin/srelens",
      links_to: "/x",
      on_path: false,
    });
    render(<McpSettingsSection />);
    expect(await screen.findByText(/isn't on your PATH/)).toBeDefined();
  });

  it("uses the absolute executable on Windows and hides unsupported installation", async () => {
    Object.defineProperty(navigator, "platform", { configurable: true, value: "Win32" });
    const path = String.raw`C:\Program Files\srelens\srelens.exe`;
    mcp.srelensCliStatus.mockResolvedValue({ installed: true, path, links_to: null, on_path: false });
    render(<McpSettingsSection />);

    expect(await screen.findByText(/Run the PowerShell command in PowerShell/)).toBeDefined();
    expect(screen.getAllByText(new RegExp(path.replace(/[\\]/g, "\\\\")))).not.toHaveLength(0);
    expect(screen.queryByRole("button", { name: /Install srelens CLI/ })).toBeNull();
  });

  it("shows a CLI status failure and allows retry instead of offering installation", async () => {
    mcp.srelensCliStatus.mockRejectedValueOnce(new Error("status unavailable"));
    render(<McpSettingsSection />);
    expect((await screen.findByRole("alert")).textContent).toMatch(/status unavailable/i);
    expect(screen.queryByRole("button", { name: "Install srelens CLI" })).toBeNull();

    mcp.srelensCliStatus.mockResolvedValue({ installed: false, path: "/home/u/.local/bin/srelens", links_to: null, on_path: false });
    fireEvent.click(screen.getByRole("button", { name: "Retry CLI status" }));
    expect(await screen.findByRole("button", { name: "Install srelens CLI" })).toBeDefined();
  });

  it("shows the client config for the selected tool and transport", async () => {
    render(<McpSettingsSection />);
    // Default tool is Claude Code (stdio) → a `claude mcp add` command.
    expect(screen.getByText("claude mcp add srelens -- srelens --mcp-stdio")).toBeDefined();
    // Switch to Codex → TOML block.
    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    expect(screen.getByText(/\[mcp_servers\.srelens\]/)).toBeDefined();
  });

  it("masks the token until revealed and rotates on request", async () => {
    const token = `${"a".repeat(60)}wxyz`;
    mcpSecurity.getMcpToken.mockResolvedValue(token);
    mcpSecurity.rotateMcpToken.mockResolvedValue(`${"b".repeat(60)}1234`);
    render(<McpSettingsSection />);

    // The masked form (last 4 chars only) is visible once the token loads,
    // but the raw 64-char value must not be anywhere in the DOM yet.
    await screen.findByText(/wxyz/);
    expect(screen.queryByText(token)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Reveal token" }));
    expect(await screen.findByText(token)).toBeDefined();

    // Rotating warns inline before it commits, then calls rotateMcpToken on confirm.
    fireEvent.click(screen.getByRole("button", { name: "Rotate token" }));
    expect(screen.getByText(/need the new value/i)).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Rotate" }));
    await waitFor(() => expect(mcpSecurity.rotateMcpToken).toHaveBeenCalled());

    // The freshly rotated token is masked again until explicitly revealed.
    expect(screen.queryByText(token)).toBeNull();
  });


});
