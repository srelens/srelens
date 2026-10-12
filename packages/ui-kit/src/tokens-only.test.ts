import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join, relative, sep } from "node:path";

/**
 * No component may name a colour of its own.
 *
 * Every value comes from a token, or the component stops following the theme
 * the moment someone switches to dark — and that failure is invisible in a
 * gallery viewed in one mode. The mock carried thirteen raw hex values for
 * exactly this reason, and they do not come across.
 *
 * Kit-wide rather than per-component: the rule is about the design system, and
 * a rule asserted in one file is a rule the next file forgets.
 *
 * Hex was the only form checked, so two of the three ways to name a colour
 * passed (#362). All three now:
 *
 * - hex;
 * - a CSS colour function. `color-mix()` over `var()`s is how the kit derives a
 *   shade from a token and is not one: its `color` is followed by `-`, not `(`.
 *   An arbitrary value — `bg-[#fff]`, `text-[rgb(0_0_0)]` — is one of these two
 *   in brackets, so it is caught as well;
 * - a Tailwind palette class: a colour utility, after any variant, then a hue
 *   at a shade (`hover:border-sky-300/50`) or black/white. The hues are Tailwind
 *   4's own list. The kit's theme colours — `text-muted`, `bg-sev-wash`,
 *   `accent-ink` — never pair a hue with a shade, so none of them matches.
 *
 * Not caught: CSS named colours (`color: "tomato"`, `bg-[white]`). That needs
 * the 148-name list, and the kit has never written one.
 */
const HUE =
  "red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|" +
  "slate|gray|zinc|neutral|stone|mauve|olive|mist|taupe";
const UTILITY =
  "bg|text|decoration|border(?:-[xysetrbl])?|outline|divide|(?:inset-)?ring(?:-offset)?|" +
  "(?:inset-|drop-|text-)?shadow|accent|caret|fill|stroke|placeholder|from|via|to";
const RAW_COLOUR = new RegExp(
  [
    /#[0-9a-fA-F]{3,8}\b/.source,
    /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/.source,
    `(?<![\\w-])(?:${UTILITY})-(?:(?:${HUE})-(?:50|[1-9]00|950)|black|white)(?![\\w-])`,
  ].join("|"),
  "i",
);

/**
 * Every source under `src`, at any depth, as a path relative to it.
 *
 * Recursive because a flat listing made "kit-wide" untrue the moment a
 * component moved into a folder: `gallery/Gallery.tsx` already existed and was
 * already exempt, and any future grouping would have opted itself out of both
 * rules while this suite stayed green. (#317 review)
 */
function sources(dir: string = __dirname): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sources(path);
    if (!/\.tsx?$/.test(entry.name) || entry.name.includes(".test.")) return [];
    return [relative(__dirname, path)];
  });
}

function read(file: string): string {
  // Strip comments: both rules are discussed in prose in several places.
  return readFileSync(join(__dirname, file), "utf8").replace(/\/\*[\s\S]*?\*\/|\/\/.*/g, "");
}

describe("the design system", () => {
  const files = sources();

  it("has components to check", () => {
    expect(files.length).toBeGreaterThan(0);
  });

  it("looks inside subdirectories", () => {
    // Guards the guard: with a flat listing this suite passes while silently
    // exempting whole folders. Conditional so that flattening the kit later is
    // not a spurious failure — it only demands recursion where nesting exists.
    const nested = readdirSync(__dirname, { withFileTypes: true }).some((e) => e.isDirectory());
    if (!nested) return;
    expect(files.some((f) => f.includes(sep))).toBe(true);
  });

  it("names no colour outside the tokens", () => {
    const offenders = files.flatMap((f) => {
      const hit = read(f).match(RAW_COLOUR);
      return hit ? [`${f}: ${hit[0]}`] : [];
    });
    expect(offenders, `raw colour values in: ${offenders.join(", ")}`).toEqual([]);
  });

  it("does not depend on the service layer", () => {
    // A design system that knows about capabilities is not reusable, and the
    // boundary is far easier to keep than to recover.
    const offenders = files.filter((f) => /@srelens\/core/.test(read(f)));
    expect(offenders, `service-layer imports in: ${offenders.join(", ")}`).toEqual([]);
  });
});

/**
 * The matcher on its own. The scan above runs over a kit that is already
 * clean, so it would stay green with any branch of `RAW_COLOUR` deleted; these
 * cases are what keep each form caught, and each near-miss allowed.
 */
describe("the raw-colour matcher", () => {
  it.each([
    "#fff",
    "#1a2b3c80",
    "rgb(0 0 0)",
    "rgba(20, 18, 25, 0.5)",
    "hsl(250 60% 50%)",
    "hsla(0, 0%, 0%, .2)",
    "oklch(0.6 0.2 250)",
    "color(display-p3 1 0 0)",
    "text-red-500",
    "bg-emerald-100",
    "hover:border-sky-300/50",
    "!ring-zinc-900",
    "dark:text-white",
    "bg-black",
    "bg-[#fff]",
    "text-[rgb(0_0_0)]",
  ])("catches %s", (raw) => {
    expect(raw).toMatch(RAW_COLOUR);
  });

  it.each([
    "color-mix(in srgb, var(--accent) 20%, transparent)",
    "text-muted",
    "bg-sev-wash",
    "accent-ink",
    "hover:bg-accent/40",
    "bg-mark-red",
    "toneColor(x)",
    "label(x)",
    "bg-[var(--field)]",
    "text-[0.75rem]",
    "bg-transparent",
    "text-current",
    "theme-blue-500",
    "subtext-red-500",
    "nav-bg-red-500",
    "bg-red",
    "red-500",
    "bg-red-5000",
  ])("lets %s through", (allowed) => {
    expect(allowed).not.toMatch(RAW_COLOUR);
  });
});

