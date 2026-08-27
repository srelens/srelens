import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import {
  auditTail,
  getMcpTokenStorage,
  promptIssues,
  respondToConfirm,
  rotateMcpToken,
  vaultLock,
} from "./mcpSecurity";

describe("mcpSecurity", () => {
  beforeEach(() => invoke.mockReset());
  // A persistent `mockRejectedValue` (as opposed to the `Once` variant) leaves
  // the mock's default implementation rejecting after the test body returns;
  // some later call into the mock during test-file teardown then surfaces as
  // an unhandled rejection attributed to whichever test last ran. Reset after
  // each test too so a "reject forever" mock never outlives its test.
  afterEach(() => invoke.mockReset());

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

  it("reports which backend holds the token", async () => {
    invoke.mockResolvedValue("file");
    await expect(getMcpTokenStorage()).resolves.toBe("file");
    expect(invoke).toHaveBeenCalledWith("mcp_token_storage");
  });

  it("promptIssues returns [] rather than throwing when the command fails", async () => {
    invoke.mockRejectedValue(new Error("nope"));
    await expect(promptIssues()).resolves.toEqual([]);
  });

  it("locks the vault by name, with an empty payload", async () => {
    invoke.mockResolvedValue(undefined);
    await vaultLock();
    expect(invoke).toHaveBeenCalledWith("vault_lock", {});
  });

  it("surfaces a refused lock instead of resolving quietly", async () => {
    // The reader must be told the workspace is still open — a swallowed
    // failure would leave the Settings pane claiming a lock that never
    // happened.
    invoke.mockRejectedValueOnce(new Error("no master password is set"));
    await expect(vaultLock()).rejects.toThrow("no master password");
  });
});
