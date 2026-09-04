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
 */
const HEX = /#[0-9a-fA-F]{3,8}\b/;

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
    const offenders = files.filter((f) => HEX.test(read(f)));
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

  /**
   * The accent a reader can CHOOSE, not just the one this theme declares.
   *
   * The test above reads `[data-theme="contrast"]`'s own block, and an accent
   * picked in the Appearance pane is not in it — it arrives from a later
   * `[data-accent="…"]` rule of equal specificity, which wins. So High
   * contrast promised AAA and delivered between 5.37:1 and 6.64:1 the moment
   * anyone chose Blue, Teal, Amber or Rose, and nothing here could see it.
   */
  function accentOverride(theme: string, accent: string): Record<string, string> {
    const css = readFileSync(join(__dirname, "styles", "tokens.css"), "utf8");
    // Every rule whose selector list carries this exact theme+accent pair, in
    // file order — the last one wins, as the cascade has it at equal weight.
    const out: Record<string, string> = {};
    for (const [, selectors, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      const matches = selectors
        .split(",")
        .some((s) => s.trim() === `[data-theme="${theme}"][data-accent="${accent}"]`);
      if (!matches) continue;
      for (const [, name, value] of body.matchAll(/(--[a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) out[name] = value;
    }
    return out;
  }

  /** The bare `[data-accent="x"]` set, which every theme inherits by default. */
  function bareAccent(accent: string): Record<string, string> {
    const css = readFileSync(join(__dirname, "styles", "tokens.css"), "utf8");
    const out: Record<string, string> = {};
    for (const [, selectors, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      if (!selectors.split(",").some((s) => s.trim() === `[data-accent="${accent}"]`)) continue;
      for (const [, name, value] of body.matchAll(/(--[a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) out[name] = value;
    }
    return out;
  }

  it("clears AAA for every accent a reader can choose, not only its own", () => {
    const base = tokens();
    const failures: string[] = [];
    for (const accent of ["blue", "teal", "amber", "rose"]) {
      // What the cascade actually resolves to: the theme's block, then the
      // bare accent rule, then any contrast-specific override.
      const t = { ...base, ...bareAccent(accent), ...accentOverride("contrast", accent) };
      const grounds = ["--canvas", "--canvas-deep", "--surface", "--surface-sunk", "--surface-raised"];
      for (const ground of grounds) {
        const ratio = contrast(t["--accent"], t[ground]);
        if (ratio < 7) failures.push(`${accent}: --accent on ${ground} = ${ratio.toFixed(2)}:1`);
      }
      const onWash = contrast(t["--accent"], t["--accent-wash"]);
      if (onWash < 7) failures.push(`${accent}: --accent on --accent-wash = ${onWash.toFixed(2)}:1`);
      const inkOn = contrast(t["--accent-ink"], t["--accent"]);
      if (inkOn < 7) failures.push(`${accent}: --accent-ink on --accent = ${inkOn.toFixed(2)}:1`);
    }
    expect(failures).toEqual([]);
  });

  it("keeps High contrast's ground pure, whatever accent is chosen", () => {
    // Every other theme tints its canvas with a few percent of the accent so
    // choosing one is visible on a screen that is mostly ground. This theme
    // must not: its ratios are all computed against pure white.
    //
    // Read as the RESOLVED value, not as the absence of a selector. The first
    // version of this test asserted no `[data-theme="contrast"][data-accent]`
    // rule existed and passed while the theme was visibly tinted, because the
    // tint arrived from `:root[data-accent]` — an element plus an attribute,
    // which outweighs `[data-theme="contrast"]` alone. Absence of a rule says
    // nothing about what the cascade does.
    const css = readFileSync(join(__dirname, "styles", "tokens.css"), "utf8");
    const own = tokens();
    const start = css.indexOf(':root[data-theme="contrast"][data-accent] {');
    expect(start, "no rule holds High contrast's ground against the tint").toBeGreaterThan(-1);
    const body = css.slice(start, css.indexOf("}", start));
    for (const token of ["--canvas", "--canvas-deep"]) {
      const held = body.match(new RegExp(`${token}:\\s*(#[0-9a-fA-F]{6})\\s*;`))?.[1];
      expect(held?.toLowerCase(), `${token} is not held at this theme's own value`).toBe(
        own[token]?.toLowerCase(),
      );
    }
    // And it must outweigh the root tint, or it is decoration.
    expect(css.indexOf(":root[data-accent] {")).toBeLessThan(start);
  });
});

/**
 * The accent-tinted grounds copy each theme's own `--canvas` as the base of a
 * `color-mix`, because a custom property cannot be defined in terms of itself.
 * A copy is a thing that drifts, so it is checked rather than trusted.
 */
describe("the accent-tinted grounds", () => {
  const css = readFileSync(join(__dirname, "styles", "tokens.css"), "utf8");

  /** A theme's own declaration of one ground token. */
  function declared(selector: string, token: string): string | undefined {
    const start = css.indexOf(`${selector} {`);
    if (start === -1) return undefined;
    const body = css.slice(start, css.indexOf("}", start));
    return body.match(new RegExp(`${token}:\\s*(#[0-9a-fA-F]{6})\\s*;`))?.[1];
  }

  const THEMES = [
    [":root", ":root[data-accent]"],
    ['[data-theme="paper"]', '[data-theme="paper"][data-accent]'],
    ['[data-theme="dark"]', '[data-theme="dark"][data-accent]'],
    ['[data-theme="midnight"]', '[data-theme="midnight"][data-accent]'],
  ] as const;

  it.each(THEMES)("mixes %s's accent tint into that theme's own ground", (plain, tinted) => {
    const start = css.indexOf(`${tinted} {`);
    expect(start, `${tinted} declares no tinted ground`).toBeGreaterThan(-1);
    const body = css.slice(start, css.indexOf("}", start));
    for (const token of ["--canvas", "--canvas-deep"]) {
      // `[^;]` rather than `[^)]`: the mix contains `var(--accent)`, so a
      // class that stops at the first `)` never reaches the base colour.
      const base = body.match(new RegExp(`${token}:\\s*color-mix\\([^;]*?(#[0-9a-fA-F]{6})\\s*\\)`))?.[1];
      expect(base, `${tinted} does not mix into a literal ${token}`).toBeDefined();
      expect(base?.toLowerCase(), `${tinted}'s ${token} base has drifted from ${plain}'s`).toBe(
        declared(plain, token)?.toLowerCase(),
      );
    }
  });

  it("leaves the surfaces content sits on untinted", () => {
    // The ground moves; the paper does not. Text keeps the contrast its theme
    // was drawn for.
    for (const [, tinted] of THEMES) {
      const start = css.indexOf(`${tinted} {`);
      const body = css.slice(start, css.indexOf("}", start));
      expect(body).not.toMatch(/--surface/);
    }
  });
});
