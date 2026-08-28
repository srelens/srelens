// @vitest-environment node
// (reads files from disk; jsdom buys nothing here)
import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * `components.json` tells the shadcn CLI where to write generated components
 * and what to import `cn` from. Nothing type-checks it, and nothing imports it,
 * so a stale alias is invisible until someone runs `shadcn add` and gets a
 * component with an unresolvable import.
 *
 * That is exactly what happened when `cn` moved out of `lib/` with the service
 * layer (#311): the alias still pointed at `@/lib/utils`, which no longer
 * existed. Asserted here so the next move of a UI helper fails a test rather
 * than a colleague's next `shadcn add`.
 */
const root = join(__dirname, "..");
const config = JSON.parse(readFileSync(join(root, "components.json"), "utf8")) as {
  aliases: Record<string, string>;
  tailwind: { css: string };
};

/** Resolve an `@/x` alias to a path on disk; `@` is `src`. */
function resolve(alias: string): string {
  return join(root, alias.replace(/^@\//, "src/"));
}

describe("components.json", () => {
  it("points `utils` at a module that exists", () => {
    // Every generated component imports `cn` from this alias.
    const utils = resolve(config.aliases.utils);
    expect(
      existsSync(`${utils}.ts`) || existsSync(`${utils}.tsx`),
      `aliases.utils "${config.aliases.utils}" does not resolve to a file`,
    ).toBe(true);
  });

  it("points `components` and `ui` at directories that exist", () => {
    for (const key of ["components", "ui"] as const) {
      expect(
        existsSync(resolve(config.aliases[key])),
        `aliases.${key} "${config.aliases[key]}" does not resolve to a directory`,
      ).toBe(true);
    }
    // `hooks` is deliberately not checked: the CLI creates that directory on
    // demand, and it has never existed in this repo.
  });

  it("points the tailwind entry at the stylesheet the app actually loads", () => {
    expect(existsSync(join(root, config.tailwind.css))).toBe(true);
  });
});
