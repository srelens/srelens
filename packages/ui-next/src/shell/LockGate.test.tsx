import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UserEvent } from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  vaultStatus: vi.fn(),
  vaultUnlockPassword: vi.fn(),
  vaultSetupPassword: vi.fn(),
  vaultRecoverPassword: vi.fn(),
  vaultBiometricUnlock: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import type { VaultStatus } from "@srelens/core";
import { LockGate, lockWorkspace, resetLock } from "./LockGate";
import { resetContexts, setContexts } from "../lib/clusters";

/**
 * The four vault modes this gate has to tell apart, kept as independent
 * fixtures. One "everything true" status would let the gate read whichever
 * field it liked — `biometricAvailable` where it meant `biometricEnrolled`,
 * say — and still pass every test in the file.
 */
const OPEN: VaultStatus = {
  mode: "unlocked",
  keySource: "password",
  biometricAvailable: true,
  biometricEnrolled: false,
};
const SEALED: VaultStatus = {
  mode: "locked",
  keySource: "password-locked",
  biometricAvailable: true,
  biometricEnrolled: false,
};
const SEALED_WITH_BIOMETRIC: VaultStatus = {
  mode: "locked",
  keySource: "biometric-locked",
  biometricAvailable: true,
  biometricEnrolled: true,
};
const NEVER_SET_UP: VaultStatus = {
  mode: "setup-required",
  keySource: "locked",
  biometricAvailable: false,
  biometricEnrolled: false,
};

/**
 * A string that is not anyone's passphrase, never named in a test title and
 * never asserted on. What is typed here only has to be refused.
 */
const TYPED = "not-the-one";

const ctx = (stableId: string) => ({
  name: stableId,
  stableId,
  cluster: stableId,
  server: "",
  isCurrent: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
});

/**
 * What the gate covers, drawn the way the window draws it. The tab strip and
 * the cluster rail are inside the children on purpose: §25 replaces the whole
 * middle band, and a test that only checked for the tile would pass over a
 * cover that left the strip and the rail live above it.
 */
function Behind() {
  return (
    <div data-testid="body">
      <div role="tablist" aria-label="Open tabs" />
      <nav aria-label="Clusters" />
    </div>
  );
}

function paint() {
  render(
    <LockGate>
      <Behind />
    </LockGate>,
  );
}

function field(): HTMLElement {
  return screen.getByLabelText("Master passphrase");
}

async function failOnce(user: UserEvent) {
  await user.clear(field());
  await user.type(field(), TYPED);
  await user.click(screen.getByRole("button", { name: "Unlock workspace" }));
}

beforeEach(() => {
  vi.clearAllMocks();
  core.isTauri.mockReturnValue(true);
  core.vaultStatus.mockResolvedValue(SEALED);
  core.vaultUnlockPassword.mockRejectedValue(new Error("that is not the passphrase"));
  core.vaultSetupPassword.mockResolvedValue(undefined);
  core.vaultRecoverPassword.mockResolvedValue("recovered-value");
  core.vaultBiometricUnlock.mockResolvedValue(undefined);
  resetLock();
  resetContexts();
});

describe("LockGate — the cover", () => {
  it("covers the whole window, tab strip and cluster rail included", async () => {
    paint();
    expect(await screen.findByText("Workspace locked")).toBeTruthy();
    expect(screen.queryByTestId("body")).toBeNull();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Clusters" })).toBeNull();
  });

  it("shows the window once the vault is open", async () => {
    core.vaultStatus.mockResolvedValue(OPEN);
    paint();
    expect(await screen.findByTestId("body")).toBeTruthy();
    expect(screen.queryByText("Workspace locked")).toBeNull();
    // The positive control for the test above: these are the very queries that
    // came back null there, so they have to find something here.
    expect(screen.getByRole("tablist")).toBeTruthy();
    expect(screen.getByRole("navigation", { name: "Clusters" })).toBeTruthy();
  });

  it("does not show the window while it is still deciding whether to ask", () => {
    // A status read that never settles. A gate that rendered its children
    // while the read was in flight would flash the live window over a sealed
    // vault on every launch — briefly, and for exactly as long as the backend
    // takes to answer.
    core.vaultStatus.mockReturnValue(new Promise<VaultStatus>(() => {}));
    paint();
    expect(screen.queryByTestId("body")).toBeNull();
    expect(screen.queryByRole("tablist")).toBeNull();
  });

  it("stays covered when the vault's own state cannot be read", async () => {
    core.vaultStatus.mockRejectedValue(new Error("vault state unavailable"));
    paint();
    expect(await screen.findByText("Secrets unavailable")).toBeTruthy();
    expect(screen.queryByTestId("body")).toBeNull();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    // "Your clusters stay sealed until this resolves" was the wrong claim in
    // the wrong direction: nothing seals a cluster, and what is actually true
    // is that this cover is not coming down.
    expect(document.body.textContent ?? "").not.toMatch(/clusters stay sealed/i);
  });

  it("asks the vault nothing where there is no vault to ask", async () => {
    // Web mode: every vault command is a Tauri command, so there is nothing to
    // unlock and a cover here would brick the browser build with no way in.
    core.isTauri.mockReturnValue(false);
    paint();
    expect(await screen.findByTestId("body")).toBeTruthy();
    expect(core.vaultStatus).not.toHaveBeenCalled();
  });
});

