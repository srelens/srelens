import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * One way to draw a failure (#1001, step 7a). A failure reaches the reader
 * through `FailureState`, `FailureAlert` or the kit's `Alert`, which classify
 * it, fold the original away and offer Retry the same way everywhere. Every
 * hand-built one said the same thing differently: a red paragraph here, a
 * bordered box there, the raw struct somewhere else.
 */
describe("one way to draw a failure", () => {
  const src = join(dirname(fileURLToPath(import.meta.url)), "..");
  const sources = (readdirSync(src, { recursive: true }) as string[])
    .map((file) => file.replaceAll("\\", "/"))
    .filter((file) => /\.(tsx?|css)$/.test(file) && !/\.test\.tsx?$/.test(file))
    .map((file) => ({ file, text: readFileSync(join(src, file), "utf8") }));

  /** Hand-written alerts kept on purpose, by file and count, and why. */
  const KEPT: Record<string, number> = {
    // The passphrase error: its own layout and shake, which LockGate's tests pin.
    "shell/LockGate.tsx": 1,
    // Read to a screen reader only: there is nothing to draw.
    "shell/StarButton.tsx": 1,
    // A sentence beside the control it reports on, inside a row of them.
    "native-components/NativeMarkdown.tsx": 1,
    "screens/settings/McpClientSetup.tsx": 1,
    // A field's own message, under the field: the inputs step (7c) gives fields one.
    "screens/Logs.tsx": 3,
    "extensions/ExtensionSettingsForm.tsx": 1,
    // A conflict to decide, with Force apply in it: the panels step (7d).
    "screens/Edit.tsx": 1,
  };

  it("draws alerts through the kit, but for the ones kept on purpose", () => {
    const drawn = Object.fromEntries(
      sources
        // Not one quoted in a comment, which says `role="alert"` in backticks.
        .map(({ file, text }) => [file, (text.match(/(^|[^`])role="alert"/g) ?? []).length] as const)
        .filter(([file, count]) => count !== (KEPT[file] ?? 0)),
    );
    expect(drawn).toEqual({});
  });

  it("has no ErrorNotice left", () => {
    expect(sources.filter(({ text }) => /\bErrorNotice\b/.test(text)).map(({ file }) => file)).toEqual([]);
  });

  it("has no hand-built error or warning box left", () => {
    const boxes = /\b(extension-error|extension-warning|app-operation-error|extension-process-alert)\b/;
    expect(sources.filter(({ text }) => boxes.test(text)).map(({ file }) => file)).toEqual([]);
  });
});
