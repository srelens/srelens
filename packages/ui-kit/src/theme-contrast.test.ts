import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(__dirname, "styles/tokens.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const themes = ["light", "paper", "dark", "midnight", "contrast"];
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


it.each(themes)("gives %s form boundaries enough contrast against their surfaces", theme => {
  const values = tokens(theme, "violet");
  expect(values["control-line"]).toMatch(/^#[\da-fA-F]{6}$/);
  for (const ground of grounds) expect(contrast(values["control-line"], values[ground]), ground).toBeGreaterThanOrEqual(3);
});
