import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * One way to draw a strip of controls and a pane's opening band (#1001, step
 * 7d). A strip is the kit's `Toolbar`; the band that names a settings pane is
 * `PaneIntro`. Before, apps drew their own `.extension-toolbar` 13 times while
 * the kit's Toolbar had no direct use, and six settings panes each drew their
 * opening heading with their own sizes and insets.
 */
describe("one way to draw a toolbar and a pane's opening", () => {
  const src = join(dirname(fileURLToPath(import.meta.url)), "..");
  const sources = (readdirSync(src, { recursive: true }) as string[])
    .map((file) => file.replaceAll("\\", "/"))
    .filter((file) => /\.(tsx|css)$/.test(file) && !/\.test\.tsx$/.test(file))
    .map((file) => ({ file, text: readFileSync(join(src, file), "utf8") }));

  /** Files whose matches differ from what is kept on purpose, with the matches. */
  const unexpected = (pattern: RegExp, kept: Record<string, number>, only: RegExp = /./) =>
    Object.fromEntries(
      sources
        .filter(({ file }) => only.test(file))
        .map(({ file, text }) => [file, text.match(pattern) ?? []] as const)
        .filter(([file, found]) => found.length !== (kept[file] ?? 0)),
    );

  it("draws no app toolbar of its own", () => {
    expect(unexpected(/extension-toolbar/g, {})).toEqual({});
  });

  it("wraps a toolbar that opens with a title, so the title cannot push its actions off the pane", () => {
    // A title can't shrink below its longest word, and an app's card title is
    // the app's own text. The app toolbars these replace all wrapped.
    expect(unexpected(/<Toolbar>\s*<\w+ className="toolbar-title/g, {})).toEqual({});
  });

  it("opens no settings pane with a heading of its own", () => {
    expect(
      unexpected(
        /<h2\b/g,
        {
          // A release's version, heading the notes under it — not the pane's name.
          "screens/settings/UpdatesPane.tsx": 1,
        },
        /^screens\/settings\/.*\.tsx$/,
      ),
    ).toEqual({});
  });

  it("insets a settings pane's bands as its PaneIntro is inset", () => {
    // 12px, a Panel head's inset. Three panes drew their bands at 16px, so
    // their text sat 4px off the intro's above it.
    expect(unexpected(/(?<![\w-])px-4(?![\w-])/g, {}, /^screens\/settings\/.*\.tsx$/)).toEqual({});
  });
});
