import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  vaultLock: vi.fn(),
  vaultChangePassword: vi.fn(),
  vaultBiometricStatus: vi.fn(),
  vaultBiometricEnable: vi.fn(),
  vaultBiometricDisable: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { setNotifier, type VaultBiometricStatus } from "@srelens/core";
import { MIN_PASSPHRASE_LENGTH, SecurityPane } from "./SecurityPane";

/**
 * `vaultBiometricStatus` returns three independent booleans
 * (`apps/desktop/src-tauri/src/vault_biometric.rs`), so the fixtures below keep
 * them independent: a sensor that exists with the gate OFF, the same sensor
 * with it ON, and no sensor at all. One fixture with everything true would let
 * the pane read any field it liked — including the wrong one — and still pass.
 */
const SENSOR_OFF: VaultBiometricStatus = { available: true, enabled: false, unlocked: true };
const SENSOR_ON: VaultBiometricStatus = { available: true, enabled: true, unlocked: true };
const NO_SENSOR: VaultBiometricStatus = { available: false, enabled: false, unlocked: true };

/**
 * Two values that share no substring, so a swapped argument pair cannot pass
 * for the right one. Never named in a test title, and neither is a passphrase
 * anyone could type anywhere real.
 */
const HELD_NOW = "aaaa1111aaaa";
const WANTED = "bbbb2222bbbb";

async function openTheChangeForm() {
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: /change passphrase/i }));
  return user;
}

