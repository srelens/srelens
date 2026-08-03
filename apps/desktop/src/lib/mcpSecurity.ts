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
  err?: string | null;
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

/** Newest first. Returns [] rather than throwing: a missing log is not an error. */
export async function auditTail(limit: number): Promise<AuditEntry[]> {
  try {
    return await invoke<AuditEntry[]>("mcp_audit_tail", { limit });
  } catch {
    return [];
  }
}
