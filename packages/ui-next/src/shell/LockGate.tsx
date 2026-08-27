import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import {
  isTauri,
  vaultBiometricUnlock,
  vaultRecoverPassword,
  vaultSetupPassword,
  vaultStatus,
  vaultUnlockPassword,
  type VaultStatus,
} from "@srelens/core";
import {
  Button,
  Checkbox,
  Eyebrow,
  Field,
  LoadingState,
  Mark,
  RawError,
  Spinner,
  TextInput,
} from "@srelens/ui-kit";
import { useContexts } from "../lib/clusters";
import { FailureAlert, friendly } from "../lib/errorCopy";

/**
 * §25's lock screen: the surface that covers the window while the vault is
 * sealed.
 *
 * **Chrome, not a route** (spec decision 5). It replaces the whole middle band
 * — the cluster rail, the sidebar, the tab strip and every tab's screen — and
 * it does that by rendering INSTEAD of its children rather than over them.
 * That distinction is the whole point. Since PR #365 a dialog is mounted in the
 * tab it was opened from, so the rail and the other tabs stay reachable behind
 * it, which is right for a dialog and exactly wrong for a lock: a cover that
 * left them live would be worse than no lock at all, because the window would
 * *look* sealed while every other tab kept running over a sealed vault. A
 * route would be worse again — it can be left by clicking another tab.
 *
 * **What this costs, said plainly.** Unmounting the children throws away DOM
 * state the tab bodies hold: a terminal's rendered scrollback, a table's scroll
 * position, an unsent form. Everything srelens treats as session state lives in
 * module-level stores instead (`tabsStore`, the forwards store, the sessions
 * store) and comes back when the cover lifts, and the backend keeps forwarding
 * and keeps its PTYs alive throughout. That trade is deliberate: the spec's
 * "locking should replace the body, not merely mark the vault closed" cannot be
 * had while the body is still mounted and still showing what it read.
 *
 * **This ports `VaultGate`** (`apps/desktop/src/components/VaultGate.tsx`),
 * which is the classic design's version of the same gate and the only one that
 * ships today — the new design's tree mounts no `VaultGate` at all
 * (`apps/desktop/src/main.tsx` renders `NextApp` and nothing else). So the
 * first-launch setup path, the unlock path, the keychain recovery path and the
 * uninvited biometric prompt are all here, behaving as they already do. §25
 * supplies the copy and the shape; none of the flows are redesigned.
 *
 * **§25's mock hint does not ship.** *"Mock: the passphrase is `srelens`, or
 * use Touch ID."* is fixture text for a mock with a fixture vault. Printing a
 * working credential on a lock screen is the single worst string this migration
 * could ship, so there is a test asserting it is absent — including from the
 * places a value can hide out of the visible text (a placeholder, a name, a
 * title, a pre-filled field).
 *
 * **Auto-lock is not built** (#367): no idle timer, and no `Lock on launch`
 * switch — no process carries the derived key across a launch, so the vault is
 * *always* sealed at launch and this gate always asks (for the passphrase, or
 * for the biometric skip where a copy of the key was put in the platform
 * store). That is not a missing preference; a switch for it would imply it
 * could be turned off.
 */

/**
 * Whether the cover is up, held outside React.
 *
 * A module store rather than component state, because the raise has to be
 * callable from places that are not inside this component and are not even
 * inside a render: `Window`'s accelerator handler, and the `onLocked` the
 * Security pane is handed three hops away. That is also what makes the contract
 * Task 8 specified achievable at all — a zero-argument, synchronous,
 * non-throwing, fire-and-forget `() => void`.
 *
 * **Nothing here awaits anything.** `vaultStatus()` is not consulted before the
 * cover goes up, and that is the point: every await between "the vault is
 * sealed" and "the window is covered" is a window of live UI over a sealed
 * vault. Cover first; reconcile after.
 */
let sealed = false;

/**
 * The mode the last successful `vaultStatus()` reported, or null if none has
 * landed yet.
 *
 * Recorded here so the titlebar's `Lock workspace` control can decide whether
 * to draw itself WITHOUT taking a second `vaultStatus()` read of its own. Two
 * readers of the same fact is how two surfaces start disagreeing about it, and
 * this gate is already reading it at launch and after every attempt. Held
 * beside `sealed` because the two are answers to one question — whether there
 * is a vault, and whether it is open.
 *
 * `null` on a failed read as well as before the first one: a refusal is not a
 * mode, and the previous value is not evidence about now. Both cases resolve
 * the same way at the call site — no control is offered.
 */
