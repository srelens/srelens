import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
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
  applyWindowBlur: vi.fn(async (_on: boolean) => true),
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
  GLASS_OPACITY,
  OPACITY,
  THEMES,
  ZOOM_STEPS,
  AppearancePane,
  applyStoredAppearance,
  hasChosenTheme,
} from "./AppearancePane";
import { rememberTheme, syncWindowBlur } from "../../lib/appearance";

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

/**
 * Window opacity is offered only where the native window can be seen through:
 * the desktop shell on macOS. `isApplePlatform` reads `navigator.platform`, so
 * that is what a test moves — the same thing the runtime answers from.
 */
function onPlatform(platform: string) {
  vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
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
    for (const name of ["data-theme", "data-accent", "data-density", "data-opacity"]) {
      root.removeAttribute(name);
    }
    root.style.removeProperty("--window-alpha");
    // The module remembers what it last asked the native window for, so that
    // it asks once per change. With the root bare this settles it back to
    // "no blur", or one test's see-through window would be the next one's
    // starting state.
    syncWindowBlur();
    setUiScale(UI_SCALE.DEFAULT, "next");
    localStorage.clear();
    vi.restoreAllMocks();
  });

  describe("theme", () => {
    it("offers every theme the design names, in order", () => {
      paint();
      expect(screen.getAllByTestId("theme-label").map((label) => label.textContent)).toEqual([
        "Light",
        "Paper",
        "Dark",
        "Midnight",
        "Glass",
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
      for (const percent of ZOOM_STEPS) expect(setUiScale(percent, "next")).toBe(percent);
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
      expect(getUiScale("next")).toBe(UI_SCALE.MIN);
      expect(core.applyUiScale).toHaveBeenCalledWith(UI_SCALE.MIN, "next");
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

  describe("window opacity", () => {
    const root = document.documentElement;

    function slider(): HTMLInputElement {
      return screen.getByRole("slider", { name: "Window opacity" }) as HTMLInputElement;
    }

    /**
     * Drag the slider to a value, as the browser reports it: a string.
     *
     * Awaited, because the pane reads the opacity back off the root through a
     * MutationObserver, and those deliver in a microtask: without the flush
     * the control still shows the old value, and a second drag back to it is
     * not a change at all.
     */
    async function slideTo(percent: number) {
      fireEvent.change(slider(), { target: { value: String(percent) } });
      await act(async () => {
        await Promise.resolve();
      });
    }

    beforeEach(() => {
      onPlatform("MacIntel");
      root.setAttribute("data-theme", "dark");
    });

    it("is a slider from the floor up to fully solid", () => {
      paint();
      expect(slider().min).toBe(String(OPACITY.MIN));
      expect(slider().max).toBe(String(OPACITY.MAX));
      expect(OPACITY.MAX).toBe(100);
      // The floor is where body text still reads over a white desktop — see
      // "what the text is read against" below, which is what holds it there.
      expect(OPACITY.MIN).toBe(60);
      expect(slider().value).toBe("100");
    });

    it("says the value it is set to, in words a slider alone does not", async () => {
      paint();
      await slideTo(85);
      expect(screen.getByTestId("opacity-value").textContent).toBe("85%");
      expect(slider().getAttribute("aria-valuetext")).toBe("85%");
    });

    it("lets only the dark grounds be seen through", () => {
      expect(TOKENS).toContain(`[data-theme="dark"][data-opacity]`);
      expect(TOKENS).toContain(`[data-theme="midnight"][data-opacity]`);
      expect(TOKENS).toContain(`[data-theme="glass"][data-opacity]`);
      for (const light of ["paper", "contrast"]) {
        expect(TOKENS).not.toContain(`[data-theme="${light}"][data-opacity]`);
      }
    });

    it("tints with the amount the page was given, and nothing else", () => {
      // The stylesheet reads the amount from the property this pane writes; a
      // rule that stopped reading it would leave the slider moving nothing.
      expect(TOKENS).toMatch(/--ground:\s*color-mix\([^;]*var\(--window-alpha/);
    });

    it("puts the chosen opacity on the document root and remembers it", async () => {
      paint();
      await slideTo(85);
      expect(root.getAttribute("data-opacity")).toBe("85");
      expect(root.style.getPropertyValue("--window-alpha")).toBe("85%");
      expect(stored()).toMatchObject({ opacity: 85 });
      expect(slider().value).toBe("85");
    });

    it("takes both off again at 100%, which the stylesheet draws bare", async () => {
      paint();
      await slideTo(85);
      await slideTo(100);
      expect(root.hasAttribute("data-opacity")).toBe(false);
      expect(root.style.getPropertyValue("--window-alpha")).toBe("");
      expect(stored()).toMatchObject({ opacity: 100 });
    });

    it("stores the opacity and nothing else", async () => {
      paint();
      await slideTo(90);
      expect(stored()).toEqual({ opacity: 90 });
    });

    it("blurs what is behind a see-through window, and stops when it is solid again", async () => {
      paint();
      await slideTo(80);
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(true);
      await slideTo(100);
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(false);
    });

    it("asks the window once for a whole drag, not once per position", async () => {
      paint();
      for (const percent of [99, 95, 90, 80, 70]) await slideTo(percent);
      expect(core.applyWindowBlur).toHaveBeenCalledTimes(1);
    });

    it("lets the reader turn the blur off, and remembers that", async () => {
      const { user } = paint();
      await slideTo(80);
      const blur = screen.getByRole("checkbox", { name: /blur/i }) as HTMLInputElement;
      expect(blur.checked).toBe(true);
      await user.click(blur);
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(false);
      expect(stored()).toMatchObject({ opacity: 80, blur: false });
    });

    it("has no blur to offer while the window is solid", () => {
      paint();
      expect((screen.getByRole("checkbox", { name: /blur/i }) as HTMLInputElement).disabled).toBe(true);
    });

    it("stops blurring behind a theme that paints the page solid, and keeps the opacity", async () => {
      const { user } = paint();
      await slideTo(80);
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(true);
      // A light theme ignores the opacity and paints solid, so there is
      // nothing showing through and the blur is drawing work nobody can see.
      await user.click(screen.getByRole("radio", { name: /^paper/i }));
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(false);
      expect(root.getAttribute("data-opacity")).toBe("80");
      expect(stored()).toMatchObject({ theme: "paper", opacity: 80 });
      // And back: the opacity was the reader's, and so was the blur.
      await user.click(screen.getByRole("radio", { name: /^midnight/i }));
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(true);
    });

    it("follows a theme changed from outside this pane, as the titlebar's button does", async () => {
      localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ opacity: 80 }));
      applyStoredAppearance();
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(true);
      // `toggleNextDesignTheme` and the OS follower write `data-theme`
      // themselves; neither knows this module exists.
      await act(async () => {
        root.removeAttribute("data-theme");
        await Promise.resolve();
      });
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(false);
    });

    it("asks again when the window refused, instead of believing it is blurred", async () => {
      core.applyWindowBlur.mockResolvedValueOnce(false);
      paint();
      await slideTo(80);
      expect(core.applyWindowBlur).toHaveBeenCalledTimes(1);
      // Nothing about what is wanted has changed — only that the first ask
      // failed. A record that said "blurred" here would never ask again.
      await slideTo(79);
      expect(core.applyWindowBlur).toHaveBeenCalledTimes(2);
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(true);
    });

    it("follows an outside theme change for an opacity first chosen this session", async () => {
      paint();
      await slideTo(80);
      await act(async () => {
        root.setAttribute("data-theme", "contrast");
        await Promise.resolve();
      });
      expect(core.applyWindowBlur).toHaveBeenLastCalledWith(false);
    });

    it("is switched off on a light theme, and says which themes it works on", () => {
      root.setAttribute("data-theme", "paper");
      paint();
      expect(slider().disabled).toBe(true);
      expect(screen.getByTestId("opacity-hint").textContent).toMatch(/dark, midnight or glass/i);
    });

    it("is not offered on the web, which has no window to see through", () => {
      core.isTauri.mockReturnValue(false);
      paint();
      expect(screen.queryByRole("slider", { name: "Window opacity" })).toBeNull();
    });

    it("is not offered off macOS, where the window is not created see-through", () => {
      onPlatform("Win32");
      paint();
      expect(screen.queryByRole("slider", { name: "Window opacity" })).toBeNull();
    });

    describe("what the text is read against", () => {
      // The token contrast suite in ui-kit checks solid colours, and a
      // see-through ground is not one: what the text sits on is the theme's
      // tint laid over whatever is behind the window. White is the worst
      // desktop there is for a dark theme, so that is what is composited here.
      // The blur softens what is behind the window; it does not darken it.
      const block = (theme: string) =>
        TOKENS.match(new RegExp(`\\n\\[data-theme="${theme}"\\] \\{([^}]*)\\}`))![1];
      const token = (theme: string, name: string) =>
        block(theme).match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`))![1];
      /** The colour the see-through page is tinted with, as the stylesheet mixes it. */
      function tint(theme: string): string {
        const rule = [...TOKENS.matchAll(/([^{}]+)\{([^{}]*--ground:\s*color-mix\(in srgb, ([^ ]+) var\(--window-alpha[^{}]*)\}/g)]
          .find(([, selectors]) => selectors.includes(`[data-theme="${theme}"][data-opacity]`));
        const mixed = rule![3];
        return mixed.startsWith("#") ? mixed : token(theme, mixed.match(/var\(--([\w-]+)\)/)![1]);
      }
      const channels = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
      const luminance = (rgb: number[]) => {
        const [r, g, b] = rgb.map((v) => v / 255).map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
        return r * 0.2126 + g * 0.7152 + b * 0.0722;
      };
      /** WCAG contrast of an ink on the theme's tint at `percent`, over a white desktop. */
      function contrastOverWhite(theme: string, ink: string, percent: number): number {
        const alpha = percent / 100;
        const ground = channels(tint(theme)).map((c) => c * alpha + 255 * (1 - alpha));
        const [lo, hi] = [luminance(ground), luminance(channels(token(theme, ink)))].sort((a, b) => a - b);
        return (hi + 0.05) / (lo + 0.05);
      }
      const INKS = ["ink", "ink-soft", "ink-muted", "ink-faint"];
      const SEE_THROUGH = ["dark", "midnight", "glass"];

      it.each(SEE_THROUGH)("keeps %s's body text readable at the floor, over a white desktop", (theme) => {
        expect(contrastOverWhite(theme, "ink", OPACITY.MIN)).toBeGreaterThanOrEqual(4.5);
      });

      it.each(SEE_THROUGH)("keeps every ink on %s readable from the legible mark up", (theme) => {
        for (const ink of INKS) {
          expect(contrastOverWhite(theme, ink, OPACITY.LEGIBLE), ink).toBeGreaterThanOrEqual(4.5);
        }
      });

      it("puts the legible mark where it is needed, not merely somewhere safe", () => {
        // Five points lower and at least one theme's faintest ink fails; that
        // is what makes the warning below worth showing where it shows.
        const lower = OPACITY.LEGIBLE - 5;
        const failing = SEE_THROUGH.filter((theme) =>
          INKS.some((ink) => contrastOverWhite(theme, ink, lower) < 4.5),
        );
        expect(failing.length).toBeGreaterThan(0);
      });

      it("opens Glass at an opacity where all of its own text is readable", () => {
        for (const ink of INKS) {
          expect(contrastOverWhite("glass", ink, GLASS_OPACITY), ink).toBeGreaterThanOrEqual(4.5);
        }
      });

      it("says so when the reader goes below the legible mark, and not above it", async () => {
        paint();
        await slideTo(OPACITY.LEGIBLE);
        expect(screen.queryByTestId("opacity-warning")).toBeNull();
        await slideTo(OPACITY.LEGIBLE - 1);
        expect(screen.getByTestId("opacity-warning").textContent).toMatch(/bright/i);
      });
    });

    describe("the Glass theme", () => {
      beforeEach(() => root.removeAttribute("data-theme"));

      it("turns a solid window see-through, because that is what makes it glass", async () => {
        const { user } = paint();
        await user.click(screen.getByRole("radio", { name: /^glass/i }));
        expect(root.getAttribute("data-theme")).toBe("glass");
        expect(root.getAttribute("data-opacity")).toBe(String(GLASS_OPACITY));
        expect(root.style.getPropertyValue("--window-alpha")).toBe(`${GLASS_OPACITY}%`);
        expect(stored()).toEqual({ theme: "glass", opacity: GLASS_OPACITY });
        expect(core.applyWindowBlur).toHaveBeenLastCalledWith(true);
        // Inside the slider's own range, or the control could not show it.
        expect(GLASS_OPACITY).toBeGreaterThanOrEqual(OPACITY.MIN);
        expect(GLASS_OPACITY).toBeLessThan(OPACITY.MAX);
      });

      it("keeps an opacity the reader already chose", async () => {
        const { user } = paint();
        await user.click(screen.getByRole("radio", { name: /^dark/i }));
        await slideTo(88);
        await user.click(screen.getByRole("radio", { name: /^glass/i }));
        expect(root.getAttribute("data-opacity")).toBe("88");
        expect(stored()).toEqual({ theme: "glass", opacity: 88 });
      });

      it("stays a solid theme where the window cannot be seen through", async () => {
        core.isTauri.mockReturnValue(false);
        const { user } = paint();
        await user.click(screen.getByRole("radio", { name: /^glass/i }));
        expect(root.getAttribute("data-theme")).toBe("glass");
        expect(root.hasAttribute("data-opacity")).toBe(false);
        expect(stored()).toEqual({ theme: "glass" });
        expect(core.applyWindowBlur).not.toHaveBeenCalled();
      });

      it("can be made see-through like the other dark themes", async () => {
        root.setAttribute("data-theme", "glass");
        paint();
        expect(slider().disabled).toBe(false);
      });
    });

    describe("at boot", () => {
      it("puts a stored opacity back on the root and blurs behind it", () => {
        localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ opacity: 90 }));
        applyStoredAppearance();
        expect(root.getAttribute("data-opacity")).toBe("90");
        expect(root.style.getPropertyValue("--window-alpha")).toBe("90%");
        expect(core.applyWindowBlur).toHaveBeenCalledWith(true);
      });

      it("leaves the blur off for a reader who turned it off", () => {
        localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ opacity: 90, blur: false }));
        applyStoredAppearance();
        expect(root.getAttribute("data-opacity")).toBe("90");
        expect(core.applyWindowBlur).not.toHaveBeenCalledWith(true);
      });

      it("does not blur behind a light theme, whatever opacity is stored", () => {
        localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ theme: "paper", opacity: 90 }));
        applyStoredAppearance();
        // The opacity is still the reader's, and waits for a theme that uses it.
        expect(root.getAttribute("data-opacity")).toBe("90");
        expect(core.applyWindowBlur).not.toHaveBeenCalledWith(true);
      });

      describe("after a reload", () => {
        /**
         * A reload starts this module afresh, but the native window keeps the
         * blur the last page put on it: switching designs reloads, and so does
         * a dev build. A fresh module instance is what the next page boots.
         *
         * Store nothing see-through here: that arms the fresh module's root
         * observer, which nothing can disconnect, and it would go on calling
         * the shared mock for the rest of this file.
         */
        async function reloaded() {
          vi.resetModules();
          return await import("../../lib/appearance");
        }

        it("tells a solid window it has no blur, in case the last page left one on it", async () => {
          localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ opacity: 100 }));
          (await reloaded()).applyStoredAppearance();
          expect(root.hasAttribute("data-opacity")).toBe(false);
          expect(core.applyWindowBlur).toHaveBeenCalledTimes(1);
          expect(core.applyWindowBlur).toHaveBeenCalledWith(false);
        });

        it("asks nothing of a window that cannot be seen through", async () => {
          // The web, and every desktop window off macOS, has never had a blur
          // to leave behind — nor a theme pick that should cost a round trip.
          core.isTauri.mockReturnValue(false);
          const appearance = await reloaded();
          appearance.applyStoredAppearance();
          appearance.syncWindowBlur();
          expect(core.applyWindowBlur).not.toHaveBeenCalled();
        });
      });

      it("ignores a stored opacity where the window cannot be seen through", () => {
        localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ opacity: 90 }));
        core.isTauri.mockReturnValue(false);
        applyStoredAppearance();
        onPlatform("Linux x86_64");
        core.isTauri.mockReturnValue(true);
        applyStoredAppearance();
        expect(root.hasAttribute("data-opacity")).toBe(false);
        expect(core.applyWindowBlur).not.toHaveBeenCalled();
      });

      it("ignores an opacity outside the range the slider offers", () => {
        for (const opacity of [12, 140, "90", null]) {
          localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ opacity, blur: "yes" }));
          applyStoredAppearance();
          expect(root.hasAttribute("data-opacity"), String(opacity)).toBe(false);
        }
      });
    });
  });

  describe("the way back to the old design", () => {
    it("offers it", async () => {
      const { user, onSwitchToClassic } = paint();
      await user.click(screen.getByRole("button", { name: /classic/i }));
      expect(onSwitchToClassic).toHaveBeenCalledTimes(1);
    });

    it("labels classic as deprecated, and says it is going away", () => {
      paint();
      expect(screen.getByRole("button", { name: "Switch to Classic (deprecated)" })).toBeDefined();
      expect(
        screen.getByText(/classic design is deprecated and will be removed in a future version/i),
      ).toBeDefined();
    });

    it("calls the new design the default, not work in progress", () => {
      paint();
      expect(screen.getByText(/you are in the new design, the default/i)).toBeDefined();
      expect(screen.queryByText(/in progress/i)).toBeNull();
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
