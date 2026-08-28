// @vitest-environment node
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join, relative, resolve, dirname } from "node:path";

/**
 * Every workspace package's components must be inside Tailwind's scan.
 *
 * Tailwind generates a utility only where it sees the class in a scanned file,
 * and it scans the source tree of the app doing the build. A package under
 * `packages/` is outside that tree, so its own classes produce no rule unless
 * its stylesheet declares `@source`. The kit shipped without one: `h-[5px]`
 * and `text-[0.6875rem]` generated nothing and `text-muted` was dropped, which
 * left the Meter's track with no height and every bar invisible — in a real
 * build only. Nothing in this suite touches CSS, so 1338 unit tests stayed
 * green through it. (#317 review)
 *
 * Checked against the files on disk rather than against the presence of the
 * directive, so a glob that stops matching — a component moved into a new
 * folder under a non-recursive pattern, or a whole new package — fails here
 * instead of in someone's window.
 */
const ROOT = resolve(__dirname, "../../..");

function walk(dir: string, test: (name: string) => boolean): string[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const path = join(dir, e.name);
    if (e.isDirectory()) return walk(path, test);
    return test(e.name) ? [path] : [];
  });
}

/**
 * A `@source` glob as a matcher over absolute paths.
 *
 * `/**\/` has to match no directory as well as several, or `src/Meter.tsx`
 * fails a pattern written to cover the whole of `src`.
 */
function globToRegExp(glob: string): RegExp {
  const literal = (part: string) =>
    part.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^/]*");
  const pattern = glob.split("/**/").map(literal).join("/(?:.*/)?");
  return new RegExp(`^${pattern}$`);
}

type Scan = { include: RegExp[]; exclude: RegExp[] };

function declaredScan(pkg: string): Scan {
  const scan: Scan = { include: [], exclude: [] };
  for (const css of walk(join(pkg, "src"), (n) => n.endsWith(".css"))) {
    const source = readFileSync(css, "utf8");
    for (const m of source.matchAll(/@source\s+(not\s+)?["']([^"']+)["']/g)) {
      const absolute = resolve(dirname(css), m[2]);
      (m[1] ? scan.exclude : scan.include).push(globToRegExp(absolute));
    }
  }
  return scan;
}

describe("tailwind source scanning", () => {
  const packages = readdirSync(join(ROOT, "packages"), { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => join(ROOT, "packages", e.name));

  it("has packages to check", () => {
    expect(packages.length).toBeGreaterThan(0);
  });

  it.each(packages.map((p) => [relative(ROOT, p), p]))(
    "%s declares a scan covering its own components",
    (_name, pkg) => {
      const components = walk(
        join(pkg, "src"),
        (n) => n.endsWith(".tsx") && !n.includes(".test."),
      );
      // A package with no components needs no scan.
      if (components.length === 0) return;

      const { include, exclude } = declaredScan(pkg);
      const uncovered = components.filter(
        (file) =>
          !include.some((re) => re.test(file)) || exclude.some((re) => re.test(file)),
      );
      expect(
        uncovered.map((f) => relative(ROOT, f)),
        `outside Tailwind's scan, so their utilities are never generated`,
      ).toEqual([]);
    },
  );
});
