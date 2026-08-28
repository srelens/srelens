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
