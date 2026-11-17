import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * One way to draw each kind of input (#1001, step 7c). Text is the kit's
 * `TextInput` or `TextArea`, a yes/no is `Checkbox`, one of a few listed out is
 * `Radio`, and one of a few side by side is `Tabs variant="segmented"
 * mode="radio"`. Before, app settings and the app manager drew bare controls,
 * four screens styled their own text area, and a choice of one was drawn as
 * pressed ghost buttons, primary and secondary buttons, and radios hidden in
 * `.seg-btn` labels.
 */
describe("one way to draw an input", () => {
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

  it("draws no bare input, text area or select, but for the ones that draw nothing of their own", () => {
    expect(
      unexpected(/<(?:input|textarea|select)\s/g, {
        // A file picker, hidden behind the Button that opens it.
        "extensions/Extensions.tsx": 1,
        "screens/settings/SkillsPane.tsx": 1,
        // A picker whose whole card is the control, with the radio hidden in
        // it for the keyboard and the checked state (theme, accent), and the
        // opacity slider: the kit has no slider and the browser's own works.
        "screens/settings/AppearancePane.tsx": 3,
        // The rail's width sliders.
        "screens/settings/PreferencePanes.tsx": 2,
        // A workspace's colour swatches, each a radio drawn as the colour.
        "screens/settings/WorkspacesPane.tsx": 1,
        // The default agent, picked by the card that describes it.
        "screens/settings/AgentPane.tsx": 1,
      }),
    ).toEqual({});
  });

  it("draws no segmented control by hand", () => {
    expect(unexpected(/className=\{?["'`][^"'`]*(?<![\w-])(?:seg|seg-btn)(?![\w-])/g, {})).toEqual({});
  });

  it("draws no choice of one as a row of pressed buttons", () => {
    expect(unexpected(/aria-pressed=\{\s*\w+\s*===\s*\w+\s*\}/g, {})).toEqual({});
  });
});
