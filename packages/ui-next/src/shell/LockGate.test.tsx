import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UserEvent } from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";

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
import {
  isWorkspaceSealed,
  LockGate,
  lockWorkspace,
  resetLock,
  useWorkspaceSealed,
  __setKnownVaultMode,
} from "./LockGate";
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

/**
 * The brand mark is the HOST's asset, injected down `NextApp` -> `Window` ->
 * `LockGate` because this package cannot import from `apps/desktop`. A value
 * that is obviously not a real path, so nothing can pass by recognising the
 * real filename, and jsdom never fetches it either way.
 */
const BRAND = "/aardvark-ledger-mark.svg";

function paint(props: { brandMarkSrc?: string; onReady?: () => void } = {}) {
  render(
    <LockGate brandMarkSrc={props.brandMarkSrc} onReady={props.onReady}>
      <Behind />
    </LockGate>,
  );
}

/**
 * A sibling of the gate, the way `Chrome` and `Status` are: outside the band
 * the cover replaces, reading the same store to decide whether to stand down.
 * Its two words are the whole question — is this control live over a window
 * that looks sealed?
 */
function Outside() {
  const sealed = useWorkspaceSealed();
  return <span data-testid="outside">{sealed ? "standing down" : "live"}</span>;
}

function paintWithSibling() {
  render(
    <>
      <Outside />
      <LockGate>
        <Behind />
      </LockGate>
    </>,
  );
}

