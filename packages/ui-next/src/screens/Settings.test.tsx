import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * Every capability the six panes reach for, replaced — and `isTauri`, which
 * decides whether the `Security` entry is drawn at all.
 *
 * `describeError`, `plural`, `isApplePlatform`, `settingsStorage` and the
 * shortcut table stay real: this file is about the shell, and a shell test that
 * also replaced the panes' own vocabulary would pass over panes that had
 * stopped saying anything.
 */
const core = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  applyUiScale: vi.fn(),
  getMcpToken: vi.fn(),
  mcpHttpStatus: vi.fn(),
  rotateMcpToken: vi.fn(),
  revokeMcpToken: vi.fn(),
  auditTail: vi.fn(),
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

import type { VaultBiometricStatus } from "@srelens/core";
import { Settings } from "./Settings";

const ROUTE = "/settings";

/** A sensor that exists with the gate off — the ordinary desktop shape. */
const SENSOR_OFF: VaultBiometricStatus = { available: true, enabled: false, unlocked: true };

/** What a running loopback server answers with. */
const STATUS_URL = "http://127.0.0.1:8765/mcp";

/** A realistic bearer value: 64 hex characters, no prefix, as the backend mints. */
const TOKEN = "4f9a2c7e1b6d80f3c9a1e7b4f2d6c8035a9e1c7b4f2d6c8035f9c1a7e4b2d6f8";

/**
 * Names that share no substring with anything this screen or the panes could
 * write on their own — no "log", no "cluster", no "resource", and not the
 * length of the real `PORTED_SCREENS`. A fixture that reuses a word the
 * component already has is how a component that invents its own list passes.
 */
const PORTED = ["Aardvark ledger", "Basalt tally", "Cinnabar dial"];

/** §23's nav, in §23's order, minus the entry decision 2 removed. */
const DESKTOP_SECTIONS = [
  "Agent & MCP",
  "Security",
  "Appearance",
  "Accessibility",
  "Shortcuts",
  "Clusters",
];

/** The same nav where no vault command can answer. */
const WEB_SECTIONS = DESKTOP_SECTIONS.filter((s) => s !== "Security");

function paint(props: { onLocked?: () => void } = {}) {
  const onSwitchToClassic = vi.fn();
  // `onLocked` is required on `RoutedScreenProps` now that `shell/LockGate`
  // exists to raise, so every render supplies one — the tests that care about
  // it pass their own spy in and read it back.
  const onLocked = props.onLocked ?? vi.fn();
  render(
    <Settings
      route={ROUTE}
      ported={PORTED}
      onSwitchToClassic={onSwitchToClassic}
      onLocked={onLocked}
    />,
  );
  return { onSwitchToClassic, onLocked, user: userEvent.setup() };
}

function sections(): string[] {
  return screen.getAllByRole("tab").map((t) => t.textContent ?? "");
}

