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
const actionGenerationRef = { current: 0 };
const actionProps = { actionGeneration: 0, actionGenerationRef };

describe("McpClientsList", () => {
  beforeEach(() => {
    actionGenerationRef.current = 0;
    mcpHttpSnapshot.mockReset().mockResolvedValue(running());
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2027-01-15T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("distinguishes a stopped server from a running server with no sessions", async () => {
    mcpHttpSnapshot.mockResolvedValue(stopped);
    const { rerender } = render(<McpClientsList {...actionProps} running={false} />);
    expect(await screen.findByText(/server is stopped/i)).toBeTruthy();
    expect(mcpHttpSnapshot).toHaveBeenCalledTimes(1);

    mcpHttpSnapshot.mockResolvedValue(running());
    rerender(<McpClientsList {...actionProps} running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
  });

  it("keeps an unresolved initial snapshot as loading, not stopped", async () => {
    mcpHttpSnapshot.mockImplementation(() => new Promise(() => {}));
    render(<McpClientsList {...actionProps} running={null} />);

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

    render(<McpClientsList {...actionProps} running={null} />);

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

  it("wraps a long unbroken client name inside the narrow key column", async () => {
    const longName = "client".repeat(14);
    mcpHttpSnapshot.mockResolvedValue(running([
      {
        name: longName,
        version: "1.2.3",
        connectedAt: Date.now() / 1000,
        lastSeen: Date.now() / 1000,
        notificationsConnected: false,
      },
    ]));

    render(
      <div style={{ width: 240 }}>
        <McpClientsList {...actionProps} running />
      </div>,
    );

    const name = await screen.findByText(longName);
    expect(name.tagName).toBe("DT");
    expect(name.classList.contains("break-words")).toBe(true);
    expect(name.parentElement?.classList.contains("fl-kv")).toBe(true);
  });

  it("shows a read failure separately and retries on request", async () => {
    mcpHttpSnapshot.mockRejectedValueOnce(new Error("snapshot unavailable"));
    render(<McpClientsList {...actionProps} running={null} />);

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
    render(<McpClientsList {...actionProps} running />);
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
    render(<McpClientsList {...actionProps} running />);
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

    render(<McpClientsList {...actionProps} running />);
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

  it("drops a pending snapshot as soon as a parent action invalidates its generation", async () => {
    let answerSnapshot!: (snapshot: typeof stopped) => void;
    const actionGenerationRef = { current: 0 };
    const onServerSnapshot = vi.fn();
    mcpHttpSnapshot.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answerSnapshot = resolve;
        }),
    );

    render(
      <McpClientsList
        running
        actionGeneration={0}
        actionGenerationRef={actionGenerationRef}
        onServerSnapshot={onServerSnapshot}
      />,
    );
    await waitFor(() => expect(mcpHttpSnapshot).toHaveBeenCalledTimes(1));

    // Parent actions bump the ref synchronously, before React can render the
    // numeric generation prop and run this component's effect cleanup.
    actionGenerationRef.current += 1;
    await act(async () => {
      answerSnapshot(stopped);
      await Promise.resolve();
    });

    expect(onServerSnapshot).not.toHaveBeenCalled();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
  });
});
