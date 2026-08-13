import { useEffect, useState } from "react";
import { Button, TextInput } from "../ui";
import { notify } from "../lib/notify";
import {
  getMcpTokenStorage,
  vaultBiometricDisable,
  vaultBiometricEnable,
  vaultBiometricStatus,
  vaultBiometricUnlock,
  vaultChangePassword,
  type VaultBiometricStatus,
  type VaultKeySource,
} from "../lib/mcpSecurity";

/**
 * Settings → Security: everything protecting the secrets vault (the master
 * password set up at first launch, the Touch ID / Windows Hello skip, and
 * the state of the key that encrypts `secrets.enc`). Moved out of the MCP
 * section — these guard ALL stored secrets, not just the MCP token.
 */
export function SecuritySettingsSection() {
  const [keySource, setKeySource] = useState<VaultKeySource | null>(null);
  const [biometric, setBiometric] = useState<VaultBiometricStatus | null>(null);
  const [biometricBusy, setBiometricBusy] = useState(false);
  const [pwCurrent, setPwCurrent] = useState("");
  const [pwNew, setPwNew] = useState("");
  const [pwConfirm, setPwConfirm] = useState("");
  const [pwBusy, setPwBusy] = useState(false);

  // What the OS calls its biometric unlock — the plugin backs onto Touch ID
  // on macOS and Windows Hello on Windows (Linux has no backend, so the
  // control never renders there and this label is moot).
  const bioLabel = navigator.userAgent.includes("Windows") ? "Windows Hello" : "Touch ID";

  useEffect(() => {
    void refreshVaultState();
  }, []);

  /** Re-read both vault facts the controls render from. */
  async function refreshVaultState() {
    await Promise.all([
      getMcpTokenStorage().then(setKeySource).catch(() => {}),
      vaultBiometricStatus().then(setBiometric).catch(() => {}),
    ]);
  }

  async function toggleBiometric(on: boolean) {
    setBiometricBusy(true);
    try {
      if (on) {
        await vaultBiometricEnable();
        notify.success(`${bioLabel} required to unlock secrets from the next launch`);
      } else {
        await vaultBiometricDisable();
        notify.success(`${bioLabel} requirement removed`);
      }
    } catch (e) {
      notify.error(String(e));
    } finally {
      setBiometricBusy(false);
      await refreshVaultState();
    }
  }

  async function unlockBiometric() {
    setBiometricBusy(true);
    try {
      await vaultBiometricUnlock();
      notify.success("Secrets unlocked");
    } catch (e) {
      notify.error(String(e));
    } finally {
      setBiometricBusy(false);
      await refreshVaultState();
    }
  }

  async function changePassword() {
    if (pwNew !== pwConfirm) {
      notify.error("The new passwords don't match");
      return;
    }
    setPwBusy(true);
    try {
      // The backend refreshes the keychain recovery copy only if one exists,
      // preserving the opt-in/opt-out made at setup. A warning means the
      // change landed but the biometric enrollment had to be turned off.
      const warning = await vaultChangePassword(pwCurrent, pwNew);
      if (warning) {
        notify.info(`Master password changed — ${warning}`);
        await refreshVaultState();
      } else {
        notify.success("Master password changed");
      }
      setPwCurrent("");
      setPwNew("");
      setPwConfirm("");
    } catch (e) {
      notify.error(String(e));
    } finally {
      setPwBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold">Security</h2>
        <p className="text-sm text-muted-foreground">
          The master password and biometric unlock protecting srelens's stored secrets — the MCP
          token and the assistant's API keys.
        </p>
      </div>

      <section className="flex flex-col gap-3">
        {keySource === "file" && (
          <p className="text-sm text-amber-600 dark:text-amber-500">
            No OS keychain is available here, so the key that encrypts srelens's secrets is stored
            in a plain file on disk (readable only by your user account) rather than the OS
            keychain — the encrypted secrets file is then only obfuscation.
          </p>
        )}
        {keySource === "locked" && (
          <p className="text-sm text-destructive">
            srelens couldn't load the key that encrypts its secrets when it started — the OS
            keychain was unreachable, or the key file couldn't be created. Stored secrets can't be
            read or changed right now; they are untouched. Restart srelens once the keychain is
            available again.
          </p>
        )}
        {keySource === "biometric-locked" && (
          <div className="flex items-center gap-3">
            <p className="text-sm text-amber-600 dark:text-amber-500">
              Secrets are locked behind {bioLabel} for this session.
            </p>
            <Button size="sm" disabled={biometricBusy} onClick={() => void unlockBiometric()}>
              Unlock with {bioLabel}
            </Button>
          </div>
        )}
        {biometric?.available && keySource !== "locked" && (
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              className="accent-primary"
              checked={biometric.enabled}
              disabled={biometricBusy || (!biometric.unlocked && !biometric.enabled)}
              onChange={(e) => void toggleBiometric(e.target.checked)}
            />
            <span>Unlock with {bioLabel} instead of the master password</span>
            <span className="text-xs text-muted-foreground">
              — asks once each time srelens starts
            </span>
          </label>
        )}
      </section>

      {(keySource === "password" || keySource === "biometric") && (
        <section className="flex flex-col gap-2 border-t border-border pt-4">
          <h3 className="text-sm font-medium">Change master password</h3>
          <div className="flex max-w-md flex-col gap-2">
            <TextInput
              type="password"
              placeholder="Current password"
              value={pwCurrent}
              onValueChange={setPwCurrent}
            />
            <TextInput
              type="password"
              placeholder="New password (at least 8 characters)"
              value={pwNew}
              onValueChange={setPwNew}
            />
            <TextInput
              type="password"
              placeholder="Confirm new password"
              value={pwConfirm}
              onValueChange={setPwConfirm}
            />
            <div>
              <Button
                disabled={pwBusy || !pwCurrent || pwNew.length < 8}
                onClick={() => void changePassword()}
              >
                Change password
              </Button>
            </div>
          </div>
        </section>
      )}
    </div>
  );
}
