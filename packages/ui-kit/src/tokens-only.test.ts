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
