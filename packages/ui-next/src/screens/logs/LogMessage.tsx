import type { ReactNode } from "react";
import { highlightRanges, inlineFields, type QueryClause } from "@srelens/core";
import { toneWash } from "@srelens/ui-kit";

/** What the message column needs from a row. */
export interface MessageRow {
  readonly message: string;
  readonly structured: boolean;
  readonly fields: Readonly<Record<string, string>>;
  readonly truncated: boolean;
}

/** What parts the message from its fields. */
const GAP = "  ";

/**
 * A row's message cell: the message, then (for a structured row) its other
 * fields in faint `key=value`, with the query's matches marked across both.
 *
 * `""` for a row with nothing to show, so the kit's `(no message)` says so.
 * A fragment would count as content and draw a blank cell.
 */
export function renderMessage(row: MessageRow, clauses: readonly QueryClause[]): ReactNode {
  const inline = row.structured ? inlineFields(row.fields) : "";
  const gap = row.message !== "" && inline !== "" ? GAP : "";
  const shown = `${row.message}${gap}${inline}`;
  if (shown === "" && !row.truncated) return "";
  // Where the faint part starts: after the message, or at 0 when there is none.
  const split = row.message === "" ? 0 : Math.min(row.message.length, shown.length);
  const ranges = clauses.length === 0 ? [] : highlightRanges(shown, clauses);
  const rest = split + gap.length;
  return (
    <>
      {marked(shown.slice(0, split), ranges, 0)}
      {split < shown.length && (
        <span className="text-faint">
          {/* The row is `white-space: nowrap`, which collapses the gap to one space. Only the gap keeps its
              spaces: pre-wrap on the whole faint part would let the row break, and the window needs its height. */}
          {gap !== "" && <span className="whitespace-pre">{marked(gap, ranges, split)}</span>}
          {marked(shown.slice(rest), ranges, rest)}
        </span>
      )}
      {/* A cut line says so at its end, in the message's own flow (#747): the
          trailing slot sits beside the message box, and with Wrap off a long
          message runs past that box and under it. */}
      {row.truncated && <span className="text-faint"> [line cut: too long]</span>}
    </>
  );
}

function marked(text: string, ranges: readonly (readonly [number, number])[], offset: number): ReactNode[] {
  const out: ReactNode[] = [];
  let at = 0;
  for (const [s, e] of ranges) {
    const start = Math.max(s - offset, 0);
    const end = Math.min(e - offset, text.length);
    if (end <= start) continue;
    if (start > at) out.push(text.slice(at, start));
    out.push(
      <mark key={start} style={{ background: toneWash("warn"), color: "inherit" }}>
        {text.slice(start, end)}
      </mark>,
    );
    at = end;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}