let knownMode: VaultStatus["mode"] | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  // A copy, because a listener may unsubscribe while this is running.
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Record what the last `vaultStatus()` said, and tell the subscribers.
 *
 * It notifies on its own rather than leaning on the `lockWorkspace()` /
 * `unsealWorkspace()` call that follows it: both of those are no-ops when the
 * cover is already in the state they want, so on the ordinary launch read of an
 * open vault nothing would have been emitted and the titlebar would never have
 * learned there was a vault to lock. Guarded on the value so a re-read that
 * changes nothing wakes nobody.
 */
function rememberMode(mode: VaultStatus["mode"] | null): void {
  if (knownMode === mode) return;
  knownMode = mode;
  emit();
}

/**
 * Raise the cover over the window. Idempotent, because `Lock now` can be
 * double-clicked and the chord can be held down — a second raise over a cover
 * that is already up notifies nobody and changes nothing.
 */
export function lockWorkspace(): void {
  if (sealed) return;
  sealed = true;
  emit();
}

/** Whether the cover is already up, for a caller deciding whether to lock. */
export function isWorkspaceSealed(): boolean {
  return sealed;
}

/**
 * Lower it. Deliberately not exported: the only thing entitled to lower this
 * cover is the gate itself, after a `vaultStatus()` that says the vault is
 * genuinely open. Anything else would be a way to dismiss a lock screen
 * without unlocking.
 */
function unsealWorkspace(): void {
  if (!sealed) return;
  sealed = false;
  emit();
}

/**
 * Reset the module's state between tests, the way `resetContexts` and
 * `resetProbes` do for their stores. Not called by anything shipped: vitest
 * isolates files, not the tests inside one, so a raise in one test would
 * otherwise still be up in the next.
 */
export function resetLock(): void {
  sealed = false;
  knownMode = null;
  emit();
}

/**
 * Put a vault mode into the store, for a test of a component that reads it but
 * does not perform the read.
 *
 * `Chrome` draws `Lock workspace` from {@link useCanLockWorkspace}, and the
 * only thing that fills that store is this gate's own `vaultStatus()` read — so
 * a `Chrome` test rendered on its own has no way to reach the four states it
 * has to tell apart. The alternative was a prop threaded from `Window` purely
 * so a test could set it, which is a shape the app does not need. Underscored
 * and documented as test support, like `resetLock` above it; nothing shipped
 * calls it.
 */
export function __setKnownVaultMode(mode: VaultStatus["mode"] | null): void {
  rememberMode(mode);
}

/**
 * A boolean, not an object. `useSyncExternalStore` re-reads its snapshot after
 * every render and compares by identity, so a getter that allocates never
 * settles — see the same note in `lib/clusters.ts`, which shipped that bug once.
 */
function useSealed(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => sealed,
    () => sealed,
  );
}

/**
 * Whether the cover is up, for a component that is NOT behind it.
 *
 * The cover replaces the middle band, and the two surfaces §25 leaves outside
 * it — the titlebar and the status bar — are siblings of this gate rather than
 * children. Excluding them visually is what §25 asks for; leaving them
 * INTERACTIVE would make the window look sealed while every control on it still
 * acted on the workspace, which is decision 5's own argument for why that is
 * worse than not locking at all. So they subscribe to the same store the cover
 * reads, and stand down while it is up.
 *
 * Exported for exactly those two callers. Anything inside the band is unmounted
 * and has no use for this.
 */
export function useWorkspaceSealed(): boolean {
  return useSealed();
}

/**
 * Whether there is an open vault for the titlebar's control to lock.
 *
 * Three conditions, and each of them is a control this project would otherwise
 * have drawn dead:
 *
 * - a vault that has never been set up REFUSES to lock. `lock_core`
 *   (`apps/desktop/src-tauri/src/vault_password.rs`) says so with a sentence,
 *   because a machine-key vault resolves its key once at open and discarding it
 *   would strand the process until restart. `mode === "setup-required"` is that
 *   state, and a button refused by design is not offered.
 * - a vault whose state has never been read is not known to be lockable. `null`
 *   covers both "the launch read has not answered" and "it refused".
 * - a vault already sealed has nothing to lock, and the cover is up over it.
 *
 * Only `unlocked` passes. Read off this module's own store rather than from a
 * `vaultStatus()` call of the caller's, so there is one read of this fact and
 * one answer to it.
 */
