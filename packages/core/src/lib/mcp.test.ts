import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
vi.mock("../transport/transport", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
}));

import { mcpHttpSnapshot } from "./mcp";

describe("mcpHttpSnapshot", () => {
  beforeEach(() => invokeCommand.mockReset());
  afterEach(() => invokeCommand.mockReset());

  it("returns the privacy-limited active-session snapshot from the desktop host", async () => {
    const snapshot = {
      running: true,
      url: "http://127.0.0.1:8765/mcp",
      clients: [
        {
          name: "Claude Code",
          version: "1.2.3",
          connectedAt: 1_800_000_000,
          lastSeen: 1_800_000_003,
          notificationsConnected: true,
        },
      ],
    };
    invokeCommand.mockResolvedValue(snapshot);

    await expect(mcpHttpSnapshot()).resolves.toEqual(snapshot);
    expect(invokeCommand).toHaveBeenCalledWith("mcp_http_snapshot");
  });

  it("propagates a host failure instead of claiming no clients are connected", async () => {
    invokeCommand.mockRejectedValueOnce(new Error("desktop host unavailable"));
    await expect(mcpHttpSnapshot()).rejects.toThrow("desktop host unavailable");
  });
});
