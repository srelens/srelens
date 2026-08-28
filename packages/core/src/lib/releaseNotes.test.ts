import { describe, expect, it } from "vitest";
import { parseReleaseNotes, parseSpans } from "./releaseNotes";

describe("parseSpans", () => {
  it("splits inline code and bold out of surrounding text", () => {
    expect(parseSpans("**Commit:** `abc123` landed")).toEqual([
      { kind: "strong", text: "Commit:" },
      { kind: "text", text: " " },
      { kind: "code", text: "abc123" },
      { kind: "text", text: " landed" },
    ]);
  });

  it("leaves unmatched markers as literal text", () => {
    expect(parseSpans("2 ** 3 is not bold")).toEqual([{ kind: "text", text: "2 ** 3 is not bold" }]);
    expect(parseSpans("a ` dangling tick")).toEqual([{ kind: "text", text: "a ` dangling tick" }]);
  });

  it("returns nothing for an empty line", () => {
    expect(parseSpans("")).toEqual([]);
  });
});

describe("parseReleaseNotes", () => {
  // Exactly what the stable release workflow emits.
  it("parses the generated stable-release shape", () => {
    const blocks = parseReleaseNotes(
      "### Features\n- **forward:** resilient port-forwards (#173)\n- web mode (#165)\n\n### Fixes\n- **tabs:** cache list rows\n",
    );
    expect(blocks).toEqual([
      { kind: "heading", spans: [{ kind: "text", text: "Features" }] },
      {
        kind: "list",
        items: [
          [
            { kind: "strong", text: "forward:" },
            { kind: "text", text: " resilient port-forwards (#173)" },
          ],
          [{ kind: "text", text: "web mode (#165)" }],
        ],
      },
      { kind: "heading", spans: [{ kind: "text", text: "Fixes" }] },
      {
        kind: "list",
        items: [
          [
            { kind: "strong", text: "tabs:" },
            { kind: "text", text: " cache list rows" },
          ],
        ],
      },
    ]);
  });

  // And what the dev-channel job emits — prose, bold labels, inline code.
  it("parses the dev pre-release shape", () => {
    const blocks = parseReleaseNotes(
      "Development pre-release — unstable.\n\n**Commit:** `deadbeef`\n\n### Recent changes\n- fix: a thing (abc1234)\n",
    );
    expect(blocks.map((b) => b.kind)).toEqual(["paragraph", "paragraph", "heading", "list"]);
  });

  it("joins hard-wrapped prose into one paragraph but keeps bullets separate", () => {
    const blocks = parseReleaseNotes("one line\nand its continuation\n- a bullet\n- another");
    expect(blocks).toEqual([
      { kind: "paragraph", spans: [{ kind: "text", text: "one line and its continuation" }] },
      {
        kind: "list",
        items: [[{ kind: "text", text: "a bullet" }], [{ kind: "text", text: "another" }]],
      },
    ]);
  });

  it("treats a blank line as a block boundary between two lists", () => {
    const blocks = parseReleaseNotes("- one\n\n- two");
    expect(blocks).toEqual([
      { kind: "list", items: [[{ kind: "text", text: "one" }]] },
      { kind: "list", items: [[{ kind: "text", text: "two" }]] },
    ]);
  });

  it("accepts * bullets and any heading depth", () => {
    const blocks = parseReleaseNotes("# Title\n* starred");
    expect(blocks).toEqual([
      { kind: "heading", spans: [{ kind: "text", text: "Title" }] },
      { kind: "list", items: [[{ kind: "text", text: "starred" }]] },
    ]);
  });

  it("keeps unrecognised text rather than dropping it", () => {
    expect(parseReleaseNotes("> a quote\n| a table |")).toEqual([
      { kind: "paragraph", spans: [{ kind: "text", text: "> a quote | a table |" }] },
    ]);
  });

  it("returns no blocks for empty or whitespace-only notes", () => {
    expect(parseReleaseNotes("")).toEqual([]);
    expect(parseReleaseNotes("\n  \n\n")).toEqual([]);
  });
});
