import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const { getInitialThemeMock, resolvedThemeModeMock, applyThemeMock } = vi.hoisted(() => ({
  getInitialThemeMock: vi.fn(),
  resolvedThemeModeMock: vi.fn(),
  applyThemeMock: vi.fn(),
}));
vi.mock("./ui/theme", () => ({
  getInitialTheme: getInitialThemeMock,
  resolvedThemeMode: resolvedThemeModeMock,
  applyTheme: applyThemeMock,
}));

import { applyNextDesignTheme, toggleNextDesignTheme } from "./design";

beforeEach(() => {
  delete document.documentElement.dataset.theme;
  getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
  resolvedThemeModeMock.mockImplementation((mode: string) => mode);
  applyThemeMock.mockReset();
});

describe("applyNextDesignTheme", () => {
  it("carries a dark preference into the new design's convention", () => {
    // The classic default IS dark, so getting this wrong sends most users to a
    // bright UI on every launch. The two designs disagree about data-theme:
    // classic puts the palette name there, ui-next reads it as the mode.
    applyNextDesignTheme();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("uses the absence of the attribute for light, matching :root", () => {
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "light" });
    document.documentElement.dataset.theme = "dark";
    applyNextDesignTheme();
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("resolves a system preference rather than passing it through", () => {
    // "system" is not one of ui-next's selectors; leaving it would match none
    // of them and silently fall back to light.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    resolvedThemeModeMock.mockReturnValue("dark");
    applyNextDesignTheme();
    expect(resolvedThemeModeMock).toHaveBeenCalledWith("system");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("never leaves the classic palette name where ui-next reads a mode", () => {
    // `data-theme="slate"` matches none of ui-next's blocks.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    applyNextDesignTheme();
    expect(document.documentElement.dataset.theme).not.toBe("slate");
  });

  it("follows the OS while the app is open, for a system preference", () => {
    // The classic tree has a matchMedia effect for this; without an equivalent
    // the new tree sat on a stale palette until a reload. (#314 review)
    const listeners: Array<() => void> = [];
    const removed: Array<() => void> = [];
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener: (_: string, fn: () => void) => listeners.push(fn),
      removeEventListener: (_: string, fn: () => void) => removed.push(fn),
    }));
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    resolvedThemeModeMock.mockReturnValue("light");

    const stop = applyNextDesignTheme();
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(listeners).toHaveLength(1);

    // The OS goes dark; the attribute must follow without a reload.
    resolvedThemeModeMock.mockReturnValue("dark");
    listeners[0]();
    expect(document.documentElement.dataset.theme).toBe("dark");

    stop();
    expect(removed).toHaveLength(1);
    vi.unstubAllGlobals();
  });

  /**
   * The reader has named a theme, so the OS no longer gets a vote.
   *
   * This function's reading is DERIVED — from classic's light/dark preference
   * — and it knows only two of ui-next's five themes: it writes
   * `data-theme="dark"` or takes the attribute off. So the listener it arms
   * for a reader on `system` was overwriting a theme chosen in the Appearance
   * pane: Midnight became plain dark, Paper became bare light, for the rest of
   * the session and with nothing on screen to say why. (#373 review)
   *
   * The predicate is passed in rather than read here, because only ui-next's
   * stored appearance record can tell a chosen theme from a derived one —
   * `dark` is both a derivation and a named theme, and a bare root is both "no
   * reading" and a chosen Light. That record lives on the chunk the new design
   * is loaded from, so the entry hands it down.
   */
  it("leaves a named theme alone, and arms nothing for the OS to change", () => {
    const listeners: Array<() => void> = [];
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener: (_: string, fn: () => void) => listeners.push(fn),
      removeEventListener: () => {},
    }));
    // Exactly the state boot leaves: mode `system`, and the stored Paper
    // already on the root from `applyStoredAppearance`.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    resolvedThemeModeMock.mockReturnValue("dark");
    document.documentElement.dataset.theme = "paper";

    const stop = applyNextDesignTheme(() => true);

    expect(document.documentElement.dataset.theme).toBe("paper");
    expect(listeners).toHaveLength(0);
    // Still a stop function, so the caller needs no branch of its own.
    expect(() => stop()).not.toThrow();
    vi.unstubAllGlobals();
  });

  it("leaves a chosen Light bare rather than re-deriving the OS's dark", () => {
    // The case the document cannot answer: chosen Light and "nothing chosen"
    // are the same root, and re-deriving would put dark on top of a reader who
    // asked for light.
    vi.stubGlobal("matchMedia", () => ({
      matches: true,
      addEventListener: () => {},
      removeEventListener: () => {},
    }));
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    resolvedThemeModeMock.mockReturnValue("dark");
    delete document.documentElement.dataset.theme;

    applyNextDesignTheme(() => true);

    expect(document.documentElement.dataset.theme).toBeUndefined();
    vi.unstubAllGlobals();
  });

  it("stands down when a theme is named after the listener is armed", () => {
    // The mid-session half, and the more common one: a reader on `system` who
    // has never opened the pane boots with the listener armed, then picks
    // Midnight. There is no reload, so this is the same listener — it has to
    // re-read the predicate on every change rather than only at arm time.
    const listeners: Array<() => void> = [];
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener: (_: string, fn: () => void) => listeners.push(fn),
      removeEventListener: () => {},
    }));
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    resolvedThemeModeMock.mockReturnValue("light");

    let named = false;
    applyNextDesignTheme(() => named);
    expect(listeners).toHaveLength(1);

    // The Appearance pane writes the attribute and records the choice.
    document.documentElement.dataset.theme = "midnight";
    named = true;

    // Dusk. The OS goes dark and this listener must keep its hands off.
    resolvedThemeModeMock.mockReturnValue("dark");
    listeners[0]();
    expect(document.documentElement.dataset.theme).toBe("midnight");
    vi.unstubAllGlobals();
  });

  it("still follows the OS for a reader who has named nothing", () => {
    // The other side of the rule: naming no theme is how "follow the OS" is
    // said, since the pane offers no `System` entry of its own. A predicate
    // that answered yes too eagerly would take that away.
    const listeners: Array<() => void> = [];
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener: (_: string, fn: () => void) => listeners.push(fn),
      removeEventListener: () => {},
    }));
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    resolvedThemeModeMock.mockReturnValue("light");

    applyNextDesignTheme(() => false);
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(listeners).toHaveLength(1);

    resolvedThemeModeMock.mockReturnValue("dark");
    listeners[0]();
    expect(document.documentElement.dataset.theme).toBe("dark");
    vi.unstubAllGlobals();
  });

  it("does not subscribe when the mode is fixed", () => {
    const listeners: Array<() => void> = [];
    vi.stubGlobal("matchMedia", () => ({
      matches: false,
      addEventListener: (_: string, fn: () => void) => listeners.push(fn),
      removeEventListener: () => {},
    }));
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    applyNextDesignTheme();
    expect(listeners).toHaveLength(0);
    vi.unstubAllGlobals();
  });
});