function outside(): string {
  return screen.getByTestId("outside").textContent ?? "";
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

  /**
   * The cover claimed `aria-modal="true"` — "there is nothing else on this
   * window to reach" — while the titlebar's theme and zoom controls stayed in
   * the tab order on purpose. One of those two statements had to go, and it was
   * not the Tab key's.
   */
  it("does not tell assistive technology that nothing outside it exists", async () => {
    paint();
    await screen.findByText("Workspace locked");
    const cover = screen.getByTestId("lock-cover");
    expect(cover.getAttribute("role")).toBe("dialog");
    expect(cover.getAttribute("aria-label")).toBe("Workspace locked");
    expect(cover.getAttribute("aria-modal")).toBeNull();
  });

  /**
   * The first render of this screen anyone had seen (#372: `screenshot.mjs`
   * drives web mode, where this gate deliberately never raises) showed an empty
   * pale box above the heading. The tile drew its own inline `<svg>` with
   * `stroke="var(--muted)"`, and there is no `--muted` token — it is
   * `--ink-muted` — so neither path painted.
   *
   * **What these tests cannot defend.** jsdom attaches no stylesheet, computes
   * no layout and fetches no image, and per #372 the screenshot harness cannot
   * reach this surface at all — so whether the composition LOOKS like one
   * object, in five themes and five accents, is not covered here and is not
   * claimed. The user is the only reviewer this screen has, and three
   * arrangements have now been put in front of them. What IS covered is that
   * the tile has something in it in each of its three states, and that nothing
   * in it is announced over the heading.
   */
  it("draws the app's own mark", async () => {
    paint({ brandMarkSrc: BRAND });
    await screen.findByText("Workspace locked");
    const mark = screen.getByTestId("lock-mark");
    expect(mark.querySelector("img")?.getAttribute("src")).toBe(BRAND);
  });

  /**
   * The padlock that used to sit beside it. It identified nothing, it read as a
   * second unrelated icon, and it repeated the heading eighteen pixels below it
   * in a picture. Asserted as an absence so it cannot drift back in.
   */
  it("draws no second glyph beside it, because the heading says the state", async () => {
    paint({ brandMarkSrc: BRAND });
    await screen.findByText("Workspace locked");
    const mark = screen.getByTestId("lock-mark");
    expect(mark.querySelectorAll("svg")).toHaveLength(0);
    expect(mark.querySelectorAll("img")).toHaveLength(1);
    expect(screen.getAllByText("Workspace locked")).toHaveLength(1);
  });

  /**
   * `Mark`'s own contract, and the reason this is a `Mark` and not an `<img>`:
   * "an image that will not load is a state, not an error to report". A host
   * that passes no asset, and an asset that will not load, both have to land on
   * something — an empty box is exactly what the user was looking at three
   * screenshots ago.
   */
  it("falls through to the mark's initials when there is no asset", async () => {
    paint();
    await screen.findByText("Workspace locked");
    const mark = screen.getByTestId("lock-mark");
    expect(mark.querySelector("img")).toBeNull();
    expect(mark.querySelector('[data-slot="chip-mark"]')?.textContent).not.toBe("");
  });

  it("falls through the same way when the asset will not load", async () => {
    paint({ brandMarkSrc: BRAND });
    await screen.findByText("Workspace locked");
    const image = screen.getByTestId("lock-mark").querySelector("img");
    expect(image).not.toBeNull();
    act(() => {
      fireEvent.error(image as HTMLImageElement);
    });
    const mark = screen.getByTestId("lock-mark");
    expect(mark.querySelector("img")).toBeNull();
    expect(mark.querySelector('[data-slot="chip-mark"]')?.textContent).not.toBe("");
  });

  it("says nothing the heading under it already says", async () => {
    paint({ brandMarkSrc: BRAND });
    await screen.findByText("Workspace locked");
    const mark = screen.getByTestId("lock-mark");
    expect(mark.querySelector('[aria-hidden="true"]')).not.toBeNull();
    expect(mark.querySelector('[role="img"]')).toBeNull();
    // The badge would ride the initials under the mark — the app's name twice.
    expect(mark.querySelector('[data-slot="chip-badge"]')).toBeNull();
  });

  it("names no colour of its own on that tile", async () => {
    paint();
    await screen.findByText("Workspace locked");
    const source = readFileSync(join(__dirname, "LockGate.tsx"), "utf8").replace(
      /\/\*[\s\S]*?\*\/|\/\/.*/g,
      "",
    );
    expect(source).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    // The token that did not exist. Both of its uses were here, and they were
    // the only two in the whole source tree.
    expect(source).not.toMatch(/var\(--muted\)/);
  });

  /**
   * The user's words: "logo is not proper and should be in center". §25 states
   * no alignment, so the instruction is the authority, and §24 — this screen's
   * sibling — is "centred at 860 px".
   *
   * Classes, not geometry: jsdom lays nothing out. What is asserted is which
   * blocks were centred and, just as deliberately, which were not — the field
   * row whose eyebrow sits opposite `Show passphrase`, and the error alert whose
   * layout, shake and wording other tests in this file already pin.
   */
  it("centres the composition without centring the form under it", async () => {
    const user = userEvent.setup();
    paint();
    const heading = await screen.findByText("Workspace locked");
    const header = heading.closest("header");
    expect(header?.className).toContain("text-center");
    expect(screen.getByTestId("lock-mark").closest("header")).toBe(header);
    // The lede rides with it, so the block reads as one thing.
    expect(header?.textContent ?? "").toMatch(/MCP bearer token/);

    // The field is outside that block and stays as it was.
    const fieldRow = screen.getByRole("button", { name: "Show passphrase" }).closest("div");
    expect(fieldRow?.className ?? "").not.toContain("text-center");

    // And so does the error, which is what carries the shake and the alert role.
    await failOnce(user);
    const alert = screen.getByRole("alert");
    expect(alert.className).toContain("shake");
    expect(alert.className).not.toContain("text-center");
    expect(alert.textContent).toBe("That passphrase is not correct.");
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

/**
 * The launch gap, which is the same fail-open this file already closes for a
 * refused read, one step earlier.
 *
 * On every desktop launch `checking` starts true and the band is covered, but
 * the module store said `sealed === false` until `vaultStatus()` answered — so
 * `Chrome` and `Status`, which are siblings of this gate and disable their
 * handlers from that store, stayed live for the whole check. The workspace
 * switcher was among them, and its `onRemove` deletes a workspace outright with
 * no dialog when it holds one tab or fewer, which is how every workspace
 * starts. A slow or hung status read is that window held open.
 *
 * The doc comment on `sealed` already names the principle — "every await
 * between 'the vault is sealed' and 'the window is covered' is a window of live
 * UI over a sealed vault". This is that await, at launch, and the safe default
 * is sealed.
 */
describe("LockGate — the launch check", () => {
  it("stands its siblings down for the whole check, not just once the vault answers", async () => {
    // A read that never settles: the check is the entire test.
    core.vaultStatus.mockReturnValue(new Promise<VaultStatus>(() => {}));
    paintWithSibling();
    expect(await screen.findByText("Checking whether the workspace is sealed")).toBeTruthy();
    expect(outside()).toBe("standing down");
  });

  /**
   * `Window` installs its keydown listener once and asks this at the
   * keystroke, so the answer has to be true during the check and not only
   * inside a render.
   */
  it("answers the same to a caller that is not in a render", () => {
    core.vaultStatus.mockReturnValue(new Promise<VaultStatus>(() => {}));
    paintWithSibling();
    expect(isWorkspaceSealed()).toBe(true);
  });

  it("gives them back once the vault reports itself open", async () => {
    core.vaultStatus.mockResolvedValue(OPEN);
    paintWithSibling();
    expect(await screen.findByTestId("body")).toBeTruthy();
    expect(outside()).toBe("live");
    expect(isWorkspaceSealed()).toBe(false);
  });

  it("keeps them down when the check comes back sealed", async () => {
    core.vaultStatus.mockResolvedValue(SEALED);
    paintWithSibling();
    await screen.findByText("Workspace locked");
    expect(outside()).toBe("standing down");
  });

  it("keeps them down when the check refuses to answer at all", async () => {
    core.vaultStatus.mockRejectedValue(new Error("vault state unavailable"));
    paintWithSibling();
    await screen.findByText("Secrets unavailable");
    expect(outside()).toBe("standing down");
  });

  /**
   * Web mode has no vault to check and this gate never covers anything there,
   * so nothing may stand down either — the positive control for every
   * assertion above, and the reason "always sealed" is not the answer.
   */
  it("stands nothing down where there is no vault to check", async () => {
    core.isTauri.mockReturnValue(false);
    paintWithSibling();
    expect(await screen.findByTestId("body")).toBeTruthy();
    expect(outside()).toBe("live");
    expect(isWorkspaceSealed()).toBe(false);
    expect(core.vaultStatus).not.toHaveBeenCalled();
  });

  /**
   * The reconcile read exists for a cover raised from OUTSIDE, with no status
   * behind it. Publishing the launch check as sealed makes `raised` true at
   * launch too, so without a guard that effect would fire a second, competing
   * `vaultStatus()` — one that may not lower the cover — alongside the launch
   * read that may.
   */
  it("takes exactly one read at launch, whatever the cover says", async () => {
    core.vaultStatus.mockResolvedValue(OPEN);
    paintWithSibling();
    expect(await screen.findByTestId("body")).toBeTruthy();
    await waitFor(() => expect(core.vaultStatus).toHaveBeenCalledTimes(1));
    expect(core.vaultStatus).toHaveBeenCalledTimes(1);
  });

  /**
   * And the reader is told the truth while it happens: the passphrase form is a
   * claim that the vault is set up and locked, which a check that has not
   * answered has not established.
   */
  it("shows the check, not an unlock form it cannot yet justify", async () => {
    core.vaultStatus.mockReturnValue(new Promise<VaultStatus>(() => {}));
    paintWithSibling();
    expect(await screen.findByText("Checking whether the workspace is sealed")).toBeTruthy();
    expect(screen.queryByLabelText("Master passphrase")).toBeNull();
    expect(screen.queryByRole("button", { name: "Unlock workspace" })).toBeNull();
  });
});

/**
 * `onReady`: the vault is usable, which is a different claim from "the cover is
 * down" and is why it is a callback rather than a store read.
 *
 * Its one consumer is the MCP auto-start (`Window`), and classic has had the
 * same callback on `VaultGate` since the vault shipped. The bearer token is
 * sealed in the vault, so a start before this cannot read or mint one and
 * nothing retries — the ordering is the whole reason it exists.
 */
describe("LockGate, reporting that the vault is usable", () => {
  it("says so once the launch read finds the vault open", async () => {
    const onReady = vi.fn();
    core.vaultStatus.mockResolvedValue(OPEN);
    paint({ onReady });
    await waitFor(() => expect(onReady).toHaveBeenCalledTimes(1));
  });

  it("says nothing while the vault is sealed", async () => {
    const onReady = vi.fn();
    core.vaultStatus.mockResolvedValue(SEALED);
    paint({ onReady });
    expect(await screen.findByRole("heading", { name: "Workspace locked" })).toBeTruthy();
    await act(async () => {});
    expect(onReady).not.toHaveBeenCalled();
  });

  it("says nothing about a vault that has never been set up", async () => {
    const onReady = vi.fn();
    core.vaultStatus.mockResolvedValue(NEVER_SET_UP);
    paint({ onReady });
    expect(await screen.findByRole("heading", { name: "Protect your workspace" })).toBeTruthy();
    await act(async () => {});
    expect(onReady).not.toHaveBeenCalled();
  });

  it("says nothing when the read refused, which is not the same as an open vault", async () => {
    const onReady = vi.fn();
    core.vaultStatus.mockRejectedValue(new Error("the vault state was never managed"));
    paint({ onReady });
    expect(await screen.findByTestId("lock-cover")).toBeTruthy();
    await act(async () => {});
    expect(onReady).not.toHaveBeenCalled();
  });

  it("says so when the reader unlocks", async () => {
    const onReady = vi.fn();
    const user = userEvent.setup();
    core.vaultStatus.mockResolvedValue(SEALED);
    core.vaultUnlockPassword.mockResolvedValue(undefined);
    paint({ onReady });
    await screen.findByRole("heading", { name: "Workspace locked" });
    core.vaultStatus.mockResolvedValue(OPEN);
    await user.type(field(), TYPED);
    await user.click(screen.getByRole("button", { name: "Unlock workspace" }));
    await waitFor(() => expect(onReady).toHaveBeenCalledTimes(1));
  });

  /**
   * Not from the reconcile read that follows a deliberate lock. That read is
   * taken with `mayOpen: false` precisely because a stale or racing `unlocked`
   * must not lower the cover — and it must not announce a usable vault either,
   * or the consumer would act behind a window that is showing the lock screen.
   */
  it("says nothing from the read that follows a lock", async () => {
    const onReady = vi.fn();
    core.vaultStatus.mockRejectedValueOnce(new Error("the vault state was never managed"));
    core.vaultStatus.mockResolvedValue(OPEN);
    paint({ onReady });
    expect(await screen.findByTestId("lock-cover")).toBeTruthy();
    // The reconcile read has run — a `null` status behind a raised cover is
    // exactly the case that takes it — and it came back `unlocked`.
    await waitFor(() => expect(core.vaultStatus.mock.calls.length).toBeGreaterThan(1));
    expect(screen.getByTestId("lock-cover")).toBeTruthy();
    expect(onReady).not.toHaveBeenCalled();
  });

  /**
   * Once per window, not once per read. The consumer is a launch action:
   * `mcp_http_start` on a listener that is already bound is not a no-op, so a
   * second report after a relock and a second unlock would be a second start.
   */
  it("says so once, however many times the vault is opened", async () => {
    const onReady = vi.fn();
    const user = userEvent.setup();
    core.vaultStatus.mockResolvedValue(OPEN);
    core.vaultUnlockPassword.mockResolvedValue(undefined);
    paint({ onReady });
    await waitFor(() => expect(onReady).toHaveBeenCalledTimes(1));
    // Sealed from outside, the way `Lock now` and the chord do it, then opened
    // again with the passphrase.
    core.vaultStatus.mockResolvedValue(SEALED);
    act(() => lockWorkspace());
    await screen.findByRole("heading", { name: "Workspace locked" });
    core.vaultStatus.mockResolvedValue(OPEN);
    await user.type(field(), TYPED);
    await user.click(screen.getByRole("button", { name: "Unlock workspace" }));
    await waitFor(() => expect(screen.getByTestId("body")).toBeTruthy());
    expect(onReady).toHaveBeenCalledTimes(1);
  });
});

/**
 * The third fail-open of this shape, and the one the gate cannot close by
 * itself: a vault whose state NOTHING has read yet.
 *
 * `sealed` and `checkingLaunch` are both written by a MOUNTED gate. Before one
 * mounts — a fresh module, a webview reload, the boot branch this window used
 * to render with no gate in it at all — both are false and `isCovered()`
 * answered "not covered" about a vault it had never read. That is a guess
 * dressed as a fact, and `AgentConsent` spends it on an Approve button: the MCP
 * server is a backend process that survives a reload, so a confirm-gated call
 * could arrive in exactly that window and be APPROVED over a vault the backend
 * had already sealed.
 *
 * `knownMode` is the honest three-state record of it — a mode, or `null` for
 * "no read has landed" — and the rule is that not-yet-known counts as covered.
 *
 * **Where a vault exists.** In web mode there is no vault, this gate takes no
 * read, and `knownMode` stays `null` for the life of the page: a bare
 * "null means covered" would leave a browser under a permanent cover, or leave
 * `AgentConsent` refusing every request forever. So the rule is conditioned on
 * the same `isTauri()` the gate itself decides to read by.
 */
describe("LockGate — a vault whose state nothing has read yet", () => {
  it("counts as covered where a vault exists, before any read has landed", () => {
    // No gate mounted, and nothing has answered: the store as a fresh module
    // has it, and as a reloaded webview has it.
    expect(isWorkspaceSealed()).toBe(true);
  });

  it("stands a sibling down for it, not only a caller outside a render", () => {
    render(<Outside />);
    expect(outside()).toBe("standing down");
  });

  it("counts as open in web mode, where no read was ever going to be taken", () => {
    core.isTauri.mockReturnValue(false);
    expect(isWorkspaceSealed()).toBe(false);
    render(<Outside />);
    expect(outside()).toBe("live");
  });

  /**
   * The positive control: this must be the ABSENCE of a read talking, not a
   * store that is now sealed forever. A landed `unlocked` gives the window
   * back.
   */
  it("gives the window back once a read has established a mode", () => {
    act(() => __setKnownVaultMode("unlocked"));
    expect(isWorkspaceSealed()).toBe(false);
    render(<Outside />);
    expect(outside()).toBe("live");
  });
});