export function useCanLockWorkspace(): boolean {
  const snapshot = () => knownMode === "unlocked" && !sealed;
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/**
 * What the OS calls its biometric unlock. Same derivation the Security pane and
 * classic's own settings section use: the platform plugin backs onto Touch ID
 * on macOS and Windows Hello on Windows, and §25's `Touch ID` is right on a Mac
 * and wrong on the machine where the gate is called something else.
 */
const BIOMETRIC_LABEL =
  typeof navigator !== "undefined" && navigator.userAgent.includes("Windows")
    ? "Windows Hello"
    : "Touch ID";

/**
 * The floor the backend enforces (`MIN_PASSWORD_LEN` in
 * `apps/desktop/src-tauri/src/vault_password.rs`), mirrored on the setup form
 * only. The unlock form does not apply it: an existing vault may have been
 * created before the rule, and refusing to even try a passphrase the backend
 * would accept is a lock-out this screen has no business inventing.
 */
const MIN_PASSPHRASE_LENGTH = 8;

/**
 * The lede, and it is NOT §25's, which said: "Your kubeconfigs and cluster
 * tokens are sealed on this machine. Unlock to derive the key — it is never
 * written to disk." Both halves are false.
 *
 * `Secrets` (`apps/desktop/src-tauri/src/vault.rs:43-50`) holds exactly two
 * fields, `mcp_token` and `llm_keys`. A kubeconfig never enters the vault:
 * `all_kubeconfig_paths()` (`crates/registry/src/lib.rs:65`) returns
 * `~/.kube/config` plus the managed files, `kubeconfig_files_in()`
 * (`crates/kube/src/connect.rs:100`) enumerates them as plain files, and there
 * is no `encrypt`, `aes` or `chacha` anywhere in `crates/kube` or
 * `crates/registry`. Classic's `VaultGate` has said this correctly since it
 * shipped, so §25's sentence was a truthfulness REGRESSION against what
 * already ships.
 *
 * And "never written to disk" is refuted by the button three inches below it:
 * `vault_biometric_enable` (`vault_biometric.rs:75-81`) writes `to_hex(&key)`
 * into the platform biometric store, which is where `Unlock with Touch ID`
 * unwraps it from. So this lede makes no claim about where the key is not —
 * the true claim ("the key is derived, not stored") stops being true the
 * moment a reader allows the biometric skip, and a lock screen is the last
 * place to put a sentence that a preference can falsify.
 */
const LEDE =
  "srelens's stored secrets — the MCP bearer token and your assistant API keys — are sealed on this machine. Unlock to derive the key that opens them.";

/**
 * §25's escalation, verbatim, and the reason the raw refusal is kept beside it.
 *
 * The first two failures say only that the passphrase is wrong; the third and
 * beyond add what a reader on their third try is really asking — whether
 * anything has been given away. Nothing has.
 *
 * A wrong passphrase is not the only way `vault_unlock_password` can refuse
 * (a vault file that will not parse, a keychain that will not answer), and this
 * sentence would be a lie about those. So the sentence is the headline, exactly
 * as §25 writes it, and the string the backend actually sent is offered folded
 * underneath — outside the live region, so what is announced is one sentence
 * and not a Rust struct.
 */
function escalate(failures: number): string {
  const first = "That passphrase is not correct.";
  return failures >= 3 ? `${first} Nothing is unsealed after a failed attempt.` : first;
}

/**
 * The footer: what the cover puts beyond reach, and how many attempts have
 * missed.
 *
 * §25 words the first half "N clusters sealed". The number is a count of kube
 * CONTEXTS, and not one of them is sealed by anything — the vault holds the
 * MCP bearer and the assistant keys, and a kubeconfig is a plain file on disk.
 * The count was right and the word beside it was a claim about encryption that
 * does not exist.
 *
 * It is not dropped, because the number is worth saying: what IS true while
 * this cover is up is that the window those clusters are reached through has
 * been replaced by this one, so they are out of reach until it lifts. That is
 * the fact the reader is looking at, and it is the one the footer states.
 */
function footer(clusters: number, failures: number): string {
  const parts: string[] = [];
  // Omitted at zero rather than rendered as "0 clusters out of reach", which
  // reads as a count that failed. A locked vault is also the state in which the
  // cluster list is least likely to have been read at all.
  if (clusters > 0) parts.push(`${clusters} ${clusters === 1 ? "cluster" : "clusters"} out of reach`);
  parts.push(
    failures === 0
      ? "no failed attempts"
      : `${failures} failed attempt${failures === 1 ? "" : "s"}`,
  );
  return parts.join(" · ");
}

/** A refusal that is not a wrong passphrase, with the sentence that frames it. */
interface Trouble {
  title: string;
  error: unknown;
}

export interface LockGateProps {
  children: ReactNode;
  /**
   * A URL for srelens's brand mark. The host's asset, injected — see
   * {@link BrandLockMark} for why this package cannot import it, and what is
   * drawn when it is absent or will not load.
   */
  brandMarkSrc?: string;
}

export function LockGate({ children, brandMarkSrc }: LockGateProps) {
  // Read once. Every vault command is a Tauri command (`invoke` from
  // `@tauri-apps/api/core`, not the transport layer that has a web half), so in
  // a browser there is no vault to seal and nothing that could unlock one — a
  // cover there would be a browser build with no way in.
  const desktop = useMemo(() => isTauri(), []);
  const raised = useSealed();
  // §25's footer counts what is sealed. The contexts store is what this window
  // knows about, filled by `Window`'s own boot listing — which is also the read
  // most likely to have come back short while the vault is closed, so the
  // footer omits the count at zero rather than reporting one it did not take.
  const clusterCount = useContexts().length;
  // True until the launch read answers. The children stay down for it: a gate
  // that rendered them while the read was in flight would flash the live window
  // over a sealed vault on every launch.
  const [checking, setChecking] = useState(desktop);
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [statusError, setStatusError] = useState<unknown>(null);
  const [passphrase, setPassphrase] = useState("");
  const [repeated, setRepeated] = useState("");
  const [keepRecovery, setKeepRecovery] = useState(true);
  const [revealed, setRevealed] = useState(false);
  const [failures, setFailures] = useState(0);
  const [refusal, setRefusal] = useState<unknown>(null);
  const [trouble, setTrouble] = useState<Trouble | null>(null);
  const [busy, setBusy] = useState<null | "passphrase" | "biometric" | "recovery">(null);
  const [recovered, setRecovered] = useState<string | null>(null);
  // The uninvited biometric sheet fires once per launch, not once per render.
  const autoPrompted = useRef(false);

  const covered = raised || checking;

  /**
   * Read what the backend says, and act on it.
   *
   * `mayOpen` is the whole safety property. Only a read that followed a real
   * unlock attempt — the launch read, an unlock, a biometric unlock, a
   * recovery — is allowed to lower the cover. The reconcile read that follows
   * a deliberate lock is not: a stale or racing `unlocked` there would undo
   * the very thing the reader asked for.
   */
  async function read({ mayOpen }: { mayOpen: boolean }): Promise<VaultStatus | null> {
    try {
      const next = await vaultStatus();
      setStatus(next);
      setStatusError(null);
      rememberMode(next.mode);
      if (next.mode === "unlocked") {
        if (mayOpen) unsealWorkspace();
      } else {
        lockWorkspace();
      }
      return next;
    } catch (error) {
      // **Fails closed.** "The backend did not answer" is not "the vault is
      // open": without this raise a refused launch read left `sealed` false,
      // `checking` went to false behind it, and the window came up live over a
      // vault whose state srelens had never managed to read. `VaultGate` keeps
      // its gate shut for the same reason, in the same words.
      // Forgotten, unlike `status` below: a refusal is not a mode, and a
      // titlebar control offered on the strength of a read that failed would be
      // a control drawn on a guess.
      rememberMode(null);
      lockWorkspace();
      // The last known status is kept rather than dropped. Only a launch read
      // that never answered leaves `status` null, and that is the one case
      // worth replacing the form with "srelens could not reach the vault" —
      // after a lock the mode is already known, and swapping the passphrase
      // field for a retry button would hide the thing that works.
      setStatusError(error);
      return null;
    }
  }

  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    void (async () => {
      const launch = await read({ mayOpen: true });
      if (cancelled) return;
      // Set after the read, not before it: `read` raises the cover in the same
      // tick, and React batches the pair, so there is no frame in which the
      // children are mounted over a sealed vault.
      setChecking(false);
      // The enrolled biometric skip IS the launch unlock — `VaultGate` raises
      // it uninvited for exactly this reason, and cancelling falls back to the
      // passphrase form with no banner, because cancelling a sheet nobody asked
      // for is not a failure and is certainly not a wrong passphrase.
      if (
        launch?.mode === "locked" &&
        launch.biometricEnrolled &&
        launch.biometricAvailable &&
        !autoPrompted.current
      ) {
        autoPrompted.current = true;
        try {
          await vaultBiometricUnlock();
        } catch {
          return;
        }
        if (cancelled) return;
        await read({ mayOpen: true });
      }
    })();
    return () => {
      cancelled = true;
    };
    // Mount only. `desktop` is read once and cannot change for this window.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A cover raised from outside — `Lock now`, or the chord — arrives with no
  // status behind it. This fills in what the tile needs to draw itself (setup
  // versus unlock, and whether there is a biometric skip) AFTER the cover is
  // already up, and cannot lower it.
  useEffect(() => {
    if (!desktop) return;
    if (!raised) return;
    if (status !== null && status.mode !== "unlocked") return;
    void read({ mayOpen: false });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [desktop, raised, status]);

  if (!covered) return <>{children}</>;

  const setup = status?.mode === "setup-required";
  const mismatch = setup && repeated !== "" && repeated !== passphrase;
  const submittable =
    passphrase !== "" &&
    busy === null &&
    (!setup || (passphrase.length >= MIN_PASSPHRASE_LENGTH && repeated === passphrase));

  async function unlock(): Promise<void> {
    setTrouble(null);
    setRefusal(null);
    setBusy("passphrase");
    try {
      await vaultUnlockPassword(passphrase);
    } catch (error) {
      setFailures((n) => n + 1);
      setRefusal(error);
      setBusy(null);
      return;
    }
    // Cleared before the status read, not after: the field is about to be
    // unmounted either way, and a passphrase left in React state over a
    // reconcile that fails is a value sitting in a heap for no reason.
    setPassphrase("");
    setFailures(0);
    setBusy(null);
    await read({ mayOpen: true });
  }

  async function create(): Promise<void> {
    setTrouble(null);
    setRefusal(null);
    setBusy("passphrase");
    try {
      await vaultSetupPassword(passphrase, keepRecovery);
    } catch (error) {
      // Not §25's escalation and not the failure counter: nothing about a
      // refused setup says a passphrase was wrong.
      setTrouble({ title: "The passphrase could not be set", error });
      setBusy(null);
      return;
    }
    setPassphrase("");
    setRepeated("");
    setBusy(null);
    await read({ mayOpen: true });
  }

  async function unlockWithBiometric(): Promise<void> {
    setTrouble(null);
    setRefusal(null);
    setBusy("biometric");
    try {
      await vaultBiometricUnlock();
    } catch (error) {
      setTrouble({ title: `${BIOMETRIC_LABEL} did not unlock the workspace`, error });
      setBusy(null);
      return;
    }
    setBusy(null);
    await read({ mayOpen: true });
  }

  function recover(): void {
    setTrouble(null);
    setRefusal(null);
    setBusy("recovery");
    void vaultRecoverPassword()
      .then((value) => setRecovered(value))
      .catch((error) => setTrouble({ title: "The recovery copy could not be read", error }))
      .finally(() => setBusy(null));
  }

  function submit(): void {
    if (!submittable) return;
    void (setup ? create() : unlock());
  }

  return (
    <Cover>
      {checking && !raised ? (
        <LoadingState label="Checking whether the workspace is sealed" />
      ) : status === null && statusError !== null ? (
        <Unreachable error={statusError} onRetry={() => void read({ mayOpen: true })} />
      ) : recovered !== null ? (
        <Recovered
          value={recovered}
          onContinue={() => {
            setRecovered(null);
            void read({ mayOpen: true });
          }}
        />
      ) : (
        <>
          {/* Centred, and only this block.
              §25 states no alignment; the user, looking at the first render of
              this screen anyone has seen, asked for the mark to be centred, and
              a centred mark over a left-aligned heading is the ragged
              composition they were describing. Its sibling §24 is "centred at
              860 px", so the pair agrees.

              The tile centres because it is `inline-flex` inside a `text-center`
              block, which is also what centres the heading and the lede. The
              lede is a judgement call: three lines with an em-dash aside read
              better ragged-right, but a left-aligned paragraph directly under a
              centred heading in a 26rem card looks like a mistake, and the
              block reading as one thing beats the paragraph reading marginally
              faster.

              What is NOT centred, deliberately, is everything below: the field's
              eyebrow sits opposite `Show passphrase` as a row and centring
              would break the pair; the `role="alert"` error keeps its own
              layout, its shake and the escalation wording tests pin; and the
              raw refusal, the failure alerts, the repeat field and the recovery
              checkbox are all left-aligned prose that a centre would only make
              harder to read. */}
          <header className="text-center">
            <BrandLockMark src={brandMarkSrc} />
            <h1 className="mt-3 text-[1.375rem] font-semibold tracking-[-0.01em]">
              {setup ? "Protect your workspace" : "Workspace locked"}
            </h1>
            <p className="mt-2 text-[0.8125rem] leading-relaxed text-muted">
              {setup
                ? "srelens seals its stored secrets — the MCP bearer token and your assistant API keys — with a key derived from a master passphrase you choose. Your kubeconfigs are not sealed: srelens reads them as ordinary files. The key is derived from the passphrase rather than kept, so srelens asks for it every time it starts."
                : LEDE}
            </p>
          </header>

          <div className="mt-5">
            <Field
              label="Master passphrase"
              action={
                <button
                  type="button"
                  className="text-[0.6875rem] text-muted underline decoration-dotted underline-offset-2"
                  onClick={() => setRevealed((on) => !on)}
                >
                  {revealed ? "Hide passphrase" : "Show passphrase"}
                </button>
              }
              hint={setup ? `At least ${MIN_PASSPHRASE_LENGTH} characters.` : undefined}
            >
              <TextInput
                // `Field` with an `action` renders the label as a sibling
                // `span` rather than wrapping the control (a `button` inside a
                // `label` would swallow its own name), so the control needs a
                // name of its own. §25 puts the reveal toggle beside the label,
                // which is what makes that the right trade here.
                aria-label="Master passphrase"
                type={revealed ? "text" : "password"}
                autoFocus
                value={passphrase}
                onValueChange={setPassphrase}
                onEnter={submit}
                // §25's placeholder. Bullets, deliberately — a hint here that
                // named anything would be a hint about a credential.
                placeholder="••••••••"
              />
            </Field>

            {setup && (
              <>
                <Field
                  label="Repeat the passphrase"
                  error={mismatch ? "The two entries do not match." : undefined}
                  hint="A mistyped passphrase would seal the vault behind something you never typed on purpose."
                >
                  <TextInput
                    // Named explicitly, like the field above it. `Field`
                    // without an `action` does wrap the control in its
                    // `<label>`, but the hint below it is inside that label
                    // too, so the computed name would be the label and the
                    // hint run together.
                    aria-label="Repeat the passphrase"
                    type={revealed ? "text" : "password"}
                    value={repeated}
                    onValueChange={setRepeated}
                    onEnter={submit}
                    invalid={mismatch === true}
                  />
                </Field>
                <div className="mt-1">
                  <Checkbox
                    checked={keepRecovery}
                    onChange={setKeepRecovery}
                    label="Keep a recovery copy in this machine's keychain"
                  />
                </div>
              </>
            )}

            {/* The live region, mounted whether or not it has anything in it.
                An `alert` announces reliably only when it was already in the
                document before the text went into it — a region mounted with
                its own message may be read as ordinary text. Empty it is
                invisible: no border, no glyph, no space of its own. */}
            <p
              role="alert"
              // `--sev` from the token axis, and the kit's own shake. Keyed on
              // the count so a second wrong attempt shakes again rather than
              // sitting still under an animation that already ran.
              key={failures}
              className={failures > 0 ? "shake mt-2 text-[0.8125rem]" : undefined}
              style={failures > 0 ? { color: "var(--sev)" } : undefined}
            >
              {failures > 0 ? escalate(failures) : ""}
            </p>
            {refusal !== null && (
              // Outside the live region on purpose: what is announced is §25's
              // sentence, and this is the string the backend sent, for the
              // reader whose refusal was not a wrong passphrase after all.
              <RawError text={friendly(refusal).raw ?? friendly(refusal).detail} className="mt-1" />
            )}

            <Button
              variant="primary"
              className="mt-3 w-full"
              disabled={!submittable}
              onClick={submit}
            >
              {busy === "passphrase" ? (
                <span className="flex items-center justify-center gap-2">
                  <Spinner className="size-3.5" />
                  {setup ? "Sealing the workspace…" : "Deriving key…"}
                </span>
              ) : setup ? (
                "Create passphrase"
              ) : (
                "Unlock workspace"
              )}
            </Button>

            {trouble !== null && (
              <FailureAlert
                // `sev`: the reader asked for something and did not get it.
                tone="sev"
                title={trouble.title}
                error={trouble.error}
                className="mt-3"
              />
            )}

            {/* Only where the skip is actually enrolled AND the machine has a
                sensor — §25 draws the panel unconditionally, and a Touch ID
                button on a machine with no Touch ID is a control that can only
                fail. Never on the setup form: there is no key to unwrap yet. */}
            {!setup && status?.biometricEnrolled === true && status.biometricAvailable && (
              <>
                <Divider />
                <Button
                  variant="secondary"
                  className="w-full"
                  disabled={busy !== null}
                  onClick={() => void unlockWithBiometric()}
                >
                  {busy === "biometric" ? (
                    <span className="relative flex items-center justify-center gap-2">
                      <span className="relative inline-flex size-3.5 items-center justify-center">
                        <span className="ring-pulse" style={{ borderColor: "var(--accent)" }} />
                      </span>
                      {`Waiting for ${BIOMETRIC_LABEL}…`}
                    </span>
                  ) : (
                    `Unlock with ${BIOMETRIC_LABEL}`
                  )}
                </Button>
              </>
            )}

            {!setup && (
              <div className="mt-3 text-center">
                <button
                  type="button"
                  className="text-[0.6875rem] text-muted underline decoration-dotted underline-offset-2"
                  disabled={busy !== null}
                  onClick={recover}
                >
                  Forgot your passphrase?
                </button>
              </div>
            )}
          </div>

          {!setup && (
            // Centred with the header, not with the body: one short line under
            // a full-width button, which is the composition's closing note
            // rather than something to read along.
            <div className="mt-5 border-t border-rule pt-3 text-center">
              <Eyebrow>{footer(clusterCount, failures)}</Eyebrow>
            </div>
          )}
        </>
      )}
    </Cover>
  );
}

/**
 * §25's first bullet — "a dark rounded lock tile" — as the app's own mark.
 *
 * **It was rendering as an empty pale box, and the cause was a token that does
 * not exist.** The tile drew its own inline `<svg>` whose two strokes were
 * `var(--muted)`. There is no `--muted` in `packages/ui-kit/src/styles/
 * tokens.css` — the token is `--ink-muted` — so both strokes resolved to an
 * invalid value, neither path painted, and what reached the screen was a
 * rounded `--surface-sunk` rectangle with nothing in it: a failed image rather
 * than a mark. It was the only surface in the app nobody had ever seen
 * rendered (#372: `scripts/screenshot.mjs` drives web mode, where this gate
 * deliberately never raises), and those were the only two `var(--muted)` in the
 * whole source tree.
 *
 * **THE MARK ALONE, AND NO PADLOCK — A DELIBERATE DEVIATION FROM §25.** §25
 * asks for "a dark rounded lock tile". There is no lock in this tile, and that
 * is a decision rather than an oversight: `<h1>Workspace locked` sits eighteen
 * pixels beneath it and says the state in words, so a padlock here repeats in a
 * picture exactly what the heading already carries. The mark identifies the
 * application; the heading carries the state; two glyphs were competing for one
 * job. **Do not add a lock back to this tile without reading the rest of this
 * paragraph.**
 *
 * Four arrangements went in front of the only reviewer this surface has — #372
 * means nobody else can see it — and only the last was accepted. An empty box
 * (the `--muted` bug above). A padlock alone, which said the state twice and
 * identified nothing. The brand mark with a padlock beside it, which read as two
 * unrelated icons that happened to be adjacent: a full-colour hexagon and a
 * thin monochrome outline, with a gap and nothing relating them. And the mark
 * on its own, which is this.
 *
 * **`Mark`'s `withBadge` could not have fixed that**, which is worth writing
 * down because it looks as though it should. It rides `short` — TEXT, the
 * initials, uppercased — along the bottom edge of a glyph or image mark, so it
 * can badge a mark with letters and not with a padlock. Corner-badging a state
 * glyph would be a new kit API, and that is a design-system decision rather
 * than something to settle inside a lock screen.
 *
 * **The kit's `Mark`, not a hand-rolled `<img>`.** It is what the cluster rail
 * draws its squares with, and it owns every property this tile had hand-rolled:
 * the rounded square, a centred image or fallback, colours from tokens, and a
 * name or `aria-hidden`.
 *
 * **The asset is the host's and is injected.** `apps/desktop/src/assets/
 * srelens-mark.svg` is the mark classic's landing page and login screen already
 * import; this package cannot reach it (`apps/desktop` depends on ui-next, so
 * the import would be a cycle), and a literal `/srelens-mark.svg` would be a
 * host path hardcoded into a package that must not know the host — right in the
 * desktop app and wrong in the kit's gallery. It arrives down the path `ported`
 * and `onSwitchToClassic` already travel: `main.tsx` -> `NextApp` -> `Window` ->
 * here.
 *
 * **A missing or broken image degrades, and never to an empty box.** With no
 * `src` — the gallery, a host that passes none — or an image that will not
 * load, `Mark` falls through to the initials derived from its name, on the
 * accent, in `--surface` ink. Its own words: "an image that will not load is a
 * state, not an error to report". That fallback is why this is a `Mark`.
 *
 * **Colour.** Nothing here names one. The fallback's two colours are `--accent`
 * and `--surface`, `Mark`'s own, so it follows the reader's accent rather than
 * assuming Violet. The brand SVG carries its own gradients — violet through
 * magenta to orange, over a dark glass hexagon — and that is the one place a
 * baked colour is right rather than a token violation: a brand mark that
 * re-coloured per theme would not be the brand mark. Being self-contained, with
 * a dark interior and a bright stroke, it reads on the light grounds (Light,
 * Paper, High contrast) and the dark ones (Dark, Midnight) alike.
 */
function BrandLockMark({ src }: { src?: string }) {
  return (
    // Decorative: the `<h1>` directly beneath says "Workspace locked", and
    // `Mark`'s own note is that an unnamed `role="img"` announces as "image"
    // and tells the listener nothing. A second "srelens" would be no better —
    // the window's titlebar is already named.
    <span data-testid="lock-mark" className="inline-flex">
      <Mark
        name="srelens"
        imageSrc={src}
        size="lg"
        // The badge rides the initials along the bottom edge of the brand mark,
        // which is the app's name twice.
        withBadge={false}
        decorative
      />
    </span>
  );
}

/**
 * §25's dark rounded tile on §24's decorative ground.
 *
 * `absolute inset-0` inside the band the window gives it rather than
 * `fixed inset-0`: the titlebar and the status bar are not part of what §25
 * replaces, and a fixed cover would take the window's own chrome with it.
 * `role="dialog"` with its own label says what this is to assistive technology.
 *
 * **No `aria-modal`, and that is a correction.** It carried `aria-modal="true"`
 * — "there is nothing else on this window to reach" — while four controls in
 * the titlebar deliberately stayed in the tab order: the theme toggle and the
 * three zoom buttons, which are kept usable because a reader who cannot read
 * the passphrase field cannot unlock (see `Chrome`). Assistive technology was
 * being told those did not exist while the Tab key went straight to them, and
 * of the two the Tab key is the one that is right. Everything else on the
 * chrome stands down while this is up — the switcher, the gear, every status
 * segment — so the four that remain are the whole of what the markup now has
 * to admit to.
 */
function Cover({ children }: { children: ReactNode }) {
  return (
    <div
      role="dialog"
      aria-label="Workspace locked"
      data-testid="lock-cover"
      className="absolute inset-0 z-50 flex min-h-0 items-center justify-center overflow-auto p-6"
      style={{
        background:
          "radial-gradient(circle at 15% 8%, color-mix(in srgb, var(--accent) 7%, transparent), transparent 30%), var(--canvas-deep)",
      }}
    >
      <div
        className="rise w-full max-w-[26rem] rounded-xl p-6"
        style={{
          background: "var(--surface)",
          border: "1px solid var(--rule)",
          boxShadow: "0 18px 48px color-mix(in srgb, var(--canvas-deep) 70%, transparent)",
        }}
      >
        {children}
      </div>
    </div>
  );
}

/**
 * The vault's own state could not be read. `VaultGate` reaches the same state
 * the same way and for the same reason: the gate must STAY CLOSED with a retry
 * rather than quietly wave the window through, because "the backend did not
 * answer" is not "the vault is open".
 */
function Unreachable({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  return (
    <>
      <h1 className="text-[1.375rem] font-semibold tracking-[-0.01em]">Secrets unavailable</h1>
      <p className="mt-2 text-[0.8125rem] leading-relaxed text-muted">
        srelens could not reach its secrets vault, so it cannot tell whether the workspace is
        sealed. This cover stays up until that is answered — retry, or restart srelens.
      </p>
      <FailureAlert
        tone="sev"
        title="The vault did not answer"
        error={error}
        className="mt-3"
      />
      <Button variant="primary" className="mt-3 w-full" onClick={onRetry}>
        Retry
      </Button>
    </>
  );
}

/**
 * The keychain recovery copy, printed because the reader asked for it.
 *
 * The vault is already unlocked by the time this renders — `vault_recover_password`
 * opens it as a side effect — so the cover is staying up purely so the reader
 * can read this before it goes. Ported from `VaultGate` unchanged in substance:
 * the one place in srelens that prints a passphrase does it because the reader
 * explicitly asked for it and has nowhere else to read it from.
 *
 * **Not "shown once".** §25's line is "This is the only time srelens shows
 * it", and that is false: `recover_password_core`
 * (`apps/desktop/src-tauri/src/vault_password.rs:212-246`) reads the same
 * keychain copy and returns the same value every time `Forgot your
 * passphrase?` is pressed, with no credential typed at all — the marker file
 * is the only gate. So the copy says what is actually true, and says the part
 * that matters more than either: a reader who kept a recovery copy has a
 * passphrase that whatever can read that keychain can read too.
 */
function Recovered({ value, onContinue }: { value: string; onContinue: () => void }) {
  return (
    <>
      <h1 className="text-[1.375rem] font-semibold tracking-[-0.01em]">Workspace unsealed</h1>
      <Eyebrow className="mt-4">Your master passphrase</Eyebrow>
      <code
        className="mt-1 block rounded px-2 py-1.5 font-mono text-[0.8125rem]"
        style={{ background: "var(--surface-sunk)", border: "1px solid var(--rule)" }}
      >
        {value}
      </code>
      <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
        Note it somewhere safe, or change it under Settings → Security. srelens shows it nowhere
        else — but this screen reads it back from the recovery copy in this machine&apos;s keychain
        every time you ask, so anything that can read that keychain can read this.
      </p>
      <Button variant="primary" className="mt-4 w-full" onClick={onContinue}>
        Continue
      </Button>
    </>
  );
}

/** §25's `or` between the passphrase and the biometric skip. */
function Divider() {
  return (
    <div className="my-3 flex items-center gap-2" aria-hidden="true">
      <span className="h-px flex-1" style={{ background: "var(--rule)" }} />
      <span className="text-[0.6875rem] text-faint">or</span>
      <span className="h-px flex-1" style={{ background: "var(--rule)" }} />
    </div>
  );
}
