// Typed wrappers for the MCP consent/token/audit commands.
import { invoke } from "@tauri-apps/api/core";

export interface ConfirmRequest {
  id: string;
  tool: string;
  args: Record<string, unknown>;
}

export interface AuditEntry {
  ts: number;
  transport: "stdio" | "http";
  tool: string;
  args: Record<string, unknown>;
  decision: "approved" | "denied" | "auto";
  outcome: "ok" | "error";
  err: string | null;
}

export async function respondToConfirm(id: string, approved: boolean): Promise<void> {
  await invoke("mcp_confirm_respond", { id, approved });
}

export async function getMcpToken(): Promise<string | null> {
  return (await invoke<string | null>("mcp_token_get")) ?? null;
}

export async function rotateMcpToken(): Promise<string> {
  return await invoke<string>("mcp_token_rotate");
}

export async function revokeMcpToken(): Promise<void> {
  await invoke("mcp_token_revoke");
}

/** Where the secrets vault's MASTER KEY lives: the OS keychain; a 0600
 * fallback file (Settings says plainly the vault is then effectively
 * unprotected); or "locked" — the keychain holding the key was unreachable
 * this launch while a vault exists, so secrets are frozen rather than
 * destroyed by a re-key. */
export async function getMcpTokenStorage(): Promise<"keychain" | "file" | "locked"> {
  return await invoke<"keychain" | "file" | "locked">("mcp_token_storage");
}

/** Newest first. Returns [] rather than throwing: a missing log is not an error. */
export async function auditTail(limit: number): Promise<AuditEntry[]> {
  try {
    return await invoke<AuditEntry[]>("mcp_audit_tail", { limit });
  } catch {
    return [];
  }
}

/** A prompt file srelens could not load, and why. */
export interface PromptIssue {
  file: string;
  problem: string;
}

/** Returns [] rather than throwing: no prompts directory is not an error. */
export async function promptIssues(): Promise<PromptIssue[]> {
  try {
    return await invoke<PromptIssue[]>("mcp_prompt_issues");
  } catch {
    return [];
  }
}
