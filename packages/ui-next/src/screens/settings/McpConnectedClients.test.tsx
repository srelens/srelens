import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({ mcpConnectedClients: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  mcpConnectedClients: (...args: unknown[]) => core.mcpConnectedClients(...args),
}));

import { McpConnectedClients } from "./McpConnectedClients";

const client = (name: string) => ({
  name,
  version: null,
  connectedAt: Date.now() / 1000 - 30,
  lastSeen: Date.now() / 1000 - 10,
  notificationsConnected: false,
});

describe("McpConnectedClients", () => {
  beforeEach(() => {
    core.mcpConnectedClients.mockReset().mockResolvedValue([]);
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2027-01-15T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("distinguishes a stopped server from a status that is not known yet", () => {
    const { rerender } = render(<McpConnectedClients running={null} />);
    expect(screen.getByText(/once the server status is known/i)).toBeTruthy();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
    expect(core.mcpConnectedClients).not.toHaveBeenCalled();

    rerender(<McpConnectedClients running={false} />);
    expect(screen.getByText(/server is stopped/i)).toBeTruthy();
    expect(core.mcpConnectedClients).not.toHaveBeenCalled();
  });

  it("distinguishes a running server with no sessions", async () => {
    render(<McpConnectedClients running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();
    expect(core.mcpConnectedClients).toHaveBeenCalledTimes(1);
  });

  it("renders identity, recency, stream state, and the privacy caveat", async () => {
    core.mcpConnectedClients.mockResolvedValue([
      {
        name: "Claude Code",
        version: "1.2.3",
        connectedAt: Date.now() / 1000 - 120,
        lastSeen: Date.now() / 1000 - 60,
        notificationsConnected: true,
      },
      client("Cursor"),
    ]);

    render(<McpConnectedClients running />);

    expect(await screen.findByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("1.2.3")).toBeTruthy();
    expect(screen.getByText("Cursor")).toBeTruthy();
    expect(screen.getByText("Notification stream open")).toBeTruthy();
    expect(screen.getByText("No notification stream")).toBeTruthy();
    expect(screen.getByText(/last request 1m ago/i)).toBeTruthy();
    expect(screen.getByText(/self-reported/i)).toBeTruthy();
    expect(screen.getByText(/no network addresses, credentials, or session IDs/i)).toBeTruthy();
    expect(screen.getByText(/not proof that the client process is still running/i)).toBeTruthy();
  });

  it("keeps a read failure distinct from an empty result and retries", async () => {
    core.mcpConnectedClients.mockRejectedValueOnce(new Error("snapshot unavailable"));
    render(<McpConnectedClients running />);

    expect(await screen.findByText(/active MCP clients could not be read/i)).toBeTruthy();
    expect(screen.queryByText(/no active HTTP client sessions/i)).toBeNull();

    core.mcpConnectedClients.mockResolvedValue([client("Codex")]);
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(await screen.findByText("Codex")).toBeTruthy();
  });

  it("polls and does not let an older slow response overwrite a newer one", async () => {
    let resolveFirst!: (clients: unknown[]) => void;
    core.mcpConnectedClients
      .mockImplementationOnce(() => new Promise((resolve) => (resolveFirst = resolve)))
      .mockResolvedValueOnce([client("Current client")]);

    render(<McpConnectedClients running />);
    await waitFor(() => expect(core.mcpConnectedClients).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(await screen.findByText("Current client")).toBeTruthy();

    await act(async () => {
      resolveFirst([client("Stale client")]);
      await Promise.resolve();
    });

    expect(screen.getByText("Current client")).toBeTruthy();
    expect(screen.queryByText("Stale client")).toBeNull();
  });
});