describe("LockGate — raising the cover", () => {
  it("covers before it has asked the vault anything, and stays covered", async () => {
    core.vaultStatus.mockResolvedValue(OPEN);
    paint();
    await screen.findByTestId("body");
    // The reconcile read never settles, so nothing below can be waiting on it.
    // An await between the lock landing and the cover going up is a window of
    // live UI over a sealed vault.
    core.vaultStatus.mockReturnValue(new Promise<VaultStatus>(() => {}));
    act(() => lockWorkspace());
    expect(screen.getByText("Workspace locked")).toBeTruthy();
    expect(screen.queryByTestId("body")).toBeNull();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Clusters" })).toBeNull();
  });

  it("is idempotent, because Lock now can be double-clicked", async () => {
    core.vaultStatus.mockResolvedValue(OPEN);
    paint();
    await screen.findByTestId("body");
    act(() => {
      lockWorkspace();
      lockWorkspace();
      lockWorkspace();
    });
    expect(screen.getAllByRole("heading", { name: "Workspace locked" })).toHaveLength(1);
    expect(screen.queryByTestId("body")).toBeNull();
  });

  it("does not lower a cover the reader asked for, whatever the vault says next", async () => {
    core.vaultStatus.mockResolvedValue(OPEN);
    paint();
    await screen.findByTestId("body");
    // The reconcile read still reports `unlocked` — a stale answer, or a lock
    // that did not take. Either way the cover was raised deliberately and only
    // an unlock may lower it.
    act(() => lockWorkspace());
    await waitFor(() => expect(core.vaultStatus).toHaveBeenCalledTimes(2));
    expect(screen.getByText("Workspace locked")).toBeTruthy();
    expect(screen.queryByTestId("body")).toBeNull();
  });

  it("lowers the cover once the passphrase opens the vault", async () => {
    const user = userEvent.setup();
    paint();
    await screen.findByText("Workspace locked");
    core.vaultUnlockPassword.mockResolvedValue(undefined);
    core.vaultStatus.mockResolvedValue(OPEN);
    await user.type(field(), TYPED);
    await user.click(screen.getByRole("button", { name: "Unlock workspace" }));
    expect(await screen.findByTestId("body")).toBeTruthy();
    expect(screen.queryByText("Workspace locked")).toBeNull();
  });
});

