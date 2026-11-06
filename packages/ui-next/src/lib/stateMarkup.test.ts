import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * One way to say "still loading" and one way to say "nothing here" (#1001,
 * step 7a). A region still loading draws `LoadingState`, whose spinner is a
 * status named by its label; a region that loaded empty draws `EmptyState`.
 * The hand-written lines said the same things as plain paragraphs, some
 * announced and some not, at a padding of their own.
 */
describe("one way to draw loading and empty", () => {
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

  it("draws no loading line by hand, but for the ones kept on purpose", () => {
    // "Loading…", "Checking the manifest…", "Comparing with the cluster…": a word in -ing, an ellipsis.
    const line = />\s*[A-Z][a-z]+ing\b[^<]*…\s*<\//g;
    expect(
      unexpected(line, {
        // A cell still resolving, inside a table row.
        "extensions/useResolvedColumns.tsx": 2,
        // Beside the bulk-action buttons in a toolbar: the toolbars step (7d).
        "extensions/ExtensionBulkActions.tsx": 1,
        // A card's own figure: the tiles step (7e) moves cards onto the kit.
        "extensions/DashboardCards.tsx": 1,
        // Chips in the prompt's status row, beside the ones that say other things.
        "shell/Console.tsx": 2,
        // A disabled picker standing in for the one still loading, with the kit's Spinner in it.
        "screens/resourceShell.tsx": 1,
        "screens/collections/NamespaceScope.tsx": 1,
        // A word in a row of cluster chips, where the picker it stands in for goes.
        "screens/settings/WorkspacesPane.tsx": 1,
      }),
    ).toEqual({});
  });

  it("names a load without a trailing ellipsis, as the spinner already says it is under way", () => {
    expect(unexpected(/<LoadingState\b[^>]*label=[^>]*…/g, {})).toEqual({});
  });

  it("draws no empty paragraph by hand, but for the ones kept on purpose", () => {
    expect(
      unexpected(/<p\b[^>]*>\s*(?:No|Nothing)\b[^<]*<\/p>/g, {
        // A card's figure: the tiles step (7e) moves cards onto the kit.
        "extensions/DashboardCards.tsx": 1,
        // A fact about one row of the permission review, beside the other rows' details.
        "extensions/ExtensionBindings.tsx": 1,
        // A note inside the file row it describes.
        "screens/connections/SourcesRail.tsx": 1,
        // A note beside the token controls it explains.
        "screens/settings/McpServer.tsx": 1,
        // Prose that starts with "Nothing": what the diff cannot say, and one of the
        // schema panel's four sentences on why it lists no keys.
        "screens/Edit.tsx": 1,
        "screens/EditAnalysis.tsx": 1,
      }),
    ).toEqual({});
  });
});
