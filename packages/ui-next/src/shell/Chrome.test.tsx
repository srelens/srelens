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
import { lockWorkspace, resetLock } from "./LockGate";

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
  resetLock();
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
});