describe("LockGate — §25's copy", () => {
  /**
   * §25's lede was "Your kubeconfigs and cluster tokens are sealed on this
   * machine. Unlock to derive the key — it is never written to disk." Both
   * halves were false. `Secrets` (`apps/desktop/src-tauri/src/vault.rs:43-50`)
   * holds `mcp_token` and `llm_keys` and nothing else; a kubeconfig is a plain
   * file. And `vault_biometric_enable` (`vault_biometric.rs:75-81`) writes
   * `to_hex(&key)` into the platform biometric store, so "never written to
   * disk" was contradicted by the `Unlock with Touch ID` button directly
   * underneath it.
   *
   * The old test's NAME claimed both properties; its body only checked that
   * one particular string was on screen, so the string being false cost
   * nothing. This one checks the properties.
   */
  it("names what is sealed, and claims nothing about where the key is not", async () => {
    paint();
    await screen.findByText("Workspace locked");
    const text = document.body.textContent ?? "";
    expect(text).toMatch(/MCP bearer token/i);
    expect(text).toMatch(/assistant API keys/i);
    expect(text).not.toMatch(/kubeconfigs[^.]*\bare sealed\b/i);
    expect(text).not.toMatch(/seals?\s+(your\s+)?kubeconfigs/i);
    expect(text).not.toMatch(/cluster tokens are sealed/i);
    expect(text).not.toMatch(/never written/i);
  });

  /**
   * The direct contradiction, in one viewport: the claim and the button that
   * refutes it. Asserted with the biometric skip actually enrolled, which is
   * the state §25's lede shipped above.
   */
  it("makes no claim the Touch ID button standing under it refutes", async () => {
    core.vaultStatus.mockResolvedValue(SEALED_WITH_BIOMETRIC);
    core.vaultBiometricUnlock.mockRejectedValue(new Error("the user cancelled"));
    paint();
    await screen.findByText("Workspace locked");
    await waitFor(() => expect(core.vaultBiometricUnlock).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: /Unlock with Touch ID/ })).toBeTruthy();
    const text = document.body.textContent ?? "";
    expect(text).not.toMatch(/never written/i);
    expect(text).not.toMatch(/memory only/i);
  });

  /**
   * The setup form's own paragraph, which said "srelens seals your kubeconfigs
   * and cluster tokens with a key derived from a master passphrase you
   * choose." Same falsehood as the lede, on the one screen a reader sees
   * before they have chosen anything.
   */
  it("tells a first-time reader what they are protecting, correctly", async () => {
    core.vaultStatus.mockResolvedValue(NEVER_SET_UP);
    paint();
    await screen.findByText("Protect your workspace");
    const text = document.body.textContent ?? "";
    expect(text).toMatch(/MCP bearer token/i);
    expect(text).toMatch(/assistant API keys/i);
    expect(text).not.toMatch(/seals?\s+(your\s+)?kubeconfigs/i);
    expect(text).toMatch(/kubeconfigs are not sealed/i);
  });

  it("hardens its wording once a reader has failed twice", async () => {
    const user = userEvent.setup();
    paint();
    await screen.findByText("Workspace locked");
    for (let n = 0; n < 2; n++) await failOnce(user);
    expect(screen.getByRole("alert").textContent).toBe("That passphrase is not correct.");
    await failOnce(user);
    expect(screen.getByRole("alert").textContent).toBe(
      "That passphrase is not correct. Nothing is unsealed after a failed attempt.",
    );
  });

  it("prints no passphrase anywhere", async () => {
    paint();
    await screen.findByText("Workspace locked");
    // §25 draws a mock hint naming a working credential. It is fixture text and
    // it does not ship.
    expect(document.body.textContent ?? "").not.toMatch(/the passphrase is/i);
    expect(document.body.textContent ?? "").not.toMatch(/\bMock:/);
    // Nor anywhere a value can hide out of the text: the field's own
    // placeholder, its name, a title attribute, or a pre-filled value.
    for (const input of Array.from(document.querySelectorAll("input"))) {
      expect(input.getAttribute("placeholder") ?? "").not.toMatch(/srelens/i);
      expect(input.getAttribute("aria-label") ?? "").not.toMatch(/srelens/i);
      expect(input.getAttribute("title") ?? "").not.toMatch(/srelens/i);
      expect(input.value).toBe("");
    }
  });

  /**
   * §25's footer says "N clusters sealed". The number is a count of kube
   * contexts, and not one of them is sealed by anything — so the count was
   * right and the word beside it was a claim about encryption that does not
   * exist. What IS true of those clusters while this cover is up is that the
   * window they are reached through has been replaced, so they are out of
   * reach. That is what the footer says now.
   */
  it("says what is true of the clusters it counts, and does not call them sealed", async () => {
    const user = userEvent.setup();
    act(() => setContexts([ctx("prod"), ctx("dev")]));
    paint();
    await screen.findByText("Workspace locked");
    expect(screen.getByText("2 clusters out of reach · no failed attempts")).toBeTruthy();
    expect(document.body.textContent ?? "").not.toMatch(/clusters? sealed/i);
    await failOnce(user);
    expect(screen.getByText("2 clusters out of reach · 1 failed attempt")).toBeTruthy();
    await failOnce(user);
    expect(screen.getByText("2 clusters out of reach · 2 failed attempts")).toBeTruthy();
  });

  it("omits the count rather than reporting a cluster list it never read", async () => {
    paint();
    await screen.findByText("Workspace locked");
    expect(screen.getByText("no failed attempts")).toBeTruthy();
    expect(document.body.textContent ?? "").not.toMatch(/0 clusters/);
  });

  it("keeps the original refusal, folded, under §25's sentence", async () => {
    const user = userEvent.setup();
    core.vaultUnlockPassword.mockRejectedValue(new Error("vault: HMAC verification failed"));
    paint();
    await screen.findByText("Workspace locked");
    await failOnce(user);
    // §25's sentence is the headline and nothing else is in the live region —
    // but the string the backend actually sent is not dropped, because "that
    // passphrase is not correct" is not the only reason an unlock can refuse.
    expect(screen.getByRole("alert").textContent).toBe("That passphrase is not correct.");
    expect(screen.getByText(/HMAC verification failed/)).toBeTruthy();
  });

  it("reveals and re-hides the passphrase on request", async () => {
    const user = userEvent.setup();
    paint();
    await screen.findByText("Workspace locked");
    expect(field().getAttribute("type")).toBe("password");
    await user.click(screen.getByRole("button", { name: "Show passphrase" }));
    expect(field().getAttribute("type")).toBe("text");
    await user.click(screen.getByRole("button", { name: "Hide passphrase" }));
    expect(field().getAttribute("type")).toBe("password");
  });

  it("will not submit an empty passphrase, and says it is working while it derives", async () => {
    const user = userEvent.setup();
    let release = () => {};
    core.vaultUnlockPassword.mockReturnValue(
      new Promise<void>((resolve) => {
        release = () => resolve();
      }),
    );
    paint();
    await screen.findByText("Workspace locked");
    expect(screen.getByRole("button", { name: "Unlock workspace" })).toHaveProperty(
      "disabled",
      true,
    );
    await user.type(field(), TYPED);
    await user.click(screen.getByRole("button", { name: "Unlock workspace" }));
    expect(await screen.findByRole("button", { name: /Deriving key/ })).toBeTruthy();
    core.vaultStatus.mockResolvedValue(OPEN);
    await act(async () => {
      release();
    });
  });
});

