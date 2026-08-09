import { describe, expect, it } from "vitest";
import { parseAssistantMarkdown } from "./assistantMarkdown";

describe("parseAssistantMarkdown", () => {
  it("splits blank-line-separated text into separate paragraphs", () => {
    const blocks = parseAssistantMarkdown("first paragraph\n\nsecond paragraph");
    expect(blocks).toEqual([
      { kind: "paragraph", spans: [{ kind: "text", text: "first paragraph" }] },
      { kind: "paragraph", spans: [{ kind: "text", text: "second paragraph" }] },
    ]);
  });

  it("joins hard-wrapped lines within a paragraph with a space", () => {
    const blocks = parseAssistantMarkdown("one line\nand its continuation");
    expect(blocks).toEqual([{ kind: "paragraph", spans: [{ kind: "text", text: "one line and its continuation" }] }]);
  });

  it("groups consecutive - / * lines into one bullet block", () => {
    const blocks = parseAssistantMarkdown("- first\n- second\n* third");
    expect(blocks).toEqual([
      {
        kind: "bullet",
        items: [
          [{ kind: "text", text: "first" }],
          [{ kind: "text", text: "second" }],
          [{ kind: "text", text: "third" }],
        ],
      },
    ]);
  });

  it("groups consecutive numbered lines into one ordered block", () => {
    const blocks = parseAssistantMarkdown("1. first\n2. second");
    expect(blocks).toEqual([
      {
        kind: "ordered",
        items: [
          [{ kind: "text", text: "first" }],
          [{ kind: "text", text: "second" }],
        ],
      },
    ]);
  });

  it("captures a fenced region as one code block, preserving inner newlines", () => {
    const blocks = parseAssistantMarkdown("```\nline one\nline two\n```");
    expect(blocks).toEqual([{ kind: "code", text: "line one\nline two" }]);
  });

  it("keeps a bullet-looking line inside a fence as part of the code block", () => {
    const blocks = parseAssistantMarkdown("```\n- not a list\n```");
    expect(blocks).toEqual([{ kind: "code", text: "- not a list" }]);
  });

  it("surfaces inline strong and code spans in a paragraph via parseSpans", () => {
    const blocks = parseAssistantMarkdown("this is **bold** and `code`");
    expect(blocks).toEqual([
      {
        kind: "paragraph",
        spans: [
          { kind: "text", text: "this is " },
          { kind: "strong", text: "bold" },
          { kind: "text", text: " and " },
          { kind: "code", text: "code" },
        ],
      },
    ]);
  });

  it("surfaces inline strong and code spans within bullet items via parseSpans", () => {
    const blocks = parseAssistantMarkdown("- **label:** value with `code`");
    expect(blocks).toEqual([
      {
        kind: "bullet",
        items: [
          [
            { kind: "strong", text: "label:" },
            { kind: "text", text: " value with " },
            { kind: "code", text: "code" },
          ],
        ],
      },
    ]);
  });

  it("mixes paragraphs, a bullet block, and code around each other", () => {
    const blocks = parseAssistantMarkdown("intro text\n- a\n- b\n\n```\ncode here\n```\n\noutro text");
    expect(blocks.map((b) => b.kind)).toEqual(["paragraph", "bullet", "code", "paragraph"]);
  });

  it("returns no blocks for empty text", () => {
    expect(parseAssistantMarkdown("")).toEqual([]);
  });

  it("never throws on ragged input (unterminated fence)", () => {
    expect(() => parseAssistantMarkdown("```\nunterminated")).not.toThrow();
    expect(parseAssistantMarkdown("```\nunterminated")).toEqual([{ kind: "code", text: "unterminated" }]);
  });
});
