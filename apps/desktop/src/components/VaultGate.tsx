import { useEffect, useRef, useState } from "react";
import { Lock } from "lucide-react";
import { Button, TextInput } from "../ui";
import {
  vaultBiometricUnlock,
  vaultRecoverPassword,
  vaultSetupPassword,
  vaultStatus,
  vaultUnlockPassword,
  type VaultStatus,
} from "../lib/mcpSecurity";

/**
 * The mandatory master-password gate (mqlens-style, issue #208): a blocking
 * overlay rendered app-wide until the secrets vault is set up and unlocked.
 * First launch shows setup (create password + optional keychain recovery
 * copy); later launches show unlock — password, or one auto-raised biometric
 * prompt when the Touch ID / Windows Hello skip is enrolled. "Forgot
 * password?" reads the opt-in keychain recovery copy and shows it once.
 *
 * Renders nothing outside a Tauri window (web mode has no vault commands).
 */
export function VaultGate({ onReady }: { onReady?: () => void }) {
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [keepRecovery, setKeepRecovery] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [recovered, setRecovered] = useState<string | null>(null);
  // The status command itself failed (desktop-only) — blocking error state.
  const [statusFailed, setStatusFailed] = useState(false);
  // The biometric prompt must fire once per launch, not once per render.
  const autoPrompted = useRef(false);
  // `onReady` fires exactly once, when the vault becomes usable — consumers
  // (the MCP auto-start) must not run against a locked vault.
  const readyNotified = useRef(false);

  function notifyReady() {
    if (!readyNotified.current) {
      readyNotified.current = true;
      onReady?.();
      // Vault-dependent views may already be mounted UNDER the gate (e.g. a
      // restored Assistant tab, whose agent list was fetched while locked
      // and saw no API keys). Broadcast so they re-read vault-backed state.
      window.dispatchEvent(new Event("srelens:vault-unlocked"));
    }
  }

  async function refresh(): Promise<VaultStatus | null> {
    try {
      const s = await vaultStatus();
      setStatus(s);
      setStatusFailed(false);
      if (s.mode === "unlocked") notifyReady();
      return s;
    } catch {
      // Only reachable in a Tauri window (the mount effect never calls this
      // in web mode): the backend genuinely failed — e.g. the config dir
      // didn't resolve and the vault state was never managed. The gate must
      // STAY CLOSED with a retry, never quietly wave the app through the
      // mandatory setup/unlock.
      setStatus(null);
      setStatusFailed(true);
      return null;
    }
  }

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    void refresh().then((s) => {
      // The enrolled biometric skip IS the launch unlock: raise it once,
      // uninvited. Cancelling falls back to the password form below.
      if (s?.mode === "locked" && s.biometricEnrolled && !autoPrompted.current) {
        autoPrompted.current = true;
        void vaultBiometricUnlock()
          .then(() => refresh())
          .catch(() => {});
      }
    });
  }, []);

  if (statusFailed) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-background">
        <div className="flex w-96 flex-col gap-4 rounded-xl border border-border bg-card p-6 shadow-lg">
          <div className="flex items-center gap-2">
            <Lock className="size-5 text-muted-foreground" aria-hidden="true" />
            <h1 className="text-lg font-semibold">Secrets unavailable</h1>
          </div>
          <p className="text-sm text-muted-foreground">
            srelens couldn't reach its secrets vault. Stored secrets stay locked until this
            resolves — retry, or restart srelens.
          </p>
          <Button onClick={() => void refresh()}>Retry</Button>
        </div>
      </div>
    );
  }

  // Stay mounted while the recovered password is on screen even though the
  // recovery flow already unlocked the vault — Continue dismisses it.
  if (!status || (status.mode === "unlocked" && recovered === null)) return null;

  async function run(action: () => Promise<unknown>) {
    setError("");
    setBusy(true);
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const setup = status.mode === "setup-required";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background">
      <form
        className="flex w-96 flex-col gap-4 rounded-xl border border-border bg-card p-6 shadow-lg"
        onSubmit={(e) => {
          e.preventDefault();
          if (setup) {
            if (password !== confirm) {
              setError("The passwords don't match.");
              return;
            }
            void run(() => vaultSetupPassword(password, keepRecovery));
          } else {
            void run(() => vaultUnlockPassword(password));
          }
        }}
      >
        <div className="flex items-center gap-2">
          <Lock className="size-5 text-muted-foreground" aria-hidden="true" />
          <h1 className="text-lg font-semibold">
            {setup ? "Protect your secrets" : "Unlock srelens"}
          </h1>
        </div>
        <p className="text-sm text-muted-foreground">
          {setup
            ? "srelens encrypts its stored secrets (MCP token, assistant API keys) with a master password you choose. You'll enter it — or use biometrics — each time srelens starts."
            : "Enter your master password to unlock srelens's stored secrets."}
        </p>

        {recovered !== null ? (
          <div className="flex flex-col gap-3">
            <p className="text-sm">
              Your master password is: <code className="rounded bg-muted px-1.5 py-0.5 font-mono">{recovered}</code>
            </p>
            <p className="text-xs text-muted-foreground">
              Note it somewhere safe — or change it in Settings → MCP. The vault is unlocked.
            </p>
            <Button
              onClick={() => {
                setRecovered(null);
                void refresh();
              }}
            >
              Continue
            </Button>
          </div>
        ) : (
          <>
            <label className="flex flex-col gap-1 text-sm">
              <span className="text-xs text-muted-foreground">Master password</span>
              <TextInput
                type="password"
                autoFocus
                value={password}
                onValueChange={setPassword}
                placeholder={setup ? "At least 8 characters" : "Master password"}
              />
            </label>
            {setup && (
              <>
                <label className="flex flex-col gap-1 text-sm">
                  <span className="text-xs text-muted-foreground">Confirm password</span>
                  <TextInput type="password" value={confirm} onValueChange={setConfirm} />
                </label>
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    className="accent-primary"
                    checked={keepRecovery}
                    onChange={(e) => setKeepRecovery(e.target.checked)}
                  />
                  <span>Keep a recovery copy in the OS keychain</span>
                </label>
              </>
            )}
            {error && <p className="text-sm text-destructive">{error}</p>}
            <Button type="submit" disabled={busy || password.length === 0}>
              {setup ? "Create password" : "Unlock"}
            </Button>
            {!setup && status.biometricEnrolled && status.biometricAvailable && (
              <Button
                type="button"
                variant="secondary"
                disabled={busy}
                onClick={() => void run(() => vaultBiometricUnlock())}
              >
                Unlock with {navigator.userAgent.includes("Windows") ? "Windows Hello" : "Touch ID"}
              </Button>
            )}
            {!setup && (
              <button
                type="button"
                className="text-xs text-muted-foreground underline"
                onClick={() => {
                  setError("");
                  setBusy(true);
                  vaultRecoverPassword()
                    .then(setRecovered)
                    .catch((e) => setError(String(e)))
                    .finally(() => setBusy(false));
                }}
              >
                Forgot password?
              </button>
            )}
          </>
        )}
      </form>
    </div>
  );
}