/**
 * The toolbar grows rather than clipping.
 *
 * `.toolbar` is worn by both `Screen`'s title bar and a standalone `Toolbar`.
 * A fixed height suits the first and breaks the second: the call sites this
 * replaces — `ResourceBrowser` and `HelmReleasesView` — pass `flex-wrap` and
 * hold a namespace picker, a search box and actions, which wrap at narrow
 * widths. A second row inside a 34px box overflows it and lands on the content
 * below, where the classic toolbar simply got taller. A minimum leaves a
 * one-line toolbar looking exactly as it did. (#325 review)
 */
describe("the toolbar's height", () => {
  it("is a minimum, not a fixed size", () => {
    const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
    const rule = css.slice(css.indexOf("  .toolbar {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("min-height");
    expect(body).not.toMatch(/[^-]height:/);
  });
});

/**
 * A live number must not move sideways when it changes.
 *
 * MEASURED in Chrome against `--font-sans` (system-ui → SF Pro Text on macOS) at
 * the table's own 13px: the digit advances run from 5.954px for "1" to 8.290px
 * for "4" — 2.3px of spread per digit. Every resource list refreshes CPU,
 * MEMORY, RESTARTS and AGE on a poll while the reader is looking at it, and
 * those columns are `text-align: end`, so a changed digit drags every digit
 * before it along with it. Sampling one MEMORY cell across polls put its text's
 * left edge at 1194.28, 1195.75, 1197.13 — 2.85px of wobble, on twenty-five rows
 * at once. Proportional figures are the whole of it: nothing about the column
 * widths moved.
 *
 * On `.tbl td` rather than only on the end-aligned ones: a pod name carries
 * digits too, and one rule cannot drift from the other.
 */
describe("a table cell's figures", () => {
  it("are tabular, so live values do not shift as they change", () => {
    const css = readFileSync(join(__dirname, "styles", "kit.css"), "utf8");
    const rule = css.slice(css.indexOf("  .tbl td {"));
    const body = rule.slice(0, rule.indexOf("}"));
    expect(body).toContain("font-variant-numeric: tabular-nums");
  });
});

/**
 * The `contrast` theme's own promise, computed rather than asserted in prose.
 *
 * ui-next's Accessibility pane tells its reader that High contrast "raises
 * every text pair above 7:1", and nothing anywhere computed a ratio — the pane
 * stated a numeric property of this stylesheet, and a test pinned the number
 * without ever checking it. One pair did not hold: `--ink-faint` on
 * `--surface-sunk` came to 6.56:1.
 *
 * So the ratios are computed here, from the block itself, over every
 * ink-on-ground and tone-on-wash pair the theme defines. WCAG's own formula
 * (relative luminance, sRGB linearisation), and 7:1 is its AAA threshold for
 * body text. A token edited into a prettier grey fails this rather than quietly
 * making a sentence in another package false.
 */
describe("the high-contrast theme", () => {
  /** WCAG 2.x relative luminance of a `#rrggbb` value. */
  function luminance(hex: string): number {
    const channel = (pair: string) => {
      const value = Number.parseInt(pair, 16) / 255;
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    };
    const bare = hex.replace("#", "");
    const [r, g, b] = [bare.slice(0, 2), bare.slice(2, 4), bare.slice(4, 6)].map(channel);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }

  function contrast(a: string, b: string): number {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  }

  /** The theme's own declarations, read off the stylesheet. */
  function tokens(): Record<string, string> {
    const css = readFileSync(join(__dirname, "styles", "tokens.css"), "utf8");
    const start = css.indexOf('[data-theme="contrast"] {');
    expect(start).toBeGreaterThan(-1);
    const body = css.slice(start, css.indexOf("}", start));
    const out: Record<string, string> = {};
    for (const [, name, value] of body.matchAll(/(--[a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) {
      out[name] = value;
    }
    return out;
  }

  it("clears AAA on every text pair it defines", () => {
    const t = tokens();
    const inks = ["--ink", "--ink-soft", "--ink-muted", "--ink-faint"];
    const grounds = ["--canvas", "--canvas-deep", "--surface", "--surface-sunk", "--surface-raised"];
    const tones = ["--accent", "--sev", "--warn", "--ok", "--info"];
    const pairs: Array<[string, string]> = [];
    for (const ink of [...inks, ...tones]) for (const ground of grounds) pairs.push([ink, ground]);
    // A tone on its own wash, which is where a badge's label sits.
    for (const tone of tones) pairs.push([tone, `${tone}-wash`]);
    pairs.push(["--accent-ink", "--accent"]);

    const failures = pairs
      .filter(([a, b]) => t[a] !== undefined && t[b] !== undefined)
      .map(([a, b]) => [a, b, contrast(t[a], t[b])] as const)
      .filter(([, , ratio]) => ratio < 7)
      .map(([a, b, ratio]) => `${a} on ${b} = ${ratio.toFixed(2)}:1`);
    expect(failures).toEqual([]);
    // Guards the guard: a typo in a token name would silently compare nothing.
    expect(pairs.length).toBeGreaterThan(40);
  });
});
