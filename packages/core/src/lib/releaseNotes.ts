// Parser for the release notes the updater shows. The notes come from our own
// release workflow, so this handles that dialect rather than markdown at large:
// `### Heading` sections, `-` bullets, plain paragraphs, and inline `**bold**`
// and `code`. Anything it doesn't recognise falls through as a paragraph, so a
// hand-edited GitHub release body still renders as readable text instead of
// disappearing.
//
// Deliberately a hand-rolled subset, not a markdown dependency: the input is
// ours, the surface is four constructs, and the output feeds React elements —
// never `dangerouslySetInnerHTML` — so untrusted note text cannot inject markup.

/** An inline run within a line: plain text, `code`, or **bold**. */
export type NoteSpan =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "strong"; text: string };

export type NoteBlock =
  | { kind: "heading"; spans: NoteSpan[] }
  | { kind: "list"; items: NoteSpan[][] }
  | { kind: "paragraph"; spans: NoteSpan[] };

/** Split one line into inline spans. Unclosed markers stay literal text. */
export function parseSpans(line: string): NoteSpan[] {
  const spans: NoteSpan[] = [];
  // Alternation order matters: `**` must be tried before a single `*` would be,
  // and backticks bind tighter than emphasis in the notes we generate.
  const pattern = /`([^`]+)`|\*\*([^*]+)\*\*/g;
  let last = 0;
  for (let m = pattern.exec(line); m; m = pattern.exec(line)) {
    if (m.index > last) spans.push({ kind: "text", text: line.slice(last, m.index) });
    if (m[1] !== undefined) spans.push({ kind: "code", text: m[1] });
    else spans.push({ kind: "strong", text: m[2] });
    last = m.index + m[0].length;
  }
  if (last < line.length) spans.push({ kind: "text", text: line.slice(last) });
  return spans;
}

/**
 * Parse release-note text into renderable blocks. Consecutive bullets collapse
 * into one list; blank lines end the current block; consecutive plain lines join
 * into a single paragraph so hard-wrapped prose doesn't render as fragments.
 */
export function parseReleaseNotes(notes: string): NoteBlock[] {
  const blocks: NoteBlock[] = [];
  let list: NoteSpan[][] | null = null;
  let para: string[] = [];

  const flushParagraph = () => {
    if (para.length) {
      blocks.push({ kind: "paragraph", spans: parseSpans(para.join(" ")) });
      para = [];
    }
  };
  const flushList = () => {
    if (list) {
      blocks.push({ kind: "list", items: list });
      list = null;
    }
  };
  const flushAll = () => {
    flushParagraph();
    flushList();
  };

  for (const raw of notes.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) {
      flushAll();
      continue;
    }
    // `[^\S\n]`/`[^\n]` rather than `\s`/`.`: those two overlap on every
    // space, so a line the pattern ultimately rejects can be split many ways
    // (js/polynomial-redos, #50 and #51). The equivalents in
    // assistantMarkdown.ts were tightened in #313; these were missed because
    // they are inline rather than named constants.
    const heading = /^#{1,6}[^\S\n]+([^\n]*)$/.exec(line);
    if (heading) {
      flushAll();
      blocks.push({ kind: "heading", spans: parseSpans(heading[1]) });
      continue;
    }
    const bullet = /^[-*][^\S\n]+([^\n]*)$/.exec(line);
    if (bullet) {
      // A bullet ends a paragraph but continues any run of bullets above it.
      flushParagraph();
      list ??= [];
      list.push(parseSpans(bullet[1]));
      continue;
    }
    flushList();
    para.push(line);
  }
  flushAll();
  return blocks;
}