describe("SecurityPane", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.vaultLock.mockResolvedValue(undefined);
    core.vaultChangePassword.mockResolvedValue(null);
    core.vaultBiometricStatus.mockResolvedValue(SENSOR_OFF);
    core.vaultBiometricEnable.mockResolvedValue(undefined);
    core.vaultBiometricDisable.mockResolvedValue(undefined);
  });

  it("locks the vault and tells the shell to cover the window", async () => {
    const onLocked = vi.fn();
    render(<SecurityPane onLocked={onLocked} />);
    await userEvent.setup().click(await screen.findByRole("button", { name: /lock now/i }));
    expect(core.vaultLock).toHaveBeenCalled();
    expect(onLocked).toHaveBeenCalled();
  });

  it("does not cover the window if locking failed", async () => {
    const onLocked = vi.fn();
    core.vaultLock.mockRejectedValue(new Error("vault_lock failed: no master password is set"));
    render(<SecurityPane onLocked={onLocked} />);
    await userEvent.setup().click(await screen.findByRole("button", { name: /lock now/i }));
    expect(await screen.findByText(/could not/i)).toBeTruthy();
    expect(onLocked).not.toHaveBeenCalled();
  });

  /**
   * A vault refusal has no branch in `describeError` — it is the generic case,
   * whose detail IS the message — so the thing worth pinning is that the
   * message went THROUGH it: `cleanErrorMessage` strips the `handler error:`
   * and `Error:` wrappers that `String(e)` would leave in front of the
   * sentence the backend actually wrote.
   */
  it("reports a failed lock through describeError, not through String(e)", async () => {
    core.vaultLock.mockRejectedValue(
      new Error("handler error: no master password is set, so there would be nothing to unlock with"),
    );
    render(<SecurityPane onLocked={() => {}} />);
    await userEvent.setup().click(await screen.findByRole("button", { name: /lock now/i }));
    const alert = await screen.findByRole("alert");
    const body = alert.querySelector("[data-slot=alert-body]");
    expect(body).not.toBeNull();
    expect(body?.textContent ?? "").toContain("no master password is set");
    expect(body?.textContent ?? "").not.toContain("handler error");
  });

  it("keeps the design's sentence about what the key seals", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    expect(
      await screen.findByText(
        /Kubeconfigs and cluster tokens are sealed at rest with a key derived from your master passphrase/i,
      ),
    ).toBeTruthy();
  });

  /**
   * The correction this pane exists to get right. Locking discards the vault
   * key and nothing else: a bearer token the MCP server has already issued,
   * and cluster clients already built, survive until restart (verified while
   * `vaultLock` was built). §23's paragraph ends "nothing is readable again
   * until you unlock", and that claim must not be on this screen.
   */
  it("does not claim that locking stops what is already open", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    await screen.findByRole("button", { name: /lock now/i });
    expect(screen.queryByText(/nothing is readable/i)).toBeNull();
    expect(screen.queryByText(/nothing.*readable.*until you unlock/i)).toBeNull();
    expect(screen.getByText(/does not close what is already open/i)).toBeTruthy();
  });

  it("asks for the current passphrase as well as the new one", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    await openTheChangeForm();
    expect(screen.getByLabelText(/current passphrase/i)).toBeTruthy();
    expect(screen.getByLabelText(/new passphrase/i)).toBeTruthy();
  });

  it("passes the held value and the wanted one in that order", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    const user = await openTheChangeForm();
    await user.type(screen.getByLabelText(/current passphrase/i), HELD_NOW);
    await user.type(screen.getByLabelText(/new passphrase/i), WANTED);
    await user.type(screen.getByLabelText(/repeat/i), WANTED);
    await user.click(screen.getByRole("button", { name: /set the new passphrase/i }));
    expect(core.vaultChangePassword).toHaveBeenCalledWith(HELD_NOW, WANTED);
  });

  it("submits on Enter, under the same rule the button follows", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    const user = await openTheChangeForm();
    await user.type(screen.getByLabelText(/current passphrase/i), HELD_NOW);
    // Enter with the repeat still empty must do nothing: the same half-typed
    // state the disabled button refuses.
    await user.type(screen.getByLabelText(/new passphrase/i), `${WANTED}{Enter}`);
    expect(core.vaultChangePassword).not.toHaveBeenCalled();
    await user.type(screen.getByLabelText(/repeat/i), `${WANTED}{Enter}`);
    expect(core.vaultChangePassword).toHaveBeenCalledWith(HELD_NOW, WANTED);
  });

  it("will not submit a repeat that does not match", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    const user = await openTheChangeForm();
    await user.type(screen.getByLabelText(/current passphrase/i), HELD_NOW);
    await user.type(screen.getByLabelText(/new passphrase/i), WANTED);
    await user.type(screen.getByLabelText(/repeat/i), `${WANTED}x`);
    const submit = screen.getByRole("button", {
      name: /set the new passphrase/i,
    }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    expect(screen.getByText(/do not match/i)).toBeTruthy();
    await user.click(submit);
    expect(core.vaultChangePassword).not.toHaveBeenCalled();
  });

  it("will not submit a new value the backend would refuse as too short", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    const user = await openTheChangeForm();
    const tooShort = "c".repeat(MIN_PASSPHRASE_LENGTH - 1);
    await user.type(screen.getByLabelText(/current passphrase/i), HELD_NOW);
    await user.type(screen.getByLabelText(/new passphrase/i), tooShort);
    await user.type(screen.getByLabelText(/repeat/i), tooShort);
    expect(
      (screen.getByRole("button", { name: /set the new passphrase/i }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("never puts what was typed anywhere it could be read", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    const user = await openTheChangeForm();
    for (const label of [/current passphrase/i, /new passphrase/i, /repeat/i]) {
      const input = screen.getByLabelText(label) as HTMLInputElement;
      await user.type(input, HELD_NOW);
      expect(input.type).toBe("password");
    }
    expect(screen.queryByText(HELD_NOW)).toBeNull();
    for (const el of Array.from(document.querySelectorAll("*"))) {
      for (const attr of ["title", "aria-label", "placeholder", "aria-describedby"]) {
        expect(el.getAttribute(attr) ?? "").not.toContain(HELD_NOW);
      }
    }
  });

  it("says the change landed, and closes the form", async () => {
    const success = vi.fn();
    const restore = setNotifier({
      success,
      error: () => {},
      info: () => {},
      updateAvailable: () => {},
      clusterSignIn: () => {},
    });
    try {
      render(<SecurityPane onLocked={() => {}} />);
      const user = await openTheChangeForm();
      await user.type(screen.getByLabelText(/current passphrase/i), HELD_NOW);
      await user.type(screen.getByLabelText(/new passphrase/i), WANTED);
      await user.type(screen.getByLabelText(/repeat/i), WANTED);
      await user.click(screen.getByRole("button", { name: /set the new passphrase/i }));
      expect(await screen.findByRole("button", { name: /change passphrase/i })).toBeTruthy();
      expect(screen.queryByLabelText(/current passphrase/i)).toBeNull();
      expect(success).toHaveBeenCalled();
    } finally {
      restore();
    }
  });

  /**
   * `vaultChangePassword` resolves to a STRING when the change landed but the
   * biometric enrollment had to be turned off (its doc comment,
   * `packages/core/src/lib/mcpSecurity.ts`). A pane that dropped that would
   * leave the reader believing an unlock method still works.
   */
  it("surfaces a change that turned biometric unlock off", async () => {
    core.vaultChangePassword.mockResolvedValue("biometric unlock was turned off");
    render(<SecurityPane onLocked={() => {}} />);
    const user = await openTheChangeForm();
    await user.type(screen.getByLabelText(/current passphrase/i), HELD_NOW);
    await user.type(screen.getByLabelText(/new passphrase/i), WANTED);
    await user.type(screen.getByLabelText(/repeat/i), WANTED);
    await user.click(screen.getByRole("button", { name: /set the new passphrase/i }));
    expect(await screen.findByText(/biometric unlock was turned off/i)).toBeTruthy();
  });

  it("reports a refused change without closing the form", async () => {
    core.vaultChangePassword.mockRejectedValue(new Error("the current password is not correct"));
    render(<SecurityPane onLocked={() => {}} />);
    const user = await openTheChangeForm();
    await user.type(screen.getByLabelText(/current passphrase/i), HELD_NOW);
    await user.type(screen.getByLabelText(/new passphrase/i), WANTED);
    await user.type(screen.getByLabelText(/repeat/i), WANTED);
    await user.click(screen.getByRole("button", { name: /set the new passphrase/i }));
    expect(await screen.findByText(/could not/i)).toBeTruthy();
    expect(screen.getByLabelText(/current passphrase/i)).toBeTruthy();
  });

  it("draws the biometric switch from the reported gate state, not from the sensor", async () => {
    core.vaultBiometricStatus.mockResolvedValue(SENSOR_ON);
    render(<SecurityPane onLocked={() => {}} />);
    const toggle = (await screen.findByRole("switch")) as HTMLButtonElement;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await userEvent.setup().click(toggle);
    expect(core.vaultBiometricDisable).toHaveBeenCalled();
    expect(core.vaultBiometricEnable).not.toHaveBeenCalled();
  });

  it("enables the gate when it is off", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    const toggle = (await screen.findByRole("switch")) as HTMLButtonElement;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    await userEvent.setup().click(toggle);
    expect(core.vaultBiometricEnable).toHaveBeenCalled();
    expect(core.vaultBiometricDisable).not.toHaveBeenCalled();
  });

  it("offers nothing to allow when the machine has no sensor, and says why", async () => {
    core.vaultBiometricStatus.mockResolvedValue(NO_SENSOR);
    render(<SecurityPane onLocked={() => {}} />);
    expect(await screen.findByText(/no biometric sensor/i)).toBeTruthy();
    expect(screen.queryByRole("switch")).toBeNull();
  });

  /**
   * The defect this branch has now hit twice: a read that failed leaving a
   * boolean at its default, which the render then states as fact. A refused
   * status must claim NEITHER that biometric unlock is available nor that it
   * is missing.
   */
  it("claims nothing about biometric unlock when the status could not be read", async () => {
    core.vaultBiometricStatus.mockRejectedValue(new Error("vault_biometric_status failed"));
    render(<SecurityPane onLocked={() => {}} />);
    expect(await screen.findByText(/could not be read/i)).toBeTruthy();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByText(/no biometric sensor/i)).toBeNull();
  });

  it("says the idle lock is not built rather than leaving it to be looked for", async () => {
    render(<SecurityPane onLocked={() => {}} />);
    expect(
      await screen.findByText(/does not lock (itself )?when idle|no idle lock/i),
    ).toBeTruthy();
    expect(screen.queryByRole("switch", { name: /lock on launch/i })).toBeNull();
    expect(screen.queryByRole("combobox", { name: /lock when idle/i })).toBeNull();
    expect(screen.queryByText(/15 minutes/i)).toBeNull();
  });

  /**
   * §23 draws a `⌘⇧L` kbd beside `Lock now`, and no such chord is bound:
   * `BINDINGS` in `packages/ui-next/src/lib/shortcuts.ts` has no `Mod+Shift+L`
   * row, so the hint would promise a key that does nothing. Whoever binds it
   * should fail this test and delete it in the same commit.
   */
  it("promises no keyboard shortcut for locking, because none is bound", async () => {
    const { container } = render(<SecurityPane onLocked={() => {}} />);
    await screen.findByRole("button", { name: /lock now/i });
    expect(container.textContent ?? "").not.toContain("⌘");
    expect(container.querySelector("kbd")).toBeNull();
  });
});
