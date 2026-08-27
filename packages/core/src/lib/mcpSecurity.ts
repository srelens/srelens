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
 * unprotected); "locked" — the keychain holding the key was unreachable this
 * launch while a vault exists, so secrets are frozen rather than destroyed by
 * a re-key; "biometric" — the Touch ID gate is on and passed; or
 * "biometric-locked" — the gate hasn't been passed yet this launch. */
export type VaultKeySource =
  | "keychain"
  | "file"
  | "locked"
  | "biometric"
  | "biometric-locked"
  | "password"
  | "password-locked";

export async function getMcpTokenStorage(): Promise<VaultKeySource> {
  return await invoke<VaultKeySource>("mcp_token_storage");
}

/** What Settings needs to render the Touch ID control. */
export interface VaultBiometricStatus {
  available: boolean;
  enabled: boolean;
  unlocked: boolean;
}

export async function vaultBiometricStatus(): Promise<VaultBiometricStatus> {
  return await invoke<VaultBiometricStatus>("vault_biometric_status");
}

export async function vaultBiometricEnable(): Promise<void> {
  await invoke("vault_biometric_enable");
}

export async function vaultBiometricDisable(): Promise<void> {
  await invoke("vault_biometric_disable");
}

/** Raises the Touch ID sheet; resolves once the vault is unlocked. */
export async function vaultBiometricUnlock(): Promise<void> {
  await invoke("vault_biometric_unlock");
}

/** What the mandatory VaultGate renders from. */
export interface VaultStatus {
  mode: "setup-required" | "locked" | "unlocked";
  keySource: VaultKeySource;
  biometricAvailable: boolean;
  biometricEnrolled: boolean;
}

export async function vaultStatus(): Promise<VaultStatus> {
  return await invoke<VaultStatus>("vault_status");
}

export async function vaultSetupPassword(password: string, keepRecovery: boolean): Promise<void> {
  await invoke("vault_setup_password", { password, keepRecovery });
}

export async function vaultUnlockPassword(password: string): Promise<void> {
  await invoke("vault_unlock_password", { password });
}

/** The explicit "Forgot password?" flow: returns the recovered password for
 * one-time display (the vault is unlocked as a side effect). */
export async function vaultRecoverPassword(): Promise<string> {
  return await invoke<string>("vault_recover_password");
}

/** Lock the workspace: the backend discards the derived key it holds in
 * memory. The vault's sealed bytes are untouched — this forgets a key, it does
 * not change one, and the same master password re-opens it. `vaultStatus()`
 * reports `"locked"` afterwards, so the gate takes over again. Rejects (and
 * the vault stays open) if there is nothing to lock to yet. */
export async function vaultLock(): Promise<void> {
  await invoke("vault_lock", {});
}

/** The backend refreshes the keychain recovery copy only if one exists —
 * setup's opt-in/opt-out choice is preserved, never silently reversed.
 * Resolves to a warning string when the change succeeded but the biometric
 * enrollment had to be turned off (its store couldn't be updated). */
export async function vaultChangePassword(current: string, next: string): Promise<string | null> {
  return await invoke<string | null>("vault_change_password", { current, new: next });
}

/**
 * The newest `limit` capability calls, newest first.
 *
 * **Rejects when the trail cannot be read**, and that is a deliberate change
 * from what this used to do. It caught every refusal and returned `[]`, which
 * left "no agent has called anything" and "srelens could not read what the
 * agent called" as the same value — so a caller could only ever draw the
 * first. That collapse shipped: the new design's `Audit` pane rendered "No
 * capability calls yet · A fresh install has made none — this is not an error"
 * over a failed read, guaranteed on the web build where every `invoke`
 * rejects, on the one screen whose whole purpose is answering "what did the
 * agent do?" after an incident.
 *
 * **And the backend now draws the same line**, which is what makes the two
 * outcomes here mean what they say. Rejecting in TypeScript alone only ever
 * distinguished an IPC failure: `srelens_mcp::audit::tail`
 * (`crates/mcp/src/audit.rs`) used to return an empty vector for a log it could
 * not open, seek or read as well as for one that did not exist, so an
 * unreadable trail still arrived here as a perfectly successful `[]`. It
 * returns `io::Result` now — `Ok(vec![])` for a log that is genuinely absent,
 * because a fresh install has made no calls, and an `Err` otherwise — and
 * `mcp_audit_tail` (`apps/desktop/src-tauri/src/mcp.rs`) maps that to a
 * refusal naming the file.
 *
 * So a resolved `[]` is "there are no entries", and a rejection is any of: the
 * log exists and could not be read, no such command, an IPC failure, or the
 * web build, where every `invoke` rejects because there is no Tauri host behind
 * it.
 */
export async function auditTail(limit: number): Promise<AuditEntry[]> {
  return await invoke<AuditEntry[]>("mcp_audit_tail", { limit });
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
