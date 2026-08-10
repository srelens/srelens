// Minimal markdown renderer for assistant chat replies — not a CommonMark
// parser. The assistant produces a handful of constructs (headings, bold,
// inline code, bullets, numbered lists, GFM tables, fenced code, paragraphs),
// so this covers exactly those, mirroring the hand-rolled approach
// `releaseNotes.ts` takes for update notes. Output feeds React elements only —
// no `dangerouslySetInnerHTML` — so a reply can never inject markup.

import { parseSpans, type NoteSpan } from "./releaseNotes";

export type MdBlock =
  | { kind: "heading"; level: number; spans: NoteSpan[] }
  | { kind: "paragraph"; spans: NoteSpan[] }
  | { kind: "bullet"; items: NoteSpan[][] }
  | { kind: "ordered"; items: NoteSpan[][] }
  | { kind: "table"; headers: NoteSpan[][]; rows: NoteSpan[][][] }
  | { kind: "code"; text: string };

const HEADING = /^(#{1,6})\s+(.*)$/;
const BULLET = /^[-*]\s+(.*)$/;
const ORDERED = /^\d+\.\s+(.*)$/;
// A GFM header/body separator row: only pipes, dashes, colons, spaces, with at
// least one dash (e.g. `|---|:--:|`).
const TABLE_SEP = /^\s*\|?[\s|:-]*-[\s|:-]*\|?\s*$/;

/** A line looks like a table row if it has an interior pipe (`a | b`), so a
 * lone trailing/leading `|` in prose isn't mistaken for one. */
function looksLikeTableRow(line: string): boolean {
  return /\S\s*\|\s*\S/.test(line);
}

/** Split `| a | b |` into `["a", "b"]`, trimming cells and dropping the empty
 * leading/trailing cells the outer pipes produce. */
function splitRow(line: string): string[] {
  const cells = line.split("|").map((c) => c.trim());
  if (cells.length && cells[0] === "") cells.shift();
  if (cells.length && cells[cells.length - 1] === "") cells.pop();
  return cells;
}

/** Parse assistant reply text into renderable blocks. No nesting, no links. */
export function parseAssistantMarkdown(md: string): MdBlock[] {
  const blocks: MdBlock[] = [];
  let para: string[] = [];
  let bullet: NoteSpan[][] | null = null;
  let ordered: NoteSpan[][] | null = null;
  let table: string[] | null = null;
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
  const flushTable = () => {
    if (!table) return;
    const rows = table;
    table = null;
    // A real GFM table has a header row, a separator row, then body rows.
    // Anything else (a stray pipe-y line) degrades to a paragraph rather than
    // vanishing.
    if (rows.length >= 2 && TABLE_SEP.test(rows[1])) {
      const headers = splitRow(rows[0]).map(parseSpans);
      const body = rows.slice(2).map((r) => splitRow(r).map(parseSpans));
      blocks.push({ kind: "table", headers, rows: body });
    } else {
      for (const r of rows) para.push(r);
      flushParagraph();
    }
  };
  const flushCode = () => {
    if (code) {
      blocks.push({ kind: "code", text: code.join("\n") });
      code = null;
    }
  };
  const flushAll = () => {
    flushParagraph();
    flushBullet();
    flushOrdered();
    flushTable();
  };

  for (const raw of (md ?? "").split(/\r?\n/)) {
    if (code) {
      if (raw.trim().startsWith("```")) flushCode();
      else code.push(raw);
      continue;
    }
    const line = raw.trim();
    if (line.startsWith("```")) {
      flushAll();
      code = [];
      continue;
    }
    if (!line) {
      flushAll();
      continue;
    }
    const headingMatch = HEADING.exec(line);
    if (headingMatch) {
      flushAll();
      blocks.push({ kind: "heading", level: headingMatch[1].length, spans: parseSpans(headingMatch[2]) });
      continue;
    }
    const bulletMatch = BULLET.exec(line);
    if (bulletMatch) {
      flushParagraph();
      flushOrdered();
      flushTable();
      bullet ??= [];
      bullet.push(parseSpans(bulletMatch[1]));
      continue;
    }
    const orderedMatch = ORDERED.exec(line);
    if (orderedMatch) {
      flushParagraph();
      flushBullet();
      flushTable();
      ordered ??= [];
      ordered.push(parseSpans(orderedMatch[1]));
      continue;
    }
    if (looksLikeTableRow(line)) {
      flushParagraph();
      flushBullet();
      flushOrdered();
      table ??= [];
      table.push(line);
      continue;
    }
    flushBullet();
    flushOrdered();
    flushTable();
    para.push(line);
  }
  // An unterminated fence still renders as code rather than vanishing.
  flushCode();
  flushAll();
  return blocks;
}
