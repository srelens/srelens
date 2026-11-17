import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({ mcpHttpSnapshot: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  mcpHttpSnapshot: (...args: unknown[]) => core.mcpHttpSnapshot(...args),
}));

import { McpConnectedClients } from "./McpConnectedClients";

const client = (name: string) => ({
  name,
  version: null,
  connectedAt: Date.now() / 1000 - 30,
  lastSeen: Date.now() / 1000 - 10,
  notificationsConnected: false,
});
const URL = "http://127.0.0.1:8765/mcp";
const stopped = { running: false, url: null, clients: [] };
const running = (clients: unknown[] = []) => ({ running: true, url: URL, clients });
const actionGenerationRef = { current: 0 };
const actionProps = { actionGeneration: 0, actionGenerationRef };

describe("McpConnectedClients", () => {
  beforeEach(() => {
    actionGenerationRef.current = 0;
    core.mcpHttpSnapshot.mockReset().mockResolvedValue(running());
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2027-01-15T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("distinguishes a stopped server from a status that is not known yet", async () => {
    core.mcpHttpSnapshot.mockImplementation(() => new Promise(() => {}));
    const { rerender } = render(<McpConnectedClients {...actionProps} running={null} />);
    expect(screen.getByText(/once the server status is known/i)).toBeTruthy();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
    expect(core.mcpHttpSnapshot).toHaveBeenCalledTimes(1);

    core.mcpHttpSnapshot.mockResolvedValue(stopped);
    rerender(<McpConnectedClients {...actionProps} running={false} />);
    expect(await screen.findByText(/server is stopped/i)).toBeTruthy();
    expect(core.mcpHttpSnapshot).toHaveBeenCalledTimes(2);
  });

  it("distinguishes a running server with no sessions", async () => {
    render(<McpConnectedClients {...actionProps} running />);
    // The kit's empty state: its title, with no full stop (#1001, step 7a).
    expect(await screen.findByText("No active HTTP client sessions")).toBeTruthy();
    expect(core.mcpHttpSnapshot).toHaveBeenCalledTimes(1);
  });

  it("renders identity, recency, stream state, and the privacy caveat", async () => {
    core.mcpHttpSnapshot.mockResolvedValue(running([
      {
        name: "Claude Code",
        version: "1.2.3",
        connectedAt: Date.now() / 1000 - 120,
        lastSeen: Date.now() / 1000 - 60,
        notificationsConnected: true,
      },
      client("Cursor"),
    ]));

    render(<McpConnectedClients {...actionProps} running />);

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
    core.mcpHttpSnapshot.mockRejectedValueOnce(new Error("snapshot unavailable"));
    render(<McpConnectedClients {...actionProps} running />);

    expect(await screen.findByText(/HTTP client sessions could not be read/i)).toBeTruthy();
    expect(screen.queryByText(/no active HTTP client sessions/i)).toBeNull();

    core.mcpHttpSnapshot.mockResolvedValue(running([client("Codex")]));
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(await screen.findByText("Codex")).toBeTruthy();
  });

  it("transitions to stopped when a later poll observes that the server ended", async () => {
    render(<McpConnectedClients {...actionProps} running />);
    expect(await screen.findByText(/no active HTTP client sessions/i)).toBeTruthy();

    core.mcpHttpSnapshot.mockResolvedValue(stopped);
    await vi.advanceTimersByTimeAsync(5_000);

    expect(await screen.findByText(/server is stopped/i)).toBeTruthy();
    expect(screen.queryByText(/no active HTTP client sessions/i)).toBeNull();
  });

  it("does not let an older slow running snapshot overwrite a newer stopped one", async () => {
    let resolveFirst!: (snapshot: unknown) => void;
    core.mcpHttpSnapshot
      .mockImplementationOnce(() => new Promise((resolve) => (resolveFirst = resolve)))
      .mockResolvedValueOnce(stopped);

    render(<McpConnectedClients {...actionProps} running />);
    await waitFor(() => expect(core.mcpHttpSnapshot).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(await screen.findByText(/server is stopped/i)).toBeTruthy();

    await act(async () => {
      resolveFirst(running([client("Stale client")]));
      await Promise.resolve();
    });

    expect(screen.getByText(/server is stopped/i)).toBeTruthy();
    expect(screen.queryByText("Stale client")).toBeNull();
  });

  it("drops a pending snapshot as soon as a parent action invalidates its generation", async () => {
    let answerSnapshot!: (snapshot: typeof stopped) => void;
    const actionGenerationRef = { current: 0 };
    const onServerSnapshot = vi.fn();
    core.mcpHttpSnapshot.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answerSnapshot = resolve;
        }),
    );

    render(
      <McpConnectedClients
        running
        actionGeneration={0}
        actionGenerationRef={actionGenerationRef}
        onServerSnapshot={onServerSnapshot}
      />,
    );
    await waitFor(() => expect(core.mcpHttpSnapshot).toHaveBeenCalledTimes(1));

    actionGenerationRef.current += 1;
    await act(async () => {
      answerSnapshot(stopped);
      await Promise.resolve();
    });

    expect(onServerSnapshot).not.toHaveBeenCalled();
    expect(screen.queryByText(/server is stopped/i)).toBeNull();
  });
});
