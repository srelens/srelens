import { useEffect, useMemo, useState } from "react";
import {
  isApplePlatform,
  notify,
  vaultBiometricDisable,
  vaultBiometricEnable,
  vaultBiometricStatus,
  vaultChangePassword,
  vaultLock,
  type VaultBiometricStatus,
} from "@srelens/core";
import { Alert, Button, Field, LoadingState, Panel, Switch, TextInput } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { hint } from "../../lib/shortcuts";

/**
 * §23's `Security` pane: the passphrase that seals the vault, the button that
 * discards its key, and the biometric skip.
 *
 * **The paragraph names what the vault holds, and §23's did not.** §23 says
 * "Kubeconfigs and cluster tokens are sealed at rest with a key derived from
 * your master passphrase", and that is false. `Secrets`
 * (`apps/desktop/src-tauri/src/vault.rs:43-50`) has exactly two fields:
 * `mcp_token` and `llm_keys`. A kubeconfig never enters the vault —
 * `all_kubeconfig_paths()` (`crates/registry/src/lib.rs:65`) returns
 * `~/.kube/config` plus the managed files, `kubeconfig_files_in()`
 * (`crates/kube/src/connect.rs:100`) enumerates them as plain files, and there
 * is no `encrypt`, `aes` or `chacha` anywhere in `crates/kube` or
 * `crates/registry`. Classic's `VaultGate`
 * (`apps/desktop/src/components/VaultGate.tsx:145`) has said it correctly
 * since it shipped — "srelens encrypts its stored secrets (MCP token,
 * assistant API keys)" — so the new design replaced an accurate sentence with
 * the mock's inaccurate one, on a Security pane. That is worse than a new
 * screen being vague, and the pane says what is actually sealed instead.
 *
 * **The three ways in are all named, because the passphrase is not the only
 * one.** "Anything that needs to read a sealed secret again asks for your
 * passphrase first" was untrue twice: `vault_biometric_enable`
 * (`apps/desktop/src-tauri/src/vault_biometric.rs:75-81`) writes
 * `to_hex(&key)` into the platform biometric store, and
 * `recover_password_core` (`vault_password.rs:212-246`) reads the OS keychain
 * and opens the vault with nothing typed at all. The phrase "the key is never
 * written down" is gone with them: this pane said it nineteen lines below the
 * hint describing the store it is written into.
 *
 * **What locking does NOT do is still said.** The design's copy
 * ends "locking the workspace discards it, and nothing is readable again until
 * you unlock." That is false as written too. `vault_lock`
 * (`apps/desktop/src-tauri/src/vault_password.rs`) discards the derived key and
 * touches nothing else — verified while it was built: a bearer token the
 * running MCP server has already issued stays valid in its memory, and cluster
 * clients built from an already-read kubeconfig keep working, until srelens
 * restarts. So this pane says what locking does — the vault is sealed, and a
 * sealed secret cannot be read again until the key is derived or unwrapped
 * again — and says plainly what it does NOT do. Nine strings claiming more than srelens knows have been
 * removed from this migration; one inside a Security pane, quoting the design,
 * is the worst place to add the tenth. Closing live clients on lock is a real
 * feature and belongs with #367.
 *
 * **No `Lock on launch` switch, no `Lock when idle` select** (#367). Neither
 * exists, and they are opposites in an important way. Locking on launch is not
 * a preference — it is unconditional: nothing carries the key in memory across
 * a launch, so `VaultGate` (`apps/desktop/src/components/VaultGate.tsx`)
 * reports `locked` on every start and asks before the vault opens. A switch for it would imply
 * it could be turned off. An idle lock, by contrast, does not exist at all —
 * there is no timer anywhere in the desktop, the core or this package. The
 * pane states both facts in one line rather than drawing a control for either,
 * because a reader who came here expecting an idle timeout should be told it is
 * not built instead of hunting for it.
 *
 * **The `⌘⇧L` kbd beside `Lock now` is read from `lib/shortcuts.ts`, not
 * typed here.** For most of this branch there was no hint at all: `BINDINGS`
 * had no `Mod+Shift+L` row, so §23's kbd would have promised a key that did
 * nothing — the same kind of false claim as the paragraph above, in three
 * characters — and the absence was pinned by a test so that whoever bound the
 * chord would fail it and add the hint in the same commit. The chord is bound
 * now (`{ type: "lock" }`, handled in `shell/Window.tsx`), so the hint is
 * drawn. It comes from {@link hint} for the reason the Shortcuts pane gives at
 * length: a glyph typed out by hand is wrong the moment the binding moves, and
 * stays wrong until somebody notices.
 *
 * **`vaultStatus()` is deliberately not read here.** Two of its four fields
 * (`biometricAvailable`, `biometricEnrolled`) duplicate
 * {@link vaultBiometricStatus}, which additionally reports whether the vault
 * holds a key; and the other two cannot vary where this pane renders — `mode`
 * is `unlocked` because the gate blocks the whole window until it is, and
 * `keySource` then describes where the MASTER key lives, which is the same
 * value Task 3 declined to render on the MCP pane for reading as a claim about
 * something else. A read whose result cannot change the screen is a call that
 * can only fail.
 */