describe("LockGate — Touch ID", () => {
  it("offers Touch ID only where it is enabled", async () => {
    paint();
    await screen.findByText("Workspace locked");
    expect(screen.queryByRole("button", { name: /touch id/i })).toBeNull();
    expect(core.vaultBiometricUnlock).not.toHaveBeenCalled();
  });

  it("raises the enrolled prompt once, uninvited, and unlocks on it", async () => {
    core.vaultStatus.mockResolvedValueOnce(SEALED_WITH_BIOMETRIC).mockResolvedValue(OPEN);
    paint();
    await waitFor(() => expect(core.vaultBiometricUnlock).toHaveBeenCalledTimes(1));
    expect(await screen.findByTestId("body")).toBeTruthy();
  });

  it("falls back to the passphrase when the enrolled prompt is cancelled", async () => {
    core.vaultStatus.mockResolvedValue(SEALED_WITH_BIOMETRIC);
    core.vaultBiometricUnlock.mockRejectedValue(new Error("the user cancelled"));
    paint();
    await screen.findByText("Workspace locked");
    // Cancelling a sheet nobody asked for is not a failure worth a banner, and
    // it is not a wrong passphrase either.
    await waitFor(() => expect(core.vaultBiometricUnlock).toHaveBeenCalledTimes(1));
    // The live region is mounted from the start on purpose — an `alert` that
    // arrives with its own text may be read as ordinary prose — so what is
    // asserted is that it is EMPTY, not that it is absent.
    expect(screen.getByRole("alert").textContent).toBe("");
    expect(field()).toBeTruthy();
    expect(screen.getByRole("button", { name: /Unlock with Touch ID/ })).toBeTruthy();
  });
});

