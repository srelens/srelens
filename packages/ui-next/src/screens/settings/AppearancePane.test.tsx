import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * This file's own directory. `import.meta.url` is not a file URL under the
 * vite-node transform, and `__dirname` is what the kit's `tokens-only` guard
 * already uses to read source off disk.
 */
const HERE = __dirname;

const core = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  applyUiScale: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { UI_SCALE, getUiScale, setUiScale } from "@srelens/core";
import {
  APPEARANCE_KEY,
  ACCENTS,
  DENSITIES,
  THEMES,
  ZOOM_STEPS,
  AppearancePane,
  applyStoredAppearance,
  hasChosenTheme,
} from "./AppearancePane";
import { rememberTheme } from "../../lib/appearance";

/**
 * The stylesheet that actually defines the themes, accents and densities this
 * pane offers. Read as text rather than trusted: jsdom attaches no stylesheet,
 * so a card for a theme nobody ever wrote a token block for would render, look
 * selected, and change nothing on screen. This is the check the brief asks for
 * — "what the app actually supports" — made executable.
 */
const TOKENS = readFileSync(join(HERE, "../../../../ui-kit/src/styles/tokens.css"), "utf8");

const PANE_SOURCE = readFileSync(join(HERE, "AppearancePane.tsx"), "utf8");

/**
 * Two lists of DIFFERENT lengths whose entries share no substring with
 * anything this pane could write on its own — no "log", no "cluster", no
 * "resource". A fixture that repeats a word the component already has is how a
 * component that invents its own list passes; a fixture of the real
 * PORTED_SCREENS length is how a hardcoded count passes.
 */
const PORTED_THREE = ["Aardvark ledger", "Basalt tally", "Cinnabar dial"];
const PORTED_FIVE = [...PORTED_THREE, "Dovetail rack", "Etruscan seam"];

function paint(props: Partial<Parameters<typeof AppearancePane>[0]> = {}) {
  const onSwitchToClassic = vi.fn();
  render(<AppearancePane ported={PORTED_THREE} onSwitchToClassic={onSwitchToClassic} {...props} />);
  return { onSwitchToClassic, user: userEvent.setup() };
}

function rootAttributes(): Record<string, string | undefined> {
  const root = document.documentElement;
  return {
    theme: root.getAttribute("data-theme") ?? undefined,
    accent: root.getAttribute("data-accent") ?? undefined,
    density: root.getAttribute("data-density") ?? undefined,
  };
}

function stored(): unknown {
  const raw = localStorage.getItem(APPEARANCE_KEY);
  return raw === null ? null : JSON.parse(raw);
}