/**
 * The floor the backend enforces: `MIN_PASSWORD_LEN` in
 * `apps/desktop/src-tauri/src/vault_password.rs`, checked by
 * `change_password_core` before anything is staged. Mirrored so the submit
 * button reflects the rule that already exists rather than inventing a second
 * one — if the two ever disagree, the backend's refusal is the truth and this
 * constant is the bug.
 */
export const MIN_PASSPHRASE_LENGTH = 8;

/**
 * What the OS calls its biometric unlock. The plugin behind
 * `vault_biometric_enable` backs onto Touch ID on macOS and Windows Hello on
 * Windows; Linux has no backend, so `available` is false there and the control
 * never renders. Same derivation the shipped Settings screen uses
 * (`apps/desktop/src/components/SecuritySettingsSection.tsx`) — §23's label
 * says `Touch ID`, which is right on a Mac and wrong on the machine where the
 * gate is called something else.
 */
const BIOMETRIC_LABEL =
  typeof navigator !== "undefined" && navigator.userAgent.includes("Windows")
    ? "Windows Hello"
    : "Touch ID";

/**
 * Loading, refused, and answered kept apart. A boolean cannot carry three
 * states: this branch has twice shipped a render that stated a default as fact
 * after the read that should have filled it was refused, and a pane about
 * security is not the place for a third.
 */
type Read<T> = { kind: "loading" } | { kind: "failed"; error: unknown } | { kind: "ready"; value: T };

export interface SecurityPaneProps {
  /**
   * Raise the lock surface over the window. Called only after `vaultLock()`
   * has resolved — a cover over a vault that is still open, or a sealed vault
   * under a window still listing clusters, would each make the paragraph above
   * untrue in a different direction.
   */
  onLocked: () => void;
}

