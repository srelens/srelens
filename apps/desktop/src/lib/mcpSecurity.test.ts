import { describe, expect, it, vi, beforeEach } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { auditTail, respondToConfirm, rotateMcpToken } from "./mcpSecurity";

describe("mcpSecurity", () => {
  beforeEach(() => invoke.mockReset());

  it("passes the confirm decision through to the command", async () => {
    invoke.mockResolvedValue(undefined);
    await respondToConfirm("abc", true);
    expect(invoke).toHaveBeenCalledWith("mcp_confirm_respond", { id: "abc", approved: true });
  });

  it("returns the rotated token", async () => {
    invoke.mockResolvedValue("f".repeat(64));
    await expect(rotateMcpToken()).resolves.toHaveLength(64);
  });

  it("returns an empty list when the audit log is unreadable", async () => {
    invoke.mockRejectedValueOnce(new Error("nope"));
    await expect(auditTail(10)).resolves.toEqual([]);
  });
});