describe("toggleNextDesignTheme", () => {
  it("flips dark to light through classic's applyTheme", () => {
    // R-E: one stored preference drives both designs, so the toggle goes
    // through the same applyTheme Settings uses rather than a private copy.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    toggleNextDesignTheme();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "light" });
  });

  it("flips light to dark", () => {
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "light" });
    toggleNextDesignTheme();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "dark" });
  });

  it("resolves system before flipping, so a resolved-dark system goes light", () => {
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    resolvedThemeModeMock.mockReturnValue("dark");
    toggleNextDesignTheme();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "light" });
  });

  it("re-asserts the new design's data-theme convention after applying", () => {
    // applyTheme writes classic's conventions (data-theme = palette name,
    // data-theme-mode = mode); ui-next reads data-theme as the mode itself.
    // Without the re-assert, one click left both designs reading garbage. The
    // mock persists like the real one, since the re-assert reads the stored
    // preference back through getInitialTheme.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    applyThemeMock.mockImplementation((t: { name: string; mode: string }) => {
      getInitialThemeMock.mockReturnValue(t);
    });
    document.documentElement.dataset.theme = "dark";
    toggleNextDesignTheme();
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});

/**
 * The wiring itself, asserted on the entry's source.
 *
 * `main.tsx` cannot be imported from a test — it installs the notifier and
 * boots the app — and this sequence exists nowhere else, so it is checked
 * where it lives. The same reason and the same technique as
 * `one-stylesheet.test.ts`.
 */
describe("the entry's theme wiring", () => {
  const main = readFileSync(join(__dirname, "main.tsx"), "utf8");

  /**
   * `main.tsx` with its comments removed. Every name asserted below also
   * appears in that file's prose, so matching the file as written would let a
   * comment stand in for the code — which is how a pin passes over a defect it
   * was written to catch. Comments there are whole-line `//`.
   */
  const code = main
    .split("\n")
    .filter((line) => !/^\s*(\/\/|\/\*|\*)/.test(line))
    .join("\n");

  it("strips the entry's comments without eating its code", () => {
    // The stripper is load-bearing for every assertion below, so it is checked
    // in both directions: code survives, prose does not.
    expect(code).toContain("void bootstrap(container);");
    expect(code).toContain("applyStoredAppearance();");
    expect(main).toContain("Verified against a real build");
    expect(code).not.toContain("Verified against a real build");
  });

  it("keeps applyNextDesignTheme's stop function rather than discarding it", () => {
    // Discarding it is the whole defect: the derived follower stayed armed
    // over the reader's chosen theme for the rest of the session.
    expect(code).toMatch(/\bconst\s+\w+\s*=\s*applyNextDesignTheme\(\)\s*;/);
  });

  it("stands that follower down before the stored theme goes on", () => {
    const name = /\bconst\s+(\w+)\s*=\s*applyNextDesignTheme\(\)\s*;/.exec(code)?.[1];
    expect(name, "main.tsx does not name applyNextDesignTheme's stop function").toBeTruthy();
    const stopAt = code.indexOf(`${name as string}();`);
    const storedAt = code.indexOf("applyStoredAppearance();");
    expect(stopAt, `main.tsx never calls ${name as string}()`).toBeGreaterThan(-1);
    expect(storedAt).toBeGreaterThan(-1);
    expect(stopAt).toBeLessThan(storedAt);
  });

  it("re-arms the follower behind the reader's own record", () => {
    // Stopping it and stopping there would leave a reader who has named no
    // theme unable to follow their OS at all, which is the only way that
    // reader has of saying "follow the OS".
    expect(code).toMatch(/applyNextDesignTheme\(\s*hasChosenTheme\s*\)/);
  });

  it("takes the predicate off the chunk the new design already rides", () => {
    // A static import of @srelens/ui-next from the entry drags the whole new
    // tree into the chunk a CLASSIC boot downloads — the same wall
    // `applyStoredAppearance` hit, answered the same way.
    expect(main).not.toMatch(/^import\s*\{[^}]*\bhasChosenTheme\b/m);
    expect(code).toMatch(
      /const\s*\[\s*,\s*\{[^}]*\bhasChosenTheme\b[^}]*\}\s*\]\s*=\s*await\s+Promise\.all\(/,
    );
  });
});
