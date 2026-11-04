import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mcpConnectedClients } = vi.hoisted(() => ({ mcpConnectedClients: vi.fn() }));
vi.mock("@srelens/core/lib/mcp", () => ({
  mcpConnectedClients: (...args: unknown[]) => mcpConnectedClients(...args),
}));

import { McpClientsList } from "./McpClientsList";

describe("McpClientsList", () => {
  beforeEach(() => {
    mcpConnectedClients.mockReset().mockResolvedValue([]);
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2027-01-15T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("distinguishes a stopped server from a running server with no sessions", async () => {
    const { rerender } = render(<McpClientsList running={false} />);
    expect(screen.getByText(/server is stopped/i)).toBeTruthy();
    expect(mcpConnectedClients).not.toHaveBeenCalled();

    rerender(<McpClientsList running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
  });

  it("renders self-reported identity, recency, and notification-stream state", async () => {
    const nowSeconds = Date.now() / 1000;
    mcpConnectedClients.mockResolvedValue([
      {
        name: "Claude Code",
        version: "1.2.3",
        connectedAt: nowSeconds - 120,
        lastSeen: nowSeconds - 60,
        notificationsConnected: true,
      },
      {
        name: "Cursor",
        version: null,
        connectedAt: nowSeconds - 30,
        lastSeen: nowSeconds - 30,
        notificationsConnected: false,
      },
    ]);

    render(<McpClientsList running />);

    expect(await screen.findByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("1.2.3")).toBeTruthy();
    expect(screen.getByText("Cursor")).toBeTruthy();
    expect(screen.getByText("Notification stream open")).toBeTruthy();
    expect(screen.getByText("No notification stream")).toBeTruthy();
    expect(screen.getByText(/1m ago/)).toBeTruthy();
    expect(screen.getByText(/self-reported/i)).toBeTruthy();
    expect(screen.getByText(/no client network addresses or credentials/i)).toBeTruthy();
    expect(screen.getByText(/not proof that the client process is still running/i)).toBeTruthy();
  });

  it("shows a read failure separately and retries on request", async () => {
    mcpConnectedClients.mockRejectedValueOnce(new Error("snapshot unavailable"));
    render(<McpClientsList running />);

    expect((await screen.findByRole("alert")).textContent).toMatch(/could not be read/i);
    expect(screen.queryByText(/no active HTTP client sessions/i)).toBeNull();

    mcpConnectedClients.mockResolvedValue([
      {
        name: "Codex",
        version: "0.9",
        connectedAt: Date.now() / 1000,
        lastSeen: Date.now() / 1000,
        notificationsConnected: false,
      },
    ]);
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(await screen.findByText("Codex")).toBeTruthy();
  });

  it("refreshes while Settings remains open", async () => {
    render(<McpClientsList running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();
    expect(mcpConnectedClients).toHaveBeenCalledTimes(1);

    mcpConnectedClients.mockResolvedValue([
      {
        name: "Cursor",
        version: "2",
        connectedAt: Date.now() / 1000,
        lastSeen: Date.now() / 1000,
        notificationsConnected: false,
      },
    ]);
    await vi.advanceTimersByTimeAsync(5_000);
    await waitFor(() => expect(screen.getByText("Cursor")).toBeTruthy());
    expect(mcpConnectedClients).toHaveBeenCalledTimes(2);
  });

  it("does not let an older slow snapshot overwrite a newer refresh", async () => {
    let resolveFirst!: (clients: unknown[]) => void;
    mcpConnectedClients
      .mockImplementationOnce(() => new Promise((resolve) => (resolveFirst = resolve)))
      .mockResolvedValueOnce([
        {
          name: "Current client",
          version: null,
          connectedAt: Date.now() / 1000,
          lastSeen: Date.now() / 1000,
          notificationsConnected: false,
        },
      ]);

    render(<McpClientsList running />);
    await waitFor(() => expect(mcpConnectedClients).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(await screen.findByText("Current client")).toBeTruthy();

    await act(async () => {
      resolveFirst([
        {
          name: "Stale client",
          version: null,
          connectedAt: Date.now() / 1000,
          lastSeen: Date.now() / 1000,
          notificationsConnected: false,
        },
      ]);
      await Promise.resolve();
    });

    expect(screen.getByText("Current client")).toBeTruthy();
    expect(screen.queryByText("Stale client")).toBeNull();
  });
});
