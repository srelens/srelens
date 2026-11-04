import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
vi.mock("../transport/transport", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
}));

import { mcpConnectedClients } from "./mcp";

describe("mcpConnectedClients", () => {
  beforeEach(() => invokeCommand.mockReset());
  afterEach(() => invokeCommand.mockReset());

  it("returns the privacy-limited active-session snapshot from the desktop host", async () => {
    const clients = [
      {
        name: "Claude Code",
        version: "1.2.3",
        connectedAt: 1_800_000_000,
        lastSeen: 1_800_000_003,
        notificationsConnected: true,
      },
    ];
    invokeCommand.mockResolvedValue(clients);

    await expect(mcpConnectedClients()).resolves.toEqual(clients);
    expect(invokeCommand).toHaveBeenCalledWith("mcp_connected_clients");
  });

  it("propagates a host failure instead of claiming no clients are connected", async () => {
    invokeCommand.mockRejectedValueOnce(new Error("desktop host unavailable"));
    await expect(mcpConnectedClients()).rejects.toThrow("desktop host unavailable");
  });
});
