import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(__dirname, "styles/tokens.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const themes = ["light", "paper", "dark", "midnight", "glass", "contrast"];
const accents = ["violet", "blue", "teal", "amber", "rose"];
const grounds = ["canvas", "canvas-deep", "surface", "surface-sunk", "surface-raised"];

// Resolve the actual theme + accent cascade, including the default :root.
// Ignore @theme aliases and density rules: these pairs are solid colour tokens.
function tokens(theme: string, accent: string) {
  const values: Record<string, string> = {};
  for (const [, selectors, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const matches = selectors.split(",").some(raw => {
      const selector = raw.split(";").at(-1)!.trim();
      if (selector === ":root") return true;
      if (!/^(?:\[data-(?:theme|accent)="[a-z]+"\])+$/.test(selector)) return false;
      return [...selector.matchAll(/data-(theme|accent)="([a-z]+)"/g)]
        .every(([, axis, value]) => value === (axis === "theme" ? theme : accent));
    });
    if (matches) for (const [, name, value] of body.matchAll(/--([\w-]+):\s*(#[\da-fA-F]{6})\s*;/g)) values[name] = value;
  }
  return values;
}
function luminance(hex: string) {
  const rgb = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4);
  return rgb[0] * .2126 + rgb[1] * .7152 + rgb[2] * .0722;
}
function contrast(a: string, b: string) {
  const [lo, hi] = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (hi + .05) / (lo + .05);
}

describe.each(themes)("%s theme contrast", theme => {
  it.each(accents)("keeps text readable with the %s accent", accent => {
    const values = tokens(theme, accent);
    const floor = theme === "contrast" ? 7 : 4.5;
    const inks = ["ink", "ink-soft", "ink-muted", "ink-faint"];
    const tones = ["accent", "sev", "warn", "ok", "info"];
    const pairs = [...inks, ...tones].flatMap(ink =>
      [...new Set([...grounds, "accent-wash", ...(tones.includes(ink) ? [`${ink}-wash`] : [])])].map(ground => [ink, ground]));
    pairs.push(["accent-ink", "accent"]);
    expect(pairs.length).toBe(59);
    const failures: string[] = [];
    for (const [ink, ground] of pairs) {
      expect(values[ink], ink).toMatch(/^#[\da-fA-F]{6}$/);
      expect(values[ground], ground).toMatch(/^#[\da-fA-F]{6}$/);
      const ratio = contrast(values[ink], values[ground]);
      if (ratio < floor) failures.push(`${ink} on ${ground}: ${ratio.toFixed(2)}:1 < ${floor}:1`);
    }
    expect(failures).toEqual([]);
  });
});

it("preserves the primary button's paired text colour on hover", () => {
  const kit = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  const hover = kit.match(/\.btn-accent:hover\s*\{([^}]+)\}/)![1];
  expect(hover).toContain("color: var(--accent-ink)");
  expect(hover).not.toContain("brightness");
  expect(hover).toContain("box-shadow: inset 0 0 0 1px var(--accent-ink)");
});

it("uses the legible metadata scale for small operational text", () => {
  const kit = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  for (const selector of [".tab-sub", ".status-seg", ".tbl thead th", ".tbl-group th", ".eyebrow"]) {
    const start = kit.indexOf(`${selector} {`);
    const body = kit.slice(start, kit.indexOf("}", start));
    expect(body, selector).toContain("font-size: var(--text-meta)");
  }
});


it.each(themes)("keeps the %s GitHub pill legible, and visible on the bar it sits in", theme => {
  const values = tokens(theme, "violet");
  const floor = theme === "contrast" ? 7 : 4.5;
  for (const surface of ["gh-surface", "gh-surface-hover"]) {
    // The words and the mark, and the star beside them.
    expect(contrast(values["gh-ink"], values[surface]), `ink on ${surface}`).toBeGreaterThanOrEqual(floor);
    expect(contrast(values["gh-star"], values[surface]), `star on ${surface}`).toBeGreaterThanOrEqual(floor);
  }
  // The pill's edge against whatever it is drawn on: a dark pill on a dark
  // title bar is found by its outline, as any control is.
  for (const ground of grounds) expect(contrast(values["gh-line"], values[ground]), ground).toBeGreaterThanOrEqual(3);
});

// #1001: the active row's wash was ~1.1:1 on the sidebar, the same as hover.
// Its text is ink rather than accent: no wash at 1.3:1 can also carry accent
// text at 4.5:1 for paper's amber, or at 7:1 for the contrast theme's blue.
it.each(themes)("gives the %s active row a fill distinct from the ground", theme => {
  const floor = theme === "contrast" ? 7 : 4.5;
  for (const accent of accents) {
    const values = tokens(theme, accent);
    const wash = values["accent-wash-strong"];
    expect(wash, accent).toMatch(/^#[\da-fA-F]{6}$/);
    expect(contrast(wash, values["surface-sunk"]), `${accent} wash on surface-sunk`).toBeGreaterThanOrEqual(1.3);
    for (const ink of ["ink", "ink-soft"]) expect(contrast(values[ink], wash), `${accent} ${ink}`).toBeGreaterThanOrEqual(floor);
    // The bar and the icon: graphics, so 3:1.
    expect(contrast(values.accent, wash), `${accent} bar`).toBeGreaterThanOrEqual(3);
  }
});

it("gives tree rows and Settings' nav one active style", () => {
  const kit = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  const rule = [...kit.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(([, selectors]) =>
    selectors.includes('.tree-row[data-active="true"]') && selectors.includes('.nav-item[data-active="true"]'));
  expect(rule, "one rule for both").toBeDefined();
  // The third, WorkspaceTree's ink-and-bar `.tree-cluster`, went with it.
  expect(kit).not.toContain(".tree-cluster");
  for (const decl of ["background: var(--accent-wash-strong)", "color: var(--ink)", "font-weight: 600", "box-shadow: inset 3px 0 0 var(--accent)"]) {
    expect(rule![2]).toContain(decl);
  }
});

// A paused or unreachable cluster's mark is dimmed; it must still be seen.
it.each(themes)("keeps an unavailable %s rail mark at 3:1 against the rail", theme => {
  const kit = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  const opacity = Number(kit.match(/\[data-unavailable="true"\]\s*\{\s*opacity:\s*([\d.]+)/)?.[1]);
  expect(opacity).toBeGreaterThan(0);
  const rgb = (hex: string) => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
  const over = (top: string, ground: string) =>
    "#" + rgb(top).map((v, i) => Math.round(v * opacity + rgb(ground)[i] * (1 - opacity)).toString(16).padStart(2, "0")).join("");
  for (const accent of accents) {
    const values = tokens(theme, accent);
    const rail = values["canvas-deep"];
    const marks = [...Object.keys(values).filter(name => name.startsWith("mark-")), "accent"];
    expect(marks.length).toBe(12);
    for (const mark of marks) expect(contrast(over(values[mark], rail), rail), `${accent} ${mark}`).toBeGreaterThanOrEqual(3);
  }
});

it.each(themes)("gives %s form boundaries enough contrast against their surfaces", theme => {
  const values = tokens(theme, "violet");
  expect(values["control-line"]).toMatch(/^#[\da-fA-F]{6}$/);
  for (const ground of grounds) expect(contrast(values["control-line"], values[ground]), ground).toBeGreaterThanOrEqual(3);
});

// #1001: the right rail's edge and the rules between the blocks in it, and in
// the quick-details peek, were --rule: about 1.2:1, a line you had to look for.
// They take the left side panel's band rule (#1006), which must stay a line
// you can see on the grounds it divides.
it("rules off the right rail and the blocks in it in --rule-strong", () => {
  const kit = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  const body = (selector: string) => {
    const start = kit.indexOf(`${selector} {`);
    expect(start, selector).toBeGreaterThan(-1);
    return kit.slice(start, kit.indexOf("}", start));
  };
  expect(body(".side-rail")).toContain("border-left: 1px solid var(--rule-strong)");
  // The peek's own edge: a right-docked pane is ruled off by its grip.
  expect(body('.resize-handle[data-edge="left"]')).toContain("border-left: 1px solid var(--rule-strong)");
  expect(body(".section + .section")).toContain("border-top: 1px solid var(--rule-strong)");
  for (const theme of themes) {
    for (const accent of accents) {
      const values = tokens(theme, accent);
      for (const ground of ["surface", "surface-sunk"]) {
        expect(contrast(values["rule-strong"], values[ground]), `${theme} ${accent} on ${ground}`).toBeGreaterThanOrEqual(1.3);
      }
    }
  }
});

// #1001: the small-caps heads (a pane's, the right rail's, a banded list's)
// were 10px in --ink-faint, the faintest text on screen naming what sits under
// it. --ink-soft on every ground is pinned for every theme and accent by the
// suite at the top of this file; the band draws on --surface-sunk.
it("draws small-caps heads at the metadata size in --ink-soft", () => {
  const kit = readFileSync(join(__dirname, "styles/kit.css"), "utf8");
  for (const selector of [".pane-head", ".subhead-caps"]) {
    const start = kit.indexOf(`${selector} {`);
    const body = kit.slice(start, kit.indexOf("}", start));
    expect(body, selector).toContain("font-size: var(--text-meta)");
    expect(body, selector).toContain("color: var(--ink-soft)");
  }
});
