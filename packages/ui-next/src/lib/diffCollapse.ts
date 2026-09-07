import type { DiffRow } from "@srelens/core";

/**
 * A diff, cut down to the parts that changed and a little of what surrounds
 * them.
 *
 * A manifest's diff is almost entirely unchanged lines. One
 * `kubectl.kubernetes.io/last-applied-configuration` annotation is a
 * thousand-character line on its own, and printing every line of a Deployment
 * to show that one label moved buries the answer in the question. So: keep
 * `context` lines either side of every change, and stand a counted gap in for
 * each run of unchanged lines longer than that.
 *
 * The gap carries the rows it hid rather than only their count, so opening it
 * is a local decision — no second pass over the original list, and no index
 * arithmetic in the component to get wrong.
 */

export type DiffSegment =
  | { kind: "rows"; rows: DiffRow[]; from: number }
  /** A run of unchanged rows worth hiding; `rows` is what opening it shows. */
  | { kind: "gap"; rows: DiffRow[]; from: number };

/** How many additions and deletions a diff carries, counting a `replace` as both. */
export function diffCounts(rows: DiffRow[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const row of rows) {
    if (row.tag === "insert" || row.tag === "replace") added += 1;
    if (row.tag === "delete" || row.tag === "replace") removed += 1;
  }
  return { added, removed };
}

/**
 * Split rows into shown runs and hidden gaps.
 *
 * A gap is only made where hiding actually pays: a run has to be longer than
 * `2 * context + minGap` before any of it is hidden, or a two-line gap
 * standing in for two lines would be a click to see less than it replaced.
 * A diff with no changes at all is left whole — that is the "unchanged"
 * case, and hiding all of it would leave an empty panel.
 */
export function collapseDiff(rows: DiffRow[], context = 3, minGap = 4): DiffSegment[] {
  const changed = rows.some((r) => r.tag !== "same");
  if (!changed) return rows.length > 0 ? [{ kind: "rows", rows, from: 0 }] : [];

  // Which rows survive: every change, plus `context` either side of one.
  const keep = new Array<boolean>(rows.length).fill(false);
  rows.forEach((row, i) => {
    if (row.tag === "same") return;
    for (let j = Math.max(0, i - context); j <= Math.min(rows.length - 1, i + context); j++) {
      keep[j] = true;
    }
  });

  // A short hidden run costs the reader a click and saves them nothing; keep it.
  let runStart = 0;
  for (let i = 0; i <= rows.length; i++) {
    if (i < rows.length && !keep[i]) continue;
    if (i - runStart > 0 && i - runStart < minGap) {
      for (let j = runStart; j < i; j++) keep[j] = true;
    }
    runStart = i + 1;
  }

  const segments: DiffSegment[] = [];
  let start = 0;
  while (start < rows.length) {
    const shown = keep[start];
    let end = start;
    while (end < rows.length && keep[end] === shown) end += 1;
    segments.push({ kind: shown ? "rows" : "gap", rows: rows.slice(start, end), from: start });
    start = end;
  }
  return segments;
}