describe("Settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.isTauri.mockReturnValue(true);
    core.getMcpToken.mockResolvedValue(TOKEN);
    core.mcpHttpStatus.mockResolvedValue(STATUS_URL);
    core.auditTail.mockResolvedValue([]);
    core.vaultLock.mockResolvedValue(undefined);
    core.vaultBiometricStatus.mockResolvedValue(SENSOR_OFF);
  });

  it("titles itself the way the design does, and takes no header action", () => {
    paint();
    expect(screen.getByRole("heading", { level: 1, name: "Settings" })).toBeTruthy();
    expect(screen.getByText("workspace")).toBeTruthy();
    expect(document.querySelector('[data-slot="screen-actions"]')).toBeNull();
  });

  it("lists every section, in order, and no section it cannot fill", () => {
    paint();
    expect(sections()).toEqual(DESKTOP_SECTIONS);
    expect(screen.queryByRole("tab", { name: /deep links/i })).toBeNull();
  });

  /**
   * The reason the `Deep links` entry is absent, pinned as a property rather
   * than asserted in a comment.
   *
   * The comment this replaces said `srelens://` "exists nowhere in this repo —
   * no scheme, no handler, no parser". All three were false: the scheme is in
   * `tauri.conf.json`, the parser is `deepLink.ts` with its own suite, the
   * handler is `deep_link.rs` wired in four places, and `App.tsx` drains and
   * routes. What is actually true is that the consumer is CLASSIC's:
   * `main.tsx` mounts `App` or `NextApp` and never both, and nothing in this
   * package touches deep links — so under the new design a link is queued and
   * nothing opens it. A pane leading with §23's "opens the exact thing it
   * refers to" would be false in the design a reader is reading it in.
   *
   * This scans the package for a consumer instead of trusting the comment.
   * Whoever wires the drain into this tree fails this test, and adds the pane
   * and its nav entry in the same commit (#370).
   */
  it("has no deep-link consumer of its own, which is why the section is absent", () => {
    const root = join(dirname(fileURLToPath(import.meta.url)), "..");
    const consumers: string[] = [];
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) {
          walk(path);
          continue;
        }
        if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) continue;
        // Comments stripped: this very absence is discussed in prose in
        // `Settings.tsx`, and a scan that read prose would find itself.
        const source = readFileSync(path, "utf8").replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "");
        if (/parseDeepLink|take_pending_deep_links|deep-link-pending/.test(source)) {
          consumers.push(relative(root, path));
        }
      }
    };
    walk(root);
    expect(consumers).toEqual([]);
  });

  it("names the rail Settings and holds it at the design's 196px", () => {
    paint();
    const rail = screen.getByRole("complementary", { name: "Settings" });
    expect(rail.style.width).toBe("196px");
  });

  it("lets the content column shrink, so a wide pane cannot push the rail off", () => {
    paint();
    const pane = document.querySelector('[data-slot="settings-content"]') as HTMLElement;
    // A flex item's implicit `min-width: auto` refuses to shrink below its
    // content, and the audit table is as wide as its widest target. Twice on
    // this migration a rail was pushed off the window by exactly this, and
    // jsdom can see none of it — so the property is asserted rather than seen.
    expect(pane.className).toContain("min-w-0");
  });

  it("opens on the first section and switches to the one asked for", async () => {
    const { user } = paint();
    expect(await screen.findByText(/never without confirmation/i)).toBeTruthy();
    await user.click(screen.getByRole("tab", { name: "Appearance" }));
    expect(screen.getByText(/px body text/i)).toBeTruthy();
  });

  it("stacks the three agent panes together", async () => {
    paint();
    expect(await screen.findByText(/never without confirmation/i)).toBeTruthy();
    expect(screen.getByText(/drops in-flight requests/i)).toBeTruthy();
    expect(screen.getByText(/every capability call/i)).toBeTruthy();
  });

  it("shows one section at a time", async () => {
    const { user } = paint();
    expect(await screen.findByText(/never without confirmation/i)).toBeTruthy();
    await user.click(screen.getByRole("tab", { name: "Clusters" }));
    expect(screen.queryByText(/never without confirmation/i)).toBeNull();
    expect(screen.getByRole("button", { name: /open connections/i })).toBeTruthy();
  });

  it("reaches each remaining section by its own words", async () => {
    const { user } = paint();
    await user.click(screen.getByRole("tab", { name: "Security" }));
    expect(await screen.findByRole("button", { name: "Lock now" })).toBeTruthy();
    await user.click(screen.getByRole("tab", { name: "Accessibility" }));
    expect(screen.getByText(/stops the live pulse/i)).toBeTruthy();
    await user.click(screen.getByRole("tab", { name: "Shortcuts" }));
    expect(screen.getByRole("list", { name: /keyboard shortcuts/i })).toBeTruthy();
  });

  it("marks the section on screen as the selected one, and only it", async () => {
    const { user } = paint();
    const selected = () =>
      screen
        .getAllByRole("tab")
        .filter((t) => t.getAttribute("aria-selected") === "true")
        .map((t) => t.textContent);
    expect(selected()).toEqual(["Agent & MCP"]);
    await user.click(screen.getByRole("tab", { name: "Shortcuts" }));
    expect(selected()).toEqual(["Shortcuts"]);
  });

  it("is one Tab stop, and moves between sections with the arrow keys", async () => {
    const { user } = paint();
    const tabs = screen.getAllByRole("tab") as HTMLButtonElement[];
    // Roving tabindex: the strip is one stop, and Tab from it leaves rather
    // than walking six sections. `role=tablist` promises this; a run of six
    // Tab stops with dead arrow keys is a worse control than plain buttons.
    expect(tabs.map((t) => t.tabIndex)).toEqual([0, -1, -1, -1, -1, -1]);
    tabs[0].focus();
    await user.keyboard("{ArrowDown}");
    expect(sections()[1]).toBe("Security");
    expect(document.activeElement?.textContent).toBe("Security");
    await user.keyboard("{End}");
    expect(document.activeElement?.textContent).toBe("Clusters");
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement?.textContent).toBe("Agent & MCP");
    await user.keyboard("{ArrowUp}");
    expect(document.activeElement?.textContent).toBe("Clusters");
    await user.keyboard("{Home}");
    expect(document.activeElement?.textContent).toBe("Agent & MCP");
  });

  it("hands Appearance the screens the host injected, not a list of its own", async () => {
    const { user, onSwitchToClassic } = paint();
    await user.click(screen.getByRole("tab", { name: "Appearance" }));
    expect(screen.getAllByTestId("ported-screen").map((n) => n.textContent)).toEqual(PORTED);
    await user.click(screen.getByRole("button", { name: /switch to the classic design/i }));
    expect(onSwitchToClassic).toHaveBeenCalledTimes(1);
  });

  it("raises the lock surface once the vault has actually been sealed", async () => {
    const onLocked = vi.fn();
    const { user } = paint({ onLocked });
    await user.click(screen.getByRole("tab", { name: "Security" }));
    await user.click(await screen.findByRole("button", { name: "Lock now" }));
    expect(core.vaultLock).toHaveBeenCalledTimes(1);
    expect(onLocked).toHaveBeenCalledTimes(1);
  });

  it("raises nothing when locking was refused", async () => {
    const onLocked = vi.fn();
    core.vaultLock.mockRejectedValue(new Error("there is no vault to lock"));
    const { user } = paint({ onLocked });
    await user.click(screen.getByRole("tab", { name: "Security" }));
    await user.click(await screen.findByRole("button", { name: "Lock now" }));
    expect(onLocked).not.toHaveBeenCalled();
  });

  describe("on the web", () => {
    beforeEach(() => {
      core.isTauri.mockReturnValue(false);
      // What a browser actually does with these. `getMcpToken`, `auditTail`
      // and the vault commands are direct `invoke`s from
      // `@tauri-apps/api/core` with no web half, so every one of them rejects
      // before it reaches a server. Left resolving, a pane that mounted them
      // anyway would look perfectly well here and be a run of failure alerts
      // in the real web build — which is the defect this section is about.
      const noHost = () => Promise.reject(new Error("window.__TAURI_INTERNALS__ is undefined"));
      core.getMcpToken.mockImplementation(noHost);
      core.mcpHttpStatus.mockImplementation(noHost);
      core.auditTail.mockImplementation(noHost);
    });

    it("draws no Security section, because no vault command can answer there", () => {
      paint();
      expect(sections()).toEqual(WEB_SECTIONS);
      expect(screen.queryByRole("tab", { name: "Security" })).toBeNull();
    });

    it("says why the section is missing, once, where the entry would have been", () => {
      paint();
      const rail = screen.getByRole("complementary", { name: "Settings" });
      expect(within(rail).getByTestId("no-security").textContent).toMatch(
        /desktop|srelens desktop app/i,
      );
      // Once. A sentence repeated per section is a sentence the reader learns
      // to skip, and this one is the only report of an absent control.
      expect(screen.getAllByTestId("no-security")).toHaveLength(1);
    });

    it("calls no vault command from any section a reader can reach", async () => {
      const { user } = paint();
      // Every entry the rail offers, not only the one it opens on: a Security
      // tab drawn here and left unvisited would pass an assertion made against
      // the first pane alone.
      for (const label of sections()) {
        await user.click(screen.getByRole("tab", { name: label }));
      }
      expect(await screen.findByRole("button", { name: /open connections/i })).toBeTruthy();
      expect(core.vaultBiometricStatus).not.toHaveBeenCalled();
      expect(core.vaultLock).not.toHaveBeenCalled();
    });

    it("still opens on the first section, which is not the missing one", async () => {
      paint();
      expect(await screen.findByText(/never without confirmation/i)).toBeTruthy();
      expect(screen.getByRole("tab", { name: "Agent & MCP" }).getAttribute("aria-selected")).toBe(
        "true",
      );
    });

    /**
     * `Agent & MCP` is the pane the web build OPENS ON, and two of its three
     * panels could not work there: `getMcpToken()` and `auditTail()` are
     * direct `invoke`s from `@tauri-apps/api/core`
     * (`packages/core/src/lib/mcpSecurity.ts`) with no web half, so both
     * rejected on every visit and the default pane was two failure alerts over
     * controls that can never act. The same reason `Security` is not in the
     * rail, one level down — so the same answer: the panels are not drawn, and
     * the reason is said once.
     */
    it("draws neither the server nor the audit trail, because no command behind them can answer", async () => {
      const { user } = paint();
      // Every entry, not only the one it opens on — a panel drawn under an
      // unvisited tab would pass an assertion made against the first pane.
      for (const label of sections()) {
        await user.click(screen.getByRole("tab", { name: label }));
      }
      expect(screen.queryByText(/drops in-flight requests/i)).toBeNull();
      expect(screen.queryByText(/every capability call/i)).toBeNull();
      expect(screen.queryByRole("button", { name: /start server/i })).toBeNull();
      expect(core.getMcpToken).not.toHaveBeenCalled();
      expect(core.mcpHttpStatus).not.toHaveBeenCalled();
      expect(core.auditTail).not.toHaveBeenCalled();
    });

    it("keeps the section, because what an agent may do is true on both", async () => {
      paint();
      expect(sections()).toContain("Agent & MCP");
      // `AgentAccess` is static information read from srelens's own capability
      // registry, and it is the pane a web reader most wants: what a connected
      // agent may do without asking.
      expect(await screen.findByText(/never without confirmation/i)).toBeTruthy();
      expect(screen.getAllByTestId("gated-capability").length).toBeGreaterThan(0);
    });

    it("says why the two panels are missing, once, inside the section that lost them", async () => {
      paint();
      const note = screen.getByTestId("no-agent-server");
      expect(note.textContent).toMatch(/desktop/i);
      expect(screen.getAllByTestId("no-agent-server")).toHaveLength(1);
      // And not in the rail: this is an absence WITHIN a section that is still
      // drawn, not an absent entry — the footnote by the nav is the report for
      // the latter and stays about `Security` alone.
      const rail = screen.getByRole("complementary", { name: "Settings" });
      expect(rail.contains(note)).toBe(false);
      expect(within(rail).getByTestId("no-security").textContent).not.toMatch(/audit|mcp server/i);
    });

    it("draws no failure alert on the pane it opens on", async () => {
      paint();
      expect(await screen.findByText(/never without confirmation/i)).toBeTruthy();
      // The whole shape of the defect: the default pane was two alerts. Any
      // `role="alert"` here would be one of them back.
      expect(screen.queryAllByRole("alert")).toEqual([]);
    });
  });

  describe("on the desktop", () => {
    it("draws all three agent panels, because every command behind them answers", async () => {
      paint();
      expect(await screen.findByText(/never without confirmation/i)).toBeTruthy();
      expect(screen.getByText(/drops in-flight requests/i)).toBeTruthy();
      expect(screen.getByText(/every capability call/i)).toBeTruthy();
      expect(screen.queryByTestId("no-agent-server")).toBeNull();
    });
  });
});
