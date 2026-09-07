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

import { DARK_NEXT_THEMES, applyNextDesignTheme, toggleNextDesignTheme } from "./design";

beforeEach(() => {
  delete document.documentElement.dataset.theme;
  getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
  resolvedThemeModeMock.mockImplementation((mode: string) => mode);
  applyThemeMock.mockReset();
  // The real `applyTheme` PERSISTS, so `getInitialTheme` reads back what it
  // last wrote. A mock that forgets cannot express this round's defect: the old
  // toggle's second step re-derived the root attribute from that store, so a
  // mock answering `dark` for ever left a correct-looking attribute behind for
  // the wrong reason — and two pins written against it passed over the bug.
  // The one test about a store that REFUSES the write overrides this.
  applyThemeMock.mockImplementation((theme: { name: string; mode: string }) => {
    getInitialThemeMock.mockReturnValue(theme);
  });
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
  /** What `Chrome` stores right after the handler: `readRootTheme()`. */
  function recorded(): string {
    return document.documentElement.dataset.theme ?? "light";
  }

  it("flips dark to light through classic's applyTheme", () => {
    // R-E: one stored preference drives both designs, so the toggle goes
    // through the same applyTheme Settings uses rather than a private copy.
    //
    // The root is set as well as the mode, because that is the state a
    // classic-dark reader is actually in — boot's own pass writes `dark` there
    // — and because the direction is read off the root now, not off the mode.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    document.documentElement.dataset.theme = "dark";
    toggleNextDesignTheme();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "light" });
  });

  it("flips light to dark", () => {
    // Bare root: what boot leaves for a classic-light reader, and ui-next's
    // spelling of Light.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "light" });
    delete document.documentElement.dataset.theme;
    toggleNextDesignTheme();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "dark" });
  });

  it("pins a system preference to the side it flipped to", () => {
    // An explicit click is an explicit answer, so `system` must not survive it
    // — the next OS change would otherwise walk back over the choice.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "system" });
    document.documentElement.dataset.theme = "dark";
    toggleNextDesignTheme();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "light" });
  });

  /**
   * The finding: the direction came from classic's stored mode, and a named
   * theme never writes it.
   *
   * `pickTheme` lives in `packages/ui-next` and classic's mode store lives
   * here, in the package that DEPENDS on it, so the pane cannot write that
   * mode — a static import upward is a cycle across the boundary, the same
   * wall the design toggle and `brandMarkSrc` both hit. So after a reader on
   * classic-dark picks Paper the window is light while the mode still says
   * `dark`, and one click flipped that stale `dark` to `light`: Paper was
   * deleted down to bare Light, the reader having asked to go DARK, with
   * nothing on screen to say what happened to the theme they named.
   *
   * The direction comes off the document now — the lightness of the theme the
   * reader can actually see. (#373 review, round 5)
   */
  it("flips away from a light theme the reader named, not classic's stale mode", () => {
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    document.documentElement.dataset.theme = "paper";

    toggleNextDesignTheme();

    expect(document.documentElement.dataset.theme).toBe("dark");
    // And classic is left agreeing with what is on screen, so the two designs
    // do not disagree about light and dark at the next boot.
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "dark" });
  });

  it("flips High contrast to dark, since its ground is white", () => {
    // The second light theme, and the one the old code got wrong in the same
    // direction: `--surface: #ffffff` with black ink.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    document.documentElement.dataset.theme = "contrast";

    toggleNextDesignTheme();

    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "dark" });
  });

  it("flips a named Midnight to light rather than deeper into the dark", () => {
    // The other way round: a dark theme named while classic's mode says light.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "light" });
    document.documentElement.dataset.theme = "midnight";

    toggleNextDesignTheme();

    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "light" });
  });

  it("does not read the mode back through storage, which may have refused it", () => {
    // `applyTheme` persists best-effort and carries on when storage throws, so
    // re-deriving the attribute from `getInitialTheme()` afterwards put the OLD
    // side straight back on the root for exactly the reader whose device would
    // not save the preference — one click, nothing changes, no explanation. The
    // attribute is written from the side just chosen instead.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    document.documentElement.dataset.theme = "dark";
    applyThemeMock.mockImplementation(() => {
      /* storage refused; the stored preference is left as it was */
    });

    toggleNextDesignTheme();

    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(applyThemeMock).toHaveBeenCalledWith({ name: "slate", mode: "light" });
  });

  /**
   * The round-3b interaction, in one place.
   *
   * `Chrome` calls `rememberTheme()` straight after this handler, which stores
   * `readRootTheme()` — so whatever this function leaves on the root is what
   * the reader's record becomes, and `hasChosenTheme` then stands the OS
   * follower down on that value for every launch after. While the direction
   * came from classic's mode, the record followed the wrong outcome too: Paper
   * was recorded as a chosen `light` the reader never asked for.
   *
   * Reproduced rather than imported: `readRootTheme` lives in ui-next, and a
   * static import of that package here would drag the whole new tree into the
   * chunk a classic boot downloads.
   */
  it("leaves the record on the side the reader can now see", () => {
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    document.documentElement.dataset.theme = "paper";

    toggleNextDesignTheme();

    expect(recorded()).toBe("dark");
  });

  /**
   * The invariant the fix has to keep, for all five themes at once: after the
   * click, classic's mode and ui-next's attribute say the same thing. Not the
   * pin for the direction — the old code satisfied this too, because it wrote
   * both sides from the same wrong reading — so it is here to stop a fix that
   * corrects the attribute and leaves classic's mode behind.
   */
  it("leaves both designs' conventions agreeing about light and dark", () => {
    const states: ReadonlyArray<[string | undefined, string]> = [
      ["paper", "dark"],
      ["contrast", "system"],
      ["midnight", "light"],
      ["dark", "light"],
      [undefined, "dark"],
    ];
    for (const [theme, mode] of states) {
      // Cleared, not reset: a reset would drop the persisting implementation
      // above and put this loop back on a store that forgets, which is how a
      // pin ends up passing over the very re-derivation it was written for.
      applyThemeMock.mockClear();
      getInitialThemeMock.mockReturnValue({ name: "slate", mode });
      if (theme === undefined) delete document.documentElement.dataset.theme;
      else document.documentElement.dataset.theme = theme;

      toggleNextDesignTheme();

      const written = applyThemeMock.mock.calls.at(-1)?.[0] as { mode: string } | undefined;
      expect(written, `no applyTheme call for ${String(theme)}`).toBeDefined();
      expect(written?.mode, `classic disagrees with the root, from ${String(theme)}`).toBe(
        recorded() === "dark" ? "dark" : "light",
      );
    }
  });

  it("re-asserts the new design's data-theme convention after applying", () => {
    // applyTheme writes classic's conventions (data-theme = palette name,
    // data-theme-mode = mode); ui-next reads data-theme as the mode itself.
    // Without the re-assert, one click left both designs reading garbage — the
    // root wearing `slate`, which matches no block in either stylesheet.
    getInitialThemeMock.mockReturnValue({ name: "slate", mode: "dark" });
    document.documentElement.dataset.theme = "dark";
    toggleNextDesignTheme();
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});

