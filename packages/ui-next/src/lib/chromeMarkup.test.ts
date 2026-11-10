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
 *
 * And one way to draw a figure, a live signal and a log line (step 7e): the
 * kit's `MetricTile`, `LiveSignal` and `LogLine`, where apps drew
 * `.dashboard-card`, `.extension-live` and `LogLineRow`.
 *
 * And one way to draw a key and its value (step 7f): the kit's `KV`, where the
 * Inspector, Topology, the Clusters pane and an app's operation screen each drew
 * their own `<dl>`.
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

  it("draws no figure tile, live signal or log line of its own", () => {
    // The band's own layout (`.dashboard-cards`, `.dashboard-cards-band`), the
    // live row beside a picker and the log's level picker stay: they place the
    // kit's parts, they don't draw them. So does `LiveStatus`, which only says
    // which word and tone a live state takes on the kit's signal.
    expect(
      unexpected(
        /\bdashboard-card(?!s(?:-band)?(?![\w-]))|extension-live(?:-dot)?(?![\w-])|\bLogLineRow\b|extension-app-log-line|extension-log-(?:time|level|source|text)(?![\w-])/g,
        {},
      ),
    ).toEqual({});
  });

  it("draws no key/value list of its own", () => {
    // The host confirmation keeps its list: classic's MCP dialog and Assistant
    // conversation render it too, and classic loads no kit stylesheet, so a `KV`
    // there would come out unstyled on a security prompt.
    expect(unexpected(/<d[lt]\b|\bextension-facts\b/g, { "confirm/HostConfirmation.tsx": 5 })).toEqual({});
  });

  /**
   * A card's class in a `className`, written out or in a string inside its
   * expression (`cx(…)`, a template). The expression is read up to its first
   * `}`, so a prop named `card` after it is not the class.
   */
  const CARD_CLASS =
    /className=(?:"[^"]*|\{[^}]*?["'`][^"'`}]*?)(?<![\w-])card(?:-head|-title)?(?![\w-])/g;

  it("draws no card of its own", () => {
    // A card is the kit's `Panel`, title, count and all. Connect wrote the
    // card's classes itself because `Panel` had no place for its count. (#1064)
    // Stylesheets that restyle a kit `Panel` where it sits are not markup.
    expect(unexpected(CARD_CLASS, {}, /\.tsx$/)).toEqual({});
  });

  it("knows a card's class however the className is written", () => {
    const caught = [
      'className="card min-w-0"',
      'className={"card"}',
      "className={`card ${open}`}",
      'className={cx("min-w-0", "card-head")}',
      'className={cx(\n  "card",\n  open,\n)}',
    ];
    const passed = [
      'className="rounded-card"',
      'className={cx("extension-card-scope")}',
      "const card = target.card",
      "<Body className={rows} hideToolbar\n  card={found} />",
    ];
    expect(caught.filter((markup) => !markup.match(CARD_CLASS))).toEqual([]);
    expect(passed.filter((markup) => markup.match(CARD_CLASS))).toEqual([]);
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
