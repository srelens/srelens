import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

/**
 * No pattern in the service layer may put `\s` next to a `.` capture.
 *
 * They overlap on every space, so the engine has more than one way to split a
 * run of them, and a line the pattern ultimately rejects is re-tried at each
 * split. That is the shape behind all nine `js/polynomial-redos` findings
 * (#43-#51), one of which took 52 seconds on a 4KB input.
 *
 * Asserted on the source because the fix in #313 missed two of them: the
 * patterns in `assistantMarkdown.ts` are named constants and were tightened,
 * while the two in `releaseNotes.ts` are written inline and were not. Both
 * modules parse markdown-ish lines, so the pair drifted apart silently and
 * only a re-scan caught it. A grep is a poor substitute for the analyser, but
 * it runs on every commit rather than on GitHub's schedule.
 *
 * `[^\S\n]` is the intended replacement for `\s` and `[^\n]` for `.`, which is
 * what the patterns meant in the first place.
 */
const AMBIGUOUS = /\\s[+*][^/\n]*\(?\.\*/;

function sources(dir: string): string[] {
  return readdirSync(dir).filter((f) => f.endsWith(".ts") && !f.endsWith(".test.ts"));
}

describe("regex shapes in the service layer", () => {
  const dir = __dirname;

  it("has sources to check", () => {
    // Guards the guard: an empty listing would pass vacuously.
    expect(sources(dir).length).toBeGreaterThan(0);
  });

  it("never places \\s beside a dot-star capture", () => {
    const offenders: string[] = [];
    for (const file of sources(dir)) {
      const lines = readFileSync(join(dir, file), "utf8").split("\n");
      lines.forEach((line, index) => {
        // Skip comments: the rule is discussed in prose in several places.
        if (/^\s*(\/\/|\*)/.test(line)) return;
        if (AMBIGUOUS.test(line)) offenders.push(`${file}:${index + 1}  ${line.trim()}`);
      });
    }
    expect(offenders, `ambiguous patterns:\n  ${offenders.join("\n  ")}`).toEqual([]);
  });
});
