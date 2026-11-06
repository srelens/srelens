import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * One way to draw each kind of button (#1001, step 7b). A box is the kit's
 * `Button`, a glyph is `IconButton`, an action inside a sentence or a row is
 * `Button variant="text"`, and a word that goes somewhere is `Button
 * variant="link"`. Before, screens put the kit's classes on a bare `<button>`
 * and patched them (`btn !py-0`), and drew a link five ways: dotted, dotted
 * and muted, solid and muted, and accent text on a ghost button.
 */
describe("one way to draw a button", () => {
  const src = join(dirname(fileURLToPath(import.meta.url)), "..");
  const sources = (readdirSync(src, { recursive: true }) as string[])
    .map((file) => file.replaceAll("\\", "/"))
    .filter((file) => /\.tsx$/.test(file) && !/\.test\.tsx$/.test(file))
    .map((file) => ({ file, text: readFileSync(join(src, file), "utf8") }));

  /** Files whose matches differ from what is kept on purpose, with the matches. */
  const unexpected = (pattern: RegExp, kept: Record<string, number>) =>
    Object.fromEntries(
      sources
        .map(({ file, text }) => [file, text.match(pattern) ?? []] as const)
        .filter(([file, found]) => found.length !== (kept[file] ?? 0)),
    );

  it("puts no kit button class on an element by hand, but for the one kept on purpose", () => {
    expect(
      unexpected(/className=\{?["'`][^"'`]*(?<![\w-])(?:btn|text-btn|icon-btn)(?![\w-])[^"'`]*["'`]/g, {
        // A Popover's trigger is the content of the Popover's own button, so
        // the look goes on a span inside it: a `Button` there would be a
        // button inside a button.
        "screens/Logs.tsx": 1,
      }),
    ).toEqual({});
  });

  it("underlines no button by hand, but for the ones that are not buttons", () => {
    expect(
      unexpected(/className=\{?["'`][^"'`]*(?<![\w-])underline(?![\w-])[^"'`]*["'`]/g, {
        // A count's stale mark, with when it was counted in its title: not a control.
        "lib/sidebarCounts.tsx": 1,
        // An `<a href>` to the release page: a real link, not a button drawn as one.
        "screens/settings/UpdatesPane.tsx": 1,
      }),
    ).toEqual({});
  });

  it("tints no button with the accent to make it a link", () => {
    expect(unexpected(/<Button\b[^>]*className=["'][^"']*(?<![\w-])text-accent(?![\w-])/g, {})).toEqual({});
  });
});
