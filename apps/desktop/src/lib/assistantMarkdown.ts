// Minimal markdown renderer for assistant chat replies — not a CommonMark
// parser. The assistant only ever produces a handful of constructs (bold,
// inline code, bullets, numbered lists, fenced code, paragraphs), so this
// covers exactly those, mirroring the hand-rolled approach `releaseNotes.ts`
// takes for update notes. Output feeds React elements only — no
// `dangerouslySetInnerHTML` — so a reply can never inject markup.

import { parseSpans, type NoteSpan } from "./releaseNotes";

export type MdBlock =
  | { kind: "paragraph"; spans: NoteSpan[] }
  | { kind: "bullet"; items: NoteSpan[][] }
  | { kind: "ordered"; items: NoteSpan[][] }
  | { kind: "code"; text: string };

const BULLET = /^[-*]\s+(.*)$/;
const ORDERED = /^\d+\.\s+(.*)$/;

/** Parse assistant reply text into renderable blocks. No nesting, no links,
 * no headings beyond inline `**bold**` — see module comment for scope. */
export function parseAssistantMarkdown(md: string): MdBlock[] {
  const blocks: MdBlock[] = [];
  let para: string[] = [];
  let bullet: NoteSpan[][] | null = null;
  let ordered: NoteSpan[][] | null = null;
  let code: string[] | null = null;

  const flushParagraph = () => {
    if (para.length) {
      blocks.push({ kind: "paragraph", spans: parseSpans(para.join(" ")) });
      para = [];
    }
  };
  const flushBullet = () => {
    if (bullet) {
      blocks.push({ kind: "bullet", items: bullet });
      bullet = null;
    }
  };
  const flushOrdered = () => {
    if (ordered) {
      blocks.push({ kind: "ordered", items: ordered });
      ordered = null;
    }
  };
  const flushCode = () => {
    if (code) {
      blocks.push({ kind: "code", text: code.join("\n") });
      code = null;
    }
  };

  for (const raw of (md ?? "").split(/\r?\n/)) {
    if (code) {
      if (raw.trim().startsWith("```")) flushCode();
      else code.push(raw);
      continue;
    }
    const line = raw.trim();
    if (line.startsWith("```")) {
      flushParagraph();
      flushBullet();
      flushOrdered();
      code = [];
      continue;
    }
    if (!line) {
      flushParagraph();
      flushBullet();
      flushOrdered();
      continue;
    }
    const bulletMatch = BULLET.exec(line);
    if (bulletMatch) {
      flushParagraph();
      flushOrdered();
      bullet ??= [];
      bullet.push(parseSpans(bulletMatch[1]));
      continue;
    }
    const orderedMatch = ORDERED.exec(line);
    if (orderedMatch) {
      flushParagraph();
      flushBullet();
      ordered ??= [];
      ordered.push(parseSpans(orderedMatch[1]));
      continue;
    }
    flushBullet();
    flushOrdered();
    para.push(line);
  }
  // An unterminated fence still renders as code rather than vanishing.
  flushCode();
  flushParagraph();
  flushBullet();
  flushOrdered();
  return blocks;
}
