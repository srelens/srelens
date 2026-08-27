import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Chrome } from "./Chrome";
import {
  createWorkspace,
  currentWorkspace,
  getState,
  openTab,
  setState,
  switchWorkspace,
  togglePin,
} from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";
import { APPEARANCE_KEY } from "../lib/appearance";
import { lockWorkspace, resetLock, __setKnownVaultMode } from "./LockGate";

// jsdom has no ResizeObserver and Radix's popper watches the trigger with one.
// The same stub the kit's Radix-backed suites carry, kept here rather than in
// the shared setup so the requirement stays visible.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// Scale is core's, and core's writes it to storage and asks the webview to
// zoom. Both are mocked: this suite is about the buttons calling the right
// thing with the right number, and `stepUiScale` stays real so the number is
// the one the app would use.
const { scale, desktop, platform } = vi.hoisted(() => ({
  scale: { get: vi.fn(() => 100), set: vi.fn((n: number) => n), apply: vi.fn() },
  desktop: vi.fn(() => true),
  platform: vi.fn(() => true),
}));

/**
 * The chord table, as a DOUBLE this file can change.
 *
 * Comparing the rendered tooltip to the real `hint("lock", …)` is not enough on
 * its own: `⌘⇧L` typed by hand into the component passes that test, because the
 * literal and the table agree today. That is the shape this codebase has already
 * shipped twelve times — a test that cannot fail for the reason its name gives.
 * So `hint` is a spy that delegates to the real table by default, and one test
 * changes what it returns and asserts the bar follows.
 */
const chords = vi.hoisted(() => ({
  hint: vi.fn(),
  real: { fn: (_action: string, _apple: boolean): string => "" },
}));
vi.mock("../lib/shortcuts", async (orig) => {
  const real = await orig<typeof import("../lib/shortcuts")>();
  chords.real.fn = (action, apple) => real.hint(action as Parameters<typeof real.hint>[0], apple);
  return { ...real, hint: chords.hint };
});
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  getUiScale: scale.get,
  setUiScale: scale.set,
  applyUiScale: scale.apply,
  isTauri: desktop,
  isApplePlatform: platform,
}));

const ctx = (id: string) => ({
  name: id, stableId: id, cluster: id, server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
});

beforeEach(() => {
  setState(defaultState([ctx("prod")]));
  vi.clearAllMocks();
  // `clearAllMocks` forgets the calls, not the implementations — so a test that
  // asked for web mode would leak into the next one without this.
  desktop.mockReturnValue(true);
  platform.mockReturnValue(true);
  chords.hint.mockImplementation(chords.real.fn);
  resetLock();
  // The ordinary desktop state: a vault that exists and is open. `resetLock`
  // leaves the mode unknown, which is the pre-launch-read state, so every test
  // that is not about that case says so.
  __setKnownVaultMode("unlocked");
});

const chrome = (props: Partial<Parameters<typeof Chrome>[0]> = {}) =>
  render(<Chrome controls="none" onToggleTheme={() => {}} onNewWorkspace={() => {}} {...props} />);

/** The chip is the first button in the bar; the panel is portalled after it. */
const openSwitcher = async () => {
  await userEvent.click(screen.getByRole("button", { name: /Default/ }));
  await screen.findByRole("dialog", { name: "Workspaces" });
};