describe("LockGate — recovery", () => {
  it("reads the keychain copy only when asked, and shows it once", async () => {
    const user = userEvent.setup();
    paint();
    await screen.findByText("Workspace locked");
    expect(core.vaultRecoverPassword).not.toHaveBeenCalled();
    core.vaultStatus.mockResolvedValue(OPEN);
    await user.click(screen.getByRole("button", { name: "Forgot your passphrase?" }));
    expect(await screen.findByText("recovered-value")).toBeTruthy();
    // The vault is already open by then — the recovery command unlocks as a
    // side effect — but the cover stays up until the reader has read it.
    expect(screen.queryByTestId("body")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByTestId("body")).toBeTruthy();
  });

  /**
   * "This is the only time srelens shows it" is false: `Forgot your
   * passphrase?` reads the same keychain copy and prints the same value every
   * time it is asked, with no credential typed
   * (`recover_password_core`, `vault_password.rs:212-246`). A reader told
   * otherwise would take no note of a passphrase they could have recovered —
   * or, worse, would believe a copy exists nowhere when one does.
   */
  it("does not claim this is the only time the passphrase can be read", async () => {
    const user = userEvent.setup();
    paint();
    await screen.findByText("Workspace locked");
    core.vaultStatus.mockResolvedValue(OPEN);
    await user.click(screen.getByRole("button", { name: "Forgot your passphrase?" }));
    await screen.findByText("recovered-value");
    const text = document.body.textContent ?? "";
    expect(text).not.toMatch(/only time/i);
    // What is true instead: this came out of the recovery copy, and the copy is
    // still there.
    expect(text).toMatch(/recovery copy/i);
  });

  it("says so when there is no recovery copy to read", async () => {
    const user = userEvent.setup();
    core.vaultRecoverPassword.mockRejectedValue(new Error("no recovery copy was kept"));
    paint();
    await screen.findByText("Workspace locked");
    await user.click(screen.getByRole("button", { name: "Forgot your passphrase?" }));
    expect(await screen.findByText(/no recovery copy was kept/)).toBeTruthy();
    // Not §25's escalation: nothing about a refused recovery says the
    // passphrase that was typed is wrong, and the counter must not move.
    expect(screen.queryByText("That passphrase is not correct.")).toBeNull();
    expect(screen.getByText(/no failed attempts/)).toBeTruthy();
  });
});

describe("LockGate — a vault that was never set up", () => {
  it("asks for a passphrase to create rather than one to enter", async () => {
    core.vaultStatus.mockResolvedValue(NEVER_SET_UP);
    paint();
    expect(await screen.findByText("Protect your workspace")).toBeTruthy();
    expect(screen.queryByText("Workspace locked")).toBeNull();
    expect(screen.getByLabelText("Master passphrase")).toBeTruthy();
    expect(screen.getByLabelText("Repeat the passphrase")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Create passphrase" })).toBeTruthy();
    expect(screen.queryByTestId("body")).toBeNull();
  });

  it("sets the passphrase up with the keychain choice the reader made", async () => {
    const user = userEvent.setup();
    core.vaultStatus.mockResolvedValue(NEVER_SET_UP);
    paint();
    await screen.findByText("Protect your workspace");
    await user.click(screen.getByRole("checkbox", { name: /recovery copy/i }));
    await user.type(screen.getByLabelText("Master passphrase"), "aaaa1111aaaa");
    await user.type(screen.getByLabelText("Repeat the passphrase"), "aaaa1111aaaa");
    core.vaultStatus.mockResolvedValue(OPEN);
    await user.click(screen.getByRole("button", { name: "Create passphrase" }));
    expect(core.vaultSetupPassword).toHaveBeenCalledWith("aaaa1111aaaa", false);
    expect(await screen.findByTestId("body")).toBeTruthy();
  });

  it("will not create a passphrase from two entries that differ", async () => {
    const user = userEvent.setup();
    core.vaultStatus.mockResolvedValue(NEVER_SET_UP);
    paint();
    await screen.findByText("Protect your workspace");
    await user.type(screen.getByLabelText("Master passphrase"), "aaaa1111aaaa");
    await user.type(screen.getByLabelText("Repeat the passphrase"), "bbbb2222bbbb");
    expect(screen.getByRole("button", { name: "Create passphrase" })).toHaveProperty(
      "disabled",
      true,
    );
    expect(screen.getByText("The two entries do not match.")).toBeTruthy();
    expect(core.vaultSetupPassword).not.toHaveBeenCalled();
  });
});