export function SecurityPane({ onLocked }: SecurityPaneProps) {
  const apple = useMemo(() => isApplePlatform(), []);
  const [biometric, setBiometric] = useState<Read<VaultBiometricStatus>>({ kind: "loading" });
  const [biometricBusy, setBiometricBusy] = useState(false);
  const [biometricError, setBiometricError] = useState<unknown>(null);
  const [locking, setLocking] = useState(false);
  const [lockError, setLockError] = useState<unknown>(null);
  const [changing, setChanging] = useState(false);
  const [changeBusy, setChangeBusy] = useState(false);
  const [changeError, setChangeError] = useState<unknown>(null);
  const [changeWarning, setChangeWarning] = useState<string | null>(null);
  const [held, setHeld] = useState("");
  const [wanted, setWanted] = useState("");
  const [repeated, setRepeated] = useState("");

  useEffect(() => {
    let cancelled = false;
    void readBiometric((read) => {
      if (!cancelled) setBiometric(read);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  async function lock() {
    setLockError(null);
    setLocking(true);
    try {
      await vaultLock();
    } catch (e) {
      // The window is exactly as it was, so nothing is raised over it.
      setLockError(e);
      setLocking(false);
      return;
    }
    setLocking(false);
    onLocked();
  }

  async function toggleBiometric(on: boolean) {
    setBiometricBusy(true);
    setBiometricError(null);
    try {
      if (on) await vaultBiometricEnable();
      else await vaultBiometricDisable();
    } catch (e) {
      setBiometricError(e);
    } finally {
      setBiometricBusy(false);
      // Re-read either way. Both commands are multi-step and either can fail
      // partway with the previous configuration intact, so what the switch
      // shows next comes from the backend rather than from what was asked for.
      await readBiometric(setBiometric);
    }
  }

  function resetChangeForm() {
    setHeld("");
    setWanted("");
    setRepeated("");
  }

  async function change() {
    setChangeError(null);
    setChangeWarning(null);
    setChangeBusy(true);
    try {
      const warning = await vaultChangePassword(held, wanted);
      resetChangeForm();
      setChanging(false);
      notify.success("Master passphrase changed");
      // A resolved STRING means the change landed but the biometric enrollment
      // had to be turned off (`vaultChangePassword`'s contract). Dropping it
      // would leave the reader believing an unlock method still works, so it
      // gets a surface that stays on screen rather than a toast.
      if (warning !== null) setChangeWarning(warning);
      await readBiometric(setBiometric);
    } catch (e) {
      // The form stays open with what was typed still in it: the usual refusal
      // here is a wrong current passphrase, and clearing the fields would make
      // the second attempt start from nothing.
      setChangeError(e);
    } finally {
      setChangeBusy(false);
    }
  }

  const mismatch = repeated !== "" && repeated !== wanted;
  const submittable =
    held !== "" && wanted.length >= MIN_PASSPHRASE_LENGTH && repeated === wanted && !changeBusy;

  /**
   * Enter from any of the three fields, guarded by the same rule the button is
   * disabled by — a passphrase form where Return does nothing is one people
   * press twice and then look for the button, and an unguarded one submits a
   * half-typed repeat straight into a re-key.
   */
  function submit() {
    if (submittable) void change();
  }

  return (
    <div className="flex flex-col gap-4">
      <Panel title="Vault">
        <p className="text-[0.75rem] leading-relaxed text-muted">
          srelens seals its own stored secrets — the MCP bearer token and your assistant API keys —
          with a key derived from your master passphrase. Your kubeconfigs are not sealed: srelens
          reads them as ordinary files, and what protects them is whatever protects them on your
          machine already.
        </p>
        <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
          The key is derived, not stored — locking the workspace discards it, and a sealed secret
          cannot be read again until it is derived again. That takes your passphrase, or{" "}
          {BIOMETRIC_LABEL} if you allowed it below, or the recovery copy if you kept one in this
          machine&apos;s keychain at setup.
        </p>
        {/* The half of the sentence §23 leaves out. #367 is where closing live
            clients on lock belongs; until then this says which of them stay. */}
        <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
          Locking seals the vault; it does not close what is already open. A bearer token the MCP
          server has already handed out, and cluster connections already made, keep working until
          srelens restarts.
        </p>
        {/* flex-wrap rather than a fixed row: the second label is the longer of
            the two and grows in translation, and a flex child with nothing to
            stop it shrinking is where `min-width: auto` has cost this migration
            eight defects. Wrapping is the behaviour that stays legible. */}
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-2">
            <Button disabled={locking} onClick={() => void lock()}>
              {locking ? "Locking…" : "Lock now"}
            </Button>
            {/* Beside the button rather than inside it: the chord is not part of
                the button's accessible name, and a `kbd` in there would be read
                out as one. */}
            <kbd className="kbd shrink-0">{hint("lock", apple)}</kbd>
          </div>
          {!changing && (
            <Button variant="secondary" onClick={() => setChanging(true)}>
              Change passphrase
            </Button>
          )}
        </div>
        {lockError !== null && (
          <FailureAlert
            // `sev`, which is the kit's assertive live region: a reader who
            // asked for the workspace to be sealed and walks away believing it
            // was is the failure this message exists to prevent.
            tone="sev"
            title="The workspace could not be locked"
            error={lockError}
            className="mt-3"
          />
        )}
        {changeWarning !== null && (
          <Alert tone="warn" title="The passphrase changed, with one consequence" className="mt-3">
            {changeWarning}
          </Alert>
        )}
        {changing && (
          <div className="mt-3 max-w-sm">
            <Field label="Current passphrase">
              <TextInput type="password" value={held} onValueChange={setHeld} onEnter={submit} />
            </Field>
            <Field
              label="New passphrase"
              hint={`At least ${MIN_PASSPHRASE_LENGTH} characters.`}
            >
              <TextInput type="password" value={wanted} onValueChange={setWanted} onEnter={submit} />
            </Field>
            {/* Not "Repeat the new passphrase": under a field already called
                `New passphrase`, two labels sharing that phrase name the same
                thing twice, and neither the reader nor a query for one of them
                can tell which is which. */}
            <Field
              label="Repeat the new one"
              error={mismatch ? "The two entries do not match." : undefined}
              hint="A mistyped passphrase would seal the vault behind something you never typed on purpose."
            >
              <TextInput
                type="password"
                value={repeated}
                onValueChange={setRepeated}
                onEnter={submit}
                invalid={mismatch}
              />
            </Field>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Button disabled={!submittable} onClick={() => void change()}>
                {changeBusy ? "Changing…" : "Set the new passphrase"}
              </Button>
              <Button
                variant="ghost"
                disabled={changeBusy}
                onClick={() => {
                  resetChangeForm();
                  setChangeError(null);
                  setChanging(false);
                }}
              >
                Cancel
              </Button>
            </div>
            {changeError !== null && (
              <FailureAlert
                tone="sev"
                title="The passphrase could not be changed"
                error={changeError}
                className="mt-3"
              />
            )}
          </div>
        )}
      </Panel>

      <Panel title="Unlocking">
        {biometric.kind === "loading" && (
          <LoadingState label="Checking what this machine can unlock with" />
        )}
        {biometric.kind === "failed" && (
          <FailureAlert
            title="Whether this machine offers biometric unlock could not be read"
            error={biometric.error}
          />
        )}
        {biometric.kind === "ready" &&
          (biometric.value.available ? (
            <Switch
              on={biometric.value.enabled}
              // Not disabled on a locked vault, though enabling one needs a key
              // in memory. The backend refuses that with a sentence of its own
              // ("the vault is locked — unlock it before enabling biometric
              // unlock"), which tells the reader more than a switch that is
              // merely dim; and the gate makes it unreachable from here anyway.
              disabled={biometricBusy}
              onChange={(on) => void toggleBiometric(on)}
              label={`Allow ${BIOMETRIC_LABEL}`}
              // §23's hint says "from the secure enclave". The key goes into
              // whatever biometric store the platform plugin has — the Secure
              // Enclave's keychain on a Mac, and something else entirely behind
              // Windows Hello — so the wording follows the label rather than
              // naming Apple's hardware on a machine that has none. "The
              // passphrase still works" is kept and is true: enabling leaves
              // `vault.json` in place, and `vault_unlock_password` derives the
              // key from it without consulting the gate.
              hint={`Puts a copy of the derived key in this machine's biometric store and unwraps it from there. Your passphrase still works.`}
            />
          ) : (
            <p className="text-[0.75rem] leading-relaxed text-muted">
              srelens found no biometric sensor it can use here, so there is nothing to allow — your
              passphrase is the only way in.
            </p>
          ))}
        {biometricError !== null && (
          <FailureAlert
            tone="sev"
            title={`${BIOMETRIC_LABEL} unlock could not be changed`}
            error={biometricError}
            className="mt-3"
          />
        )}
        {/* #367: neither of §23's other two controls is drawn. See the note at
            the top of this file for why the two absences are different. */}
        <p className="mt-3 text-[0.75rem] leading-relaxed text-muted">
          srelens starts locked every time: nothing carries the key in memory across a launch, so
          every start asks for it — your passphrase, or {BIOMETRIC_LABEL} where that is allowed. It
          does not lock itself when idle, though: there is no idle timer, and a workspace left
          unlocked stays unlocked until you lock it.
        </p>
      </Panel>
    </div>
  );
}

/** One place that turns the biometric read into one of its three states. */
async function readBiometric(apply: (read: Read<VaultBiometricStatus>) => void): Promise<void> {
  try {
    apply({ kind: "ready", value: await vaultBiometricStatus() });
  } catch (error) {
    apply({ kind: "failed", error });
  }
}