describe("Chrome", () => {
  it("names the workspace and the active cluster in the bar", () => {
    chrome({ clusterName: "prod" });
    expect(screen.getByRole("button", { name: /Default/ })).toBeDefined();
    expect(screen.getByText("prod")).toBeDefined();
  });

  it("leaves room for the real traffic lights the overlay keeps", () => {
    // Found on a real machine: with an overlay titlebar, macOS's own traffic
    // lights stay and the painted ones doubled them. Under the overlay this
    // bar paints none and starts its content past the natives instead.
    chrome({ controls: "none" });
    const gap = document.querySelector("[data-native-lights]");
    expect(gap).not.toBeNull();
    expect(gap?.hasAttribute("data-tauri-drag-region")).toBe(true);
  });

  it("paints the picture only where no real window has lights", () => {
    // An Apple browser: no native lights in the page, so the kit's picture
    // shows and the gap has nothing to clear.
    desktop.mockReturnValue(false);
    chrome({ controls: "macos" });
    expect(document.querySelector("[data-native-lights]")).toBeNull();
  });

  it("gives a non-Apple window neither picture nor gap", () => {
    platform.mockReturnValue(false);
    chrome({ controls: "none" });
    expect(document.querySelector("[data-native-lights]")).toBeNull();
  });

  it("never doubles up: a caller asking the kit for the picture under Tauri+Apple gets no gap too", () => {
    // `controls="macos"` under real Tauri on Apple asks the kit to draw its
    // own three lights. If the gap were derived independently of `controls`,
    // this is exactly the case that redraws both — the doubling `ec024b5`
    // exists to remove. desktop/platform default to true in beforeEach.
    chrome({ controls: "macos" });
    expect(document.querySelector("[data-native-lights]")).toBeNull();
    expect(document.querySelectorAll("[data-light]")).toHaveLength(3);
  });

  it("zooms through uiScale", async () => {
    chrome();
    await userEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(scale.set).toHaveBeenCalledWith(110);
    expect(scale.apply).toHaveBeenCalledWith(110);
  });

  it("switches workspaces from the switcher", async () => {
    const id = createWorkspace("Team", ["prod"]);
    setState({ ...getState(), currentId: getState().workspaces[0].id });
    chrome();
    await openSwitcher();
    await userEvent.click(screen.getByRole("button", { name: /^Team/ }));
    expect(currentWorkspace().id).toBe(id);
  });

  it("asks before removing a workspace holding tabs the user opened", async () => {
    const id = createWorkspace("Team", ["prod"]);
    // `createWorkspace` switches into it, so this tab lands in Team.
    openTab("/k/pods");
    switchWorkspace(getState().workspaces[0].id);
    chrome();
    await openSwitcher();
    await userEvent.click(screen.getByRole("button", { name: /Remove Team/ }));
    expect(screen.getByRole("dialog", { name: /Remove Team/ })).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(getState().workspaces.some((w) => w.id === id)).toBe(false);
  });

  it("removes a workspace holding only its pinned home tab without asking", async () => {
    // Nothing to lose: a question here is a question about nothing.
    const id = createWorkspace("Team", ["prod"]);
    switchWorkspace(getState().workspaces[0].id);
    chrome();
    await openSwitcher();
    await userEvent.click(screen.getByRole("button", { name: /Remove Team/ }));
    expect(screen.queryByRole("dialog", { name: /Remove Team/ })).toBeNull();
    expect(getState().workspaces.some((w) => w.id === id)).toBe(false);
  });

  it("still asks when the tabs the user opened have been pinned", async () => {
    // Pinning is the user saying a tab matters. A rule that skipped the
    // question when every tab was pinned dropped exactly the tabs someone had
    // marked as worth keeping, silently and with no undo. Only the seeded home
    // tab — the single tab a workspace is born with — is nothing to lose.
    const id = createWorkspace("Team", ["prod"]);
    openTab("/k/pods");
    togglePin(currentWorkspace().activeId);
    switchWorkspace(getState().workspaces[0].id);
    chrome();
    await openSwitcher();
    await userEvent.click(screen.getByRole("button", { name: /Remove Team/ }));
    expect(screen.getByRole("dialog", { name: /Remove Team/ })).toBeDefined();
    expect(getState().workspaces.some((w) => w.id === id)).toBe(true);
  });

  it("keeps the workspace when the confirmation is dismissed", async () => {
    const id = createWorkspace("Team", ["prod"]);
    openTab("/k/pods");
    switchWorkspace(getState().workspaces[0].id);
    chrome();
    await openSwitcher();
    await userEvent.click(screen.getByRole("button", { name: /Remove Team/ }));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /Remove Team/ })).toBeNull());
    expect(getState().workspaces.some((w) => w.id === id)).toBe(true);
  });

  it("offers no zoom controls in web mode, where the browser's own zoom applies", () => {
    desktop.mockReturnValue(false);
    chrome();
    expect(screen.queryByRole("button", { name: "Zoom in" })).toBeNull();
    expect(screen.getByRole("button", { name: "Theme" })).toBeDefined();
  });

  it("opens Settings from the appearance action and calls the theme toggle", async () => {
    const onToggleTheme = vi.fn();
    chrome({ onToggleTheme });
    await userEvent.click(screen.getByRole("button", { name: "Theme" }));
    expect(onToggleTheme).toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Appearance settings" }));
    expect(currentWorkspace().tabs.some((t) => t.route === "/settings")).toBe(true);
  });

  /**
   * Finding 7's other half. The Appearance pane's store was the only writer of
   * the appearance record, so this button — which is the other thing a reader
   * can change `data-theme` with — was remembered nowhere, and boot put the
   * pane's older theme back over it at the next launch. Its choice is recorded
   * now, and recorded AFTER the host has written the root, so what is stored is
   * the value that actually landed.
   */
  it("records the theme the host put on the root, so the next launch keeps it", async () => {
    localStorage.removeItem(APPEARANCE_KEY);
    document.documentElement.setAttribute("data-theme", "dark");
    chrome({
      onToggleTheme: () => {
        // What `toggleNextDesignTheme` does: writes the root itself. Light is
        // the bare root, so the attribute comes off.
        document.documentElement.removeAttribute("data-theme");
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Theme" }));
    expect(JSON.parse(localStorage.getItem(APPEARANCE_KEY) ?? "null")).toEqual({ theme: "light" });
  });

  it("asks the switcher for a new workspace rather than making one itself", async () => {
    const onNewWorkspace = vi.fn();
    chrome({ onNewWorkspace });
    await openSwitcher();
    await userEvent.click(screen.getByRole("button", { name: "New workspace" }));
    expect(onNewWorkspace).toHaveBeenCalledTimes(1);
    expect(getState().workspaces).toHaveLength(1);
  });

  /**
   * Spec decision 5, at this component's own boundary. Behind a raised cover
   * the gear opened `/settings` and the switcher's `onRemove` deleted a
   * workspace outright — no dialog, no credential — whenever it held one tab,
   * which is how every workspace starts. `Window`'s suite proves it end to
   * end; this pins the bar itself, so editing `Chrome` alone cannot undo it.
   */
  describe("while the vault is sealed", () => {
    it("keeps the workspace name as a readout, with nothing to press", () => {
      const id = createWorkspace("Team", ["prod"]);
      switchWorkspace(id);
      lockWorkspace();
      chrome();
      expect(screen.queryByRole("button", { name: /^Team/ })).toBeNull();
      expect(screen.getByText("Team")).toBeDefined();
      expect(getState().workspaces.some((w) => w.id === id)).toBe(true);
    });

    it("disables the way into Settings, and says why", async () => {
      lockWorkspace();
      chrome();
      const gear = screen.getByRole("button", { name: "Appearance settings" }) as HTMLButtonElement;
      expect(gear.disabled).toBe(true);
      expect(gear.getAttribute("title")).toMatch(/unlock/i);
      await userEvent.click(gear);
      expect(currentWorkspace().tabs.some((t) => t.route === "/settings")).toBe(false);
    });

    /**
     * Both of these change how the lock screen LOOKS and nothing else, and a
     * reader who cannot read the passphrase field cannot unlock. Taking them
     * away would be a lock-out rather than a lock.
     */
    it("still lets the reader make this screen legible", async () => {
      const onToggleTheme = vi.fn();
      lockWorkspace();
      chrome({ onToggleTheme });
      await userEvent.click(screen.getByRole("button", { name: "Theme" }));
      expect(onToggleTheme).toHaveBeenCalled();
      await userEvent.click(screen.getByRole("button", { name: "Zoom in" }));
      expect(scale.apply).toHaveBeenCalled();
    });
  });

  /**
   * `⌘⇧L` has been bound since Task 9 and nothing on screen offered it: the only
   * ways to lock were a keyboard chord and a button buried in Settings →
   * Security. This is the discoverable door — and the four cases below are the
   * ones where drawing it would be drawing a control that cannot work.
   */
  describe("Lock workspace", () => {
    it("hands the press straight to the window's own lock, not a second path", async () => {
      const onLock = vi.fn();
      chrome({ onLock });
      await userEvent.click(screen.getByRole("button", { name: "Lock workspace" }));
      expect(onLock).toHaveBeenCalledTimes(1);
    });

    it("carries the chord from the table that binds it", () => {
      chrome({ onLock: () => {} });
      const chord = chords.real.fn("lock", true);
      // Non-empty, as `SecurityPane`'s own hint test asserts: `hint` returns ""
      // for an unbound action, and a test that accepted that would pass over a
      // control promising a key nothing answers.
      expect(chord).not.toBe("");
      expect(screen.getByRole("button", { name: "Lock workspace" }).getAttribute("title")).toBe(
        `Lock workspace \u00b7 ${chord}`,
      );
      expect(chords.hint).toHaveBeenCalledWith("lock", true);
    });

    /**
     * The test above passes on a hand-typed `\u2318\u21e7L`, because the literal and
     * the table agree today. This one does not: it moves the binding in the
     * double and requires the bar to follow.
     */
    it("follows the table when the binding moves, rather than a literal", () => {
      chords.hint.mockReturnValue("Ctrl+Alt+Q");
      chrome({ onLock: () => {} });
      expect(screen.getByRole("button", { name: "Lock workspace" }).getAttribute("title")).toBe(
        "Lock workspace \u00b7 Ctrl+Alt+Q",
      );
    });

    it("says only the action when the chord is unbound", () => {
      // `hint` returns "" for an action with no row, and a tooltip ending in a
      // bare separator would promise a key that does not exist.
      chords.hint.mockReturnValue("");
      chrome({ onLock: () => {} });
      expect(screen.getByRole("button", { name: "Lock workspace" }).getAttribute("title")).toBe(
        "Lock workspace",
      );
    });

    it("is not drawn in web mode, where every vault command rejects", () => {
      desktop.mockReturnValue(false);
      chrome({ onLock: () => {} });
      expect(screen.queryByRole("button", { name: "Lock workspace" })).toBeNull();
    });

    /**
     * Locking a pre-setup vault is REFUSED by `lock_core`
     * (`apps/desktop/src-tauri/src/vault_password.rs`): a machine-key vault
     * resolves its key once at open, and discarding it would strand the process
     * until restart. A control refused by design is not offered.
     */
    it("is not drawn when there is no vault to lock", () => {
      __setKnownVaultMode("setup-required");
      chrome({ onLock: () => {} });
      expect(screen.queryByRole("button", { name: "Lock workspace" })).toBeNull();
    });

    it("is not drawn before the vault's state is known, or when reading it failed", () => {
      resetLock();
      chrome({ onLock: () => {} });
      expect(screen.queryByRole("button", { name: "Lock workspace" })).toBeNull();
    });

    it("is not the one live control on a locked window", () => {
      lockWorkspace();
      chrome({ onLock: () => {} });
      expect(screen.queryByRole("button", { name: "Lock workspace" })).toBeNull();
    });

    it("is not drawn where no window is behind the bar to lock", () => {
      chrome();
      expect(screen.queryByRole("button", { name: "Lock workspace" })).toBeNull();
    });
  });
});
