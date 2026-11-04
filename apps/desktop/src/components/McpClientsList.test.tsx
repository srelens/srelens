import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mcpHttpSnapshot } = vi.hoisted(() => ({ mcpHttpSnapshot: vi.fn() }));
vi.mock("@srelens/core/lib/mcp", () => ({
  mcpHttpSnapshot: (...args: unknown[]) => mcpHttpSnapshot(...args),
}));

import { McpClientsList } from "./McpClientsList";

const URL = "http://127.0.0.1:8765/mcp";
const stopped = { running: false, url: null, clients: [] };
const running = (clients: unknown[] = []) => ({ running: true, url: URL, clients });

describe("McpClientsList", () => {
  beforeEach(() => {
    mcpHttpSnapshot.mockReset().mockResolvedValue(running());
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2027-01-15T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("distinguishes a stopped server from a running server with no sessions", async () => {
    mcpHttpSnapshot.mockResolvedValue(stopped);
    const { rerender } = render(<McpClientsList running={false} />);
    expect(await screen.findByText(/server is stopped/i)).toBeTruthy();
    expect(mcpHttpSnapshot).toHaveBeenCalledTimes(1);

    mcpHttpSnapshot.mockResolvedValue(running());
    rerender(<McpClientsList running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
  });

  it("keeps an unresolved initial snapshot as loading, not stopped", async () => {
    mcpHttpSnapshot.mockImplementation(() => new Promise(() => {}));
    render(<McpClientsList running={null} />);

    expect(screen.getByRole("status", { name: /checking MCP HTTP server/i })).toBeTruthy();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
    expect(mcpHttpSnapshot).toHaveBeenCalledTimes(1);
  });

  it("renders self-reported identity, recency, and notification-stream state", async () => {
    const nowSeconds = Date.now() / 1000;
    mcpHttpSnapshot.mockResolvedValue(running([
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
    ]));

    render(<McpClientsList running={null} />);

    expect(await screen.findByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("1.2.3")).toBeTruthy();
    expect(screen.getByText("Cursor")).toBeTruthy();
    expect(screen.getByText("Notification stream open")).toBeTruthy();
    expect(screen.getByText("No notification stream")).toBeTruthy();
    expect(screen.getByText(/1m ago/)).toBeTruthy();
    expect(screen.getAllByText(/Last request/)[0]?.getAttribute("title")).toMatch(
      /^Session started /,
    );
    expect(screen.getByText(/self-reported/i)).toBeTruthy();
    expect(screen.getByText(/no client network addresses or credentials/i)).toBeTruthy();
    expect(screen.getByText(/not proof that the client process is still running/i)).toBeTruthy();
  });

  it("shows a read failure separately and retries on request", async () => {
    mcpHttpSnapshot.mockRejectedValueOnce(new Error("snapshot unavailable"));
    render(<McpClientsList running={null} />);

    expect((await screen.findByRole("alert")).textContent).toMatch(/could not be read/i);
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
    expect(screen.queryByText(/no active HTTP client sessions/i)).toBeNull();

    mcpHttpSnapshot.mockResolvedValue(running([
      {
        name: "Codex",
        version: "0.9",
        connectedAt: Date.now() / 1000,
        lastSeen: Date.now() / 1000,
        notificationsConnected: false,
      },
    ]));
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(await screen.findByText("Codex")).toBeTruthy();
  });

  it("refreshes while Settings remains open", async () => {
    render(<McpClientsList running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();
    expect(mcpHttpSnapshot).toHaveBeenCalledTimes(1);

    mcpHttpSnapshot.mockResolvedValue(running([
      {
        name: "Cursor",
        version: "2",
        connectedAt: Date.now() / 1000,
        lastSeen: Date.now() / 1000,
        notificationsConnected: false,
      },
    ]));
    await vi.advanceTimersByTimeAsync(5_000);
    await waitFor(() => expect(screen.getByText("Cursor")).toBeTruthy());
    expect(mcpHttpSnapshot).toHaveBeenCalledTimes(2);
  });

  it("transitions to stopped when a later poll observes that the server ended", async () => {
    render(<McpClientsList running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();

    mcpHttpSnapshot.mockResolvedValue(stopped);
    await vi.advanceTimersByTimeAsync(5_000);

    expect(await screen.findByText(/server is stopped/i)).toBeTruthy();
    expect(screen.queryByText(/no active HTTP client sessions/i)).toBeNull();
  });

  it("does not let an older slow running snapshot overwrite a newer stopped one", async () => {
    let resolveFirst!: (snapshot: unknown) => void;
    mcpHttpSnapshot
      .mockImplementationOnce(() => new Promise((resolve) => (resolveFirst = resolve)))
      .mockResolvedValueOnce(stopped);

    render(<McpClientsList running />);
    await waitFor(() => expect(mcpHttpSnapshot).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(await screen.findByText(/server is stopped/i)).toBeTruthy();

    await act(async () => {
      resolveFirst(running([
        {
          name: "Stale client",
          version: null,
          connectedAt: Date.now() / 1000,
          lastSeen: Date.now() / 1000,
          notificationsConnected: false,
        },
      ]));
      await Promise.resolve();
    });

    expect(screen.getByText(/server is stopped/i)).toBeTruthy();
    expect(screen.queryByText("Stale client")).toBeNull();
  });
});