describe("AppearancePane", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.isTauri.mockReturnValue(true);
  });

  // The pane writes on the document root, which outlives a React tree; without
  // this one test's Midnight is the next test's starting state.
  afterEach(() => {
    const root = document.documentElement;
    for (const name of ["data-theme", "data-accent", "data-density"]) root.removeAttribute(name);
    setUiScale(UI_SCALE.DEFAULT);
    localStorage.clear();
  });

  describe("theme", () => {
    it("offers every theme the design names, in order", () => {
      paint();
      expect(screen.getAllByTestId("theme-label").map((label) => label.textContent)).toEqual([
        "Light",
        "Paper",
        "Dark",
        "Midnight",
        "High contrast",
      ]);
    });

    it("offers no theme the stylesheet cannot draw", () => {
      for (const theme of THEMES) {
        // `light` is the bare `:root` block — the absence of the attribute — so
        // it is the one id with no selector of its own, and the pane must take
        // the attribute OFF for it rather than write `data-theme="light"`.
        if (theme.id === "light") {
          expect(TOKENS).not.toContain(`[data-theme="light"]`);
          continue;
        }
        expect(TOKENS, `no token block for the ${theme.id} theme`).toContain(
          `[data-theme="${theme.id}"]`,
        );
      }
    });

    it("puts the chosen theme on the document root and remembers it", async () => {
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: /midnight/i }));
      expect(rootAttributes().theme).toBe("midnight");
      expect(stored()).toMatchObject({ theme: "midnight" });
    });

    it("takes the attribute off again for Light, which the stylesheet draws bare", async () => {
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: /midnight/i }));
      await user.click(screen.getByRole("radio", { name: /^light/i }));
      expect(rootAttributes().theme).toBeUndefined();
      expect(stored()).toMatchObject({ theme: "light" });
    });

    it("shows what the document is wearing, not what this pane last wrote", () => {
      // Boot writes `data-theme` from the stored light/dark preference
      // (`applyNextDesignTheme`), and the titlebar's theme button overwrites it
      // mid-session. A pane that trusted its own store would sit there showing
      // Midnight over a document that is plainly dark.
      document.documentElement.setAttribute("data-theme", "dark");
      paint();
      // `.checked` rather than an aria attribute: these are real radio inputs,
      // so the browser owns the state and there is nothing to mirror.
      expect((screen.getByRole("radio", { name: /^dark/i }) as HTMLInputElement).checked).toBe(true);
      expect(
        (screen.getByRole("radio", { name: /midnight/i }) as HTMLInputElement).checked,
      ).toBe(false);
    });
  });

  describe("accent", () => {
    it("offers every accent the design names, in order", () => {
      paint();
      expect(screen.getAllByTestId("accent-label").map((label) => label.textContent)).toEqual([
        "Violet",
        "Blue",
        "Teal",
        "Amber",
        "Rose",
      ]);
    });

    it("offers no accent the stylesheet cannot draw", () => {
      for (const accent of ACCENTS) {
        // Violet is `--accent` as declared on `:root`, so like `light` it is an
        // absence rather than a selector.
        if (accent.id === "violet") {
          expect(TOKENS).not.toContain(`[data-accent="violet"]`);
          continue;
        }
        expect(TOKENS, `no token block for the ${accent.id} accent`).toContain(
          `[data-accent="${accent.id}"]`,
        );
      }
    });

    it("puts the chosen accent on the document root and remembers it", async () => {
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: /teal/i }));
      expect(rootAttributes().accent).toBe("teal");
      expect(stored()).toMatchObject({ accent: "teal" });
    });

    it("paints each swatch from that accent's own token", () => {
      paint();
      // The swatch carries the attribute the token block is keyed on, which is
      // the only way five different accents can be drawn without five hex
      // literals in this file. Violet is the bare `:root` value, so it carries
      // no attribute — the same absence the stylesheet expresses.
      const swatches = screen.getAllByTestId("accent-swatch");
      expect(swatches.map((s) => s.getAttribute("data-accent"))).toEqual([
        null,
        "blue",
        "teal",
        "amber",
        "rose",
      ]);
    });
  });

  describe("density", () => {
    it("offers the three densities the stylesheet defines", () => {
      paint();
      expect(screen.getAllByTestId("density-label").map((label) => label.textContent)).toEqual([
        "Compact",
        "Default",
        "Comfortable",
      ]);
      for (const density of DENSITIES) {
        if (density.id === "default") {
          expect(TOKENS).not.toContain(`[data-density="default"]`);
          continue;
        }
        expect(TOKENS, `no token block for ${density.id} density`).toContain(
          `[data-density="${density.id}"]`,
        );
      }
    });

    it("puts the chosen density on the document root and remembers it", async () => {
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: /comfortable/i }));
      expect(rootAttributes().density).toBe("comfortable");
      expect(stored()).toMatchObject({ density: "comfortable" });
    });

    it("says density moves the rows, and claims no text size for it", () => {
      paint();
      // §23's density hints read `12px text`, `13px text`, `14px text`. The
      // `[data-density]` blocks set `--row-h`, `--pad-y` and `--pane-head-h`
      // and no font size at all, so that copy would be the migration's
      // signature defect: a sentence claiming more than srelens does.
      const hint = screen.getByTestId("density-hint").textContent ?? "";
      expect(hint).toMatch(/row/i);
      expect(hint).not.toMatch(/\d+\s*px text/i);
    });
  });

  describe("interface zoom", () => {
    it("offers exactly the scales core supports", () => {
      paint();
      const offered = screen.getAllByTestId("zoom-label").map((l) => l.textContent);
      expect(offered).toEqual(ZOOM_STEPS.map((percent) => `${percent}%`));
      // Derived, not transcribed: every option must be a value `setUiScale`
      // stores unchanged, and both ends of core's range must be reachable.
      for (const percent of ZOOM_STEPS) expect(setUiScale(percent)).toBe(percent);
      expect(offered).toContain(`${UI_SCALE.MIN}%`);
      expect(offered).toContain(`${UI_SCALE.MAX}%`);
    });

    it("says what the current zoom means in pixels", () => {
      paint();
      expect(screen.getByText(/px body text/i)).toBeTruthy();
    });

    it("persists a picked zoom and asks the webview for it", async () => {
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: `${UI_SCALE.MIN}%` }));
      expect(getUiScale()).toBe(UI_SCALE.MIN);
      expect(core.applyUiScale).toHaveBeenCalledWith(UI_SCALE.MIN);
    });

    it("names the chord that does the same thing, from the bindings that exist", () => {
      paint();
      // Read from `lib/shortcuts.ts` rather than typed out, so a rebound zoom
      // key cannot leave this hint describing a chord nothing listens for.
      expect(screen.getByTestId("zoom-hint").textContent).toMatch(/⌘=|Ctrl\+=/);
    });

    it("offers no zoom control on the web, where the browser's own zoom applies", () => {
      core.isTauri.mockReturnValue(false);
      paint();
      expect(screen.queryAllByTestId("zoom-label")).toHaveLength(0);
      expect(screen.getByText(/browser/i)).toBeTruthy();
    });
  });

  describe("the way back to the old design", () => {
    it("offers it", async () => {
      const { user, onSwitchToClassic } = paint();
      await user.click(screen.getByRole("button", { name: /classic/i }));
      expect(onSwitchToClassic).toHaveBeenCalledTimes(1);
    });

    it("names the screens that have been ported", () => {
      paint();
      expect(screen.getAllByTestId("ported-screen").map((li) => li.textContent)).toEqual(
        PORTED_THREE,
      );
    });

    it("follows the list it is given rather than one of its own", () => {
      paint({ ported: PORTED_FIVE });
      expect(screen.getAllByTestId("ported-screen").map((li) => li.textContent)).toEqual(
        PORTED_FIVE,
      );
    });

    it("still offers the way out when nothing has been ported", () => {
      paint({ ported: [] });
      expect(screen.queryAllByTestId("ported-screen")).toHaveLength(0);
      expect(screen.getByRole("button", { name: /classic/i })).toBeTruthy();
    });

    it("says in the source why the design does not draw this", () => {
      // The mock is drawn as of step 11, after the toggle is deleted. Without
      // this note someone "corrects" the pane against §23 and takes the only
      // way back to a working design with it.
      expect(PANE_SOURCE).toMatch(/step 11/i);
    });
  });

  describe("the boot seam", () => {
    it("applies a stored appearance to the root", () => {
      localStorage.setItem(
        APPEARANCE_KEY,
        JSON.stringify({ theme: "paper", accent: "rose", density: "compact" }),
      );
      applyStoredAppearance();
      expect(rootAttributes()).toEqual({ theme: "paper", accent: "rose", density: "compact" });
    });

    it("leaves the root bare for the defaults, and for a document it cannot read", () => {
      localStorage.setItem(APPEARANCE_KEY, "{ not json");
      applyStoredAppearance();
      expect(rootAttributes()).toEqual({
        theme: undefined,
        accent: undefined,
        density: undefined,
      });
    });

    it("leaves an axis nobody has chosen exactly as boot left it", () => {
      // `applyNextDesignTheme()` in apps/desktop/src/design.ts runs FIRST and
      // puts data-theme="dark" on the root for anyone whose classic preference
      // resolves dark — which is the default. Writing every axis from the
      // defaults here would spell theme "light", and light is the ABSENCE of
      // the attribute, so this pass would strip that dark back off and the new
      // design would boot light for almost every reader.
      document.documentElement.setAttribute("data-theme", "dark");
      applyStoredAppearance();
      expect(rootAttributes()).toEqual({
        theme: "dark",
        accent: undefined,
        density: undefined,
      });
    });

    it("still restores every axis for a reader who has chosen, over what boot set", () => {
      // The other half of the rule above: a document `remember` wrote always
      // carries all three axes, so a stored choice wins outright — including a
      // stored light over the dark boot just set.
      document.documentElement.setAttribute("data-theme", "dark");
      localStorage.setItem(
        APPEARANCE_KEY,
        JSON.stringify({ theme: "light", accent: "teal", density: "comfortable" }),
      );
      applyStoredAppearance();
      expect(rootAttributes()).toEqual({
        theme: undefined,
        accent: "teal",
        density: "comfortable",
      });
    });

    /**
     * Finding 7. `remember` read the three axes off the DOCUMENT, so choosing
     * an accent stored whatever `data-theme` happened to be there — and boot
     * (`applyNextDesignTheme()`) puts dark there for every reader whose classic
     * preference resolves dark, which is the default. The stray value then won
     * at the next launch, because `applyStoredAppearance` runs after boot's
     * own pass.
     */
    it("stores only the axis the reader actually chose", async () => {
      document.documentElement.setAttribute("data-theme", "dark");
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: /Teal/ }));
      expect(stored()).toEqual({ accent: "teal" });
    });

    /**
     * The whole scenario, end to end: pick an accent, then use the titlebar's
     * light/dark button, then boot. The reader's most recent explicit theme
     * choice has to be the one that comes back — and it was not: boot applied
     * light and the accent-pick's stray `theme: "dark"` put dark back over it,
     * with nothing on screen to say why.
     */
    it("keeps the reader's most recent theme choice across the next launch", async () => {
      document.documentElement.setAttribute("data-theme", "dark");
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: /Teal/ }));

      // The titlebar's button, as `Chrome` calls it: the host writes the root,
      // then the record follows what landed there. Light is the bare root.
      document.documentElement.removeAttribute("data-theme");
      rememberTheme();

      // The next launch: boot's own pass puts dark on first, then the store.
      document.documentElement.setAttribute("data-theme", "dark");
      applyStoredAppearance();
      expect(rootAttributes()).toEqual({
        theme: undefined,
        accent: "teal",
        density: undefined,
      });
    });

    it("lets the pane have the last word when the pane is what wrote last", async () => {
      // The mirror of the test above, so neither writer is privileged: the same
      // two writes in the other order end on the pane's theme.
      document.documentElement.removeAttribute("data-theme");
      rememberTheme();
      const { user } = paint();
      await user.click(screen.getByRole("radio", { name: /Midnight/ }));
      document.documentElement.setAttribute("data-theme", "dark");
      applyStoredAppearance();
      expect(rootAttributes().theme).toBe("midnight");
    });

    /**
     * Finding: the OS kept a vote after the reader had named a theme.
     *
     * Boot's `applyNextDesignTheme()` arms a `prefers-color-scheme` listener
     * for a reader whose classic mode is `system`, and that listener writes
     * `data-theme` too — but it knows only `dark` and bare light, so the next
     * OS change turned a chosen Midnight into plain dark, or deleted a chosen
     * Paper down to light, for the rest of the session.
     *
     * The root cannot decide this: `dark` is BOTH a derived value and one of
     * the five named themes, and the absence of the attribute is both "no
     * reading" and a chosen Light. Only the stored record separates a choice
     * from a derivation, which is what this predicate is for.
     */
    describe("hasChosenTheme", () => {
      it("says no for a reader who has never chosen anything", () => {
        expect(hasChosenTheme()).toBe(false);
      });

      it("says no when the OS reading is on the root but nothing is stored", () => {
        // The exact boot state for the default classic preference. Answering
        // yes here would freeze every such reader out of following their OS.
        document.documentElement.setAttribute("data-theme", "dark");
        expect(hasChosenTheme()).toBe(false);
      });

      it("says no for a reader who chose an accent but no theme", async () => {
        // The per-axis rule made visible: picking Teal stores an accent and
        // nothing else, so the OS keeps its vote.
        const { user } = paint();
        await user.click(screen.getByRole("radio", { name: /Teal/ }));
        expect(stored()).toEqual({ accent: "teal" });
        expect(hasChosenTheme()).toBe(false);
      });

      it("says yes once the pane's Theme control has been used", async () => {
        const { user } = paint();
        await user.click(screen.getByRole("radio", { name: /Midnight/ }));
        expect(hasChosenTheme()).toBe(true);
      });

      it("says yes for a chosen Light, which leaves the root bare", async () => {
        // The case no reading of the document can get right: chosen Light and
        // "nothing chosen" are the same root. The record tells them apart.
        //
        // Via Midnight, because Light is what a bare root already reads as, so
        // its radio starts checked and clicking it fires no change at all.
        const { user } = paint();
        await user.click(screen.getByRole("radio", { name: /Midnight/ }));
        await user.click(screen.getByRole("radio", { name: /^Light/ }));
        expect(rootAttributes().theme).toBeUndefined();
        expect(stored()).toEqual({ theme: "light" });
        expect(hasChosenTheme()).toBe(true);
      });

      it("says yes for a chosen Dark, which looks exactly like the OS reading", () => {
        // The mirror: `data-theme="dark"` is both. Deciding on the root would
        // have been wrong for precisely the reader who asked for Dark.
        document.documentElement.setAttribute("data-theme", "dark");
        rememberTheme();
        expect(hasChosenTheme()).toBe(true);
      });

      it("says yes once the titlebar's light/dark button has been used", () => {
        // `Chrome` calls the host's toggle and then `rememberTheme`, so the
        // second writer of this axis counts as a choice too.
        document.documentElement.removeAttribute("data-theme");
        rememberTheme();
        expect(hasChosenTheme()).toBe(true);
      });

      it("says no for a stored theme this build cannot read", () => {
        // An unparsable document, or a theme id no stylesheet defines, is not
        // a choice this build can honour — so the OS keeps its vote rather
        // than the reader being pinned to whatever boot happened to derive.
        localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ theme: "neon" }));
        expect(hasChosenTheme()).toBe(false);
        localStorage.setItem(APPEARANCE_KEY, "{ not json");
        expect(hasChosenTheme()).toBe(false);
      });
    });

    it("ignores a value no stylesheet defines", () => {
      localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ theme: "neon", accent: 7 }));
      applyStoredAppearance();
      expect(rootAttributes().theme).toBeUndefined();
      expect(rootAttributes().accent).toBeUndefined();
    });
  });

  it("names no colour of its own", () => {
    // The kit's `tokens-only` guard does not reach this package, and this is
    // the one pane in the app whose subject IS colour: §23 lists the five
    // accents as hex literals, and every one of them already exists as a
    // token.
    const withoutComments = PANE_SOURCE.replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "");
    expect(withoutComments).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });
});
