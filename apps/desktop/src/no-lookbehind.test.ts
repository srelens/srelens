// @vitest-environment node
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join, relative, resolve } from "node:path";

/**
 * No shipped file may contain a regex lookbehind, `(?<=` or `(?<!`.
 *
 * Safari before 16.4 (macOS before 13.3) does not support them. A literal
 * is a SyntaxError when the script is parsed, so one lookbehind anywhere in
 * the bundle stops the whole app loading on such a machine; one built with
 * `new RegExp` throws when it runs, which for the Logs screen is while it
 * renders. Named groups, `(?<name>`, are fine and are not matched.
 *
 * `logQuery.test.ts` guards the one module that was bitten. This guards the
 * rest, so the next lookbehind is caught here and not on someone's Mac.
 */
const ROOT = resolve(__dirname, "../../..");

const SHIPPED = ["packages/core/src", "packages/ui-kit/src", "packages/ui-next/src", "apps/desktop/src"];

const LOOKBEHIND = /\(\?<[=!]/;

function walk(dir: string, test: (name: string) => boolean): string[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const path = join(dir, e.name);
    if (e.isDirectory()) return walk(path, test);
    return test(e.name) ? [path] : [];
  });
}

/** Source that ships: `.ts`/`.tsx`, but not a test and not the setup a test runs after. */
const isShipped = (name: string) =>
  /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) && !/^test-setup\.tsx?$/.test(name);

describe("regex lookbehind", () => {
  const files = SHIPPED.flatMap((dir) => walk(join(ROOT, dir), isShipped));

  it("has shipped files to check", () => {
    expect(files).toContain(join(ROOT, "packages/core/src/lib/logTerms.ts"));
    expect(files).not.toContain(join(ROOT, "packages/core/src/lib/logTerms.test.ts"));
  });

  it("is told apart from a named group", () => {
    expect(LOOKBEHIND.test("/(?<=%),/")).toBe(true);
    expect(LOOKBEHIND.test("/(?<!:)\\/\\//")).toBe(true);
    expect(LOOKBEHIND.test("/(?<year>\\d{4})/")).toBe(false);
  });

  it("appears in no file that ships", () => {
    const offenders = files.filter((file) => LOOKBEHIND.test(readFileSync(file, "utf8")));
    expect(
      offenders.map((f) => relative(ROOT, f)),
      "use a one-character prefix group instead, as highlightRanges does in logQuery.ts",
    ).toEqual([]);
  });
});
