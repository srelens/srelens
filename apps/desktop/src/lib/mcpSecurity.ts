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

/** The backend refreshes the keychain recovery copy only if one exists —
 * setup's opt-in/opt-out choice is preserved, never silently reversed.
 * Resolves to a warning string when the change succeeded but the biometric
 * enrollment had to be turned off (its store couldn't be updated). */
export async function vaultChangePassword(current: string, next: string): Promise<string | null> {
  return await invoke<string | null>("vault_change_password", { current, new: next });
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