/**
 * Where the toggle's idea of "dark" comes from.
 *
 * The button has to know which side of light/dark each of ui-next's five themes
 * is on before it can flip away from one, and that answer is the STYLESHEET's:
 * whatever `tokens.css` paints is what the reader sees, so a list written from
 * memory here would be a second opinion that could silently disagree with it.
 * `apps/desktop` cannot import the kit — that would put the whole kit in the
 * chunk a classic boot downloads — so the set is copied into `design.ts` and
 * pinned here, against the stylesheet, twice over: once from the grounds each
 * theme declares, and once from the stylesheet's own grouping of the dark ones.
 *
 * The same technique and the same reason as the kit's `tokens-only` guard and
 * the entry wiring below: jsdom attaches no stylesheet, so nothing about the
 * real palette can be observed at runtime here.
 */
describe("the toggle's dark side", () => {
  /** The stylesheet, comments removed — see the self-check below. */
  const TOKENS = readFileSync(
    join(__dirname, "../../../packages/ui-kit/src/styles/tokens.css"),
    "utf8",
  ).replace(/\/\*[\s\S]*?\*\//g, "");

  /** Every theme id the stylesheet keys a rule on. */
  const NAMED = [...new Set([...TOKENS.matchAll(/\[data-theme="([a-z]+)"\]/g)].map((m) => m[1]))];

  /** The selector a theme's own token block is written under. */
  function selectorFor(id: string): string {
    // `light` is the bare `:root`, which is why `writeAxis` spells it as the
    // ABSENCE of the attribute — there is no `[data-theme="light"]` rule.
    return id === "light" ? ":root" : `[data-theme="${id}"]`;
  }

  /** One rule's colour declarations, found by its exact selector. */
  function declarations(id: string): Record<string, string> {
    const selector = selectorFor(id);
    const at = TOKENS.indexOf(`${selector} {`);
    expect(at, `tokens.css has no ${selector} block`).toBeGreaterThan(-1);
    const body = TOKENS.slice(at, TOKENS.indexOf("}", at));
    const out: Record<string, string> = {};
    for (const [, name, value] of body.matchAll(/(--[a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) {
      out[name] = value;
    }
    return out;
  }

  /** WCAG relative luminance, as the kit's own token guard computes it. */
  function luminance(hex: string): number {
    const channel = (pair: string) => {
      const v = parseInt(pair, 16) / 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    };
    const bare = hex.replace("#", "");
    const [r, g, b] = [bare.slice(0, 2), bare.slice(2, 4), bare.slice(4, 6)].map(channel);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }

  /**
   * The dark side, from what each theme paints: a dark theme is one whose
   * ground is darker than the text on it. Read off the declarations rather than
   * matched by name, so a theme renamed or added lands on the right side by
   * itself.
   */
  function darkGrounds(): Set<string> {
    const dark = new Set<string>();
    for (const id of ["light", ...NAMED]) {
      const t = declarations(id);
      if (luminance(t["--canvas"]) < luminance(t["--ink"])) dark.add(id);
    }
    return dark;
  }

  /**
   * The dark side, from the stylesheet's own grouping. Every rule that is for
   * the dark grounds — the mark palette, each accent override — lists them
   * together, and the light ones are left to the rules above it. An
   * independent reading of the same file, so the two cannot drift apart
   * unnoticed either.
   */
  function groupedDarkIds(): Set<string> {
    const ids = new Set<string>();
    for (const [, selector] of TOKENS.matchAll(/([^{}]*)\{[^{}]*\}/g)) {
      const listed = [...selector.matchAll(/\[data-theme="([a-z]+)"\]/g)].map((m) => m[1]);
      if (new Set(listed).size > 1) for (const id of listed) ids.add(id);
    }
    return ids;
  }

  it("strips the stylesheet's comments without eating its declarations", () => {
    // Load-bearing for everything below, so checked both ways: rules survive,
    // prose does not — including the one hex that lives inside a comment and
    // would otherwise be read as a token of the block it sits in.
    expect(TOKENS).toContain('[data-theme="paper"] {');
    expect(TOKENS).toContain("--canvas: #f5f4f8;");
    expect(TOKENS).not.toContain("warm light, easier under office lighting");
    expect(TOKENS).not.toContain("#565656");
  });

  it("finds a ground and an ink for every theme the stylesheet keys a rule on", () => {
    // Anti-vacuity: a parse that found nothing would agree with any set at all.
    expect(NAMED.length).toBeGreaterThan(1);
    for (const id of ["light", ...NAMED]) {
      const t = declarations(id);
      expect(t["--canvas"], `${id} declares no --canvas`).toMatch(/^#[0-9a-f]{6}$/i);
      expect(t["--ink"], `${id} declares no --ink`).toMatch(/^#[0-9a-f]{6}$/i);
    }
    // And the derivation separates them rather than sorting every theme onto
    // one side, which would also agree with a set that had drifted.
    const dark = darkGrounds();
    expect(dark.size).toBeGreaterThan(0);
    expect(dark.size).toBeLessThan(NAMED.length + 1);
  });

  it("is the set of grounds tokens.css actually paints dark", () => {
    // Add a theme to `tokens.css`, or change one theme's ground, and this fails
    // here instead of the button quietly treating it as light.
    expect(DARK_NEXT_THEMES).toEqual(darkGrounds());
  });

  it("is the set the stylesheet itself groups as the dark side", () => {
    expect(DARK_NEXT_THEMES).toEqual(groupedDarkIds());
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
