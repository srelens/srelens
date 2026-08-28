import { cx } from "./cx";
import { toneColor, toneWash } from "./tone";

/**
 * Local copy of core's `DiffRow` shape, not an import of it.
 *
 * The kit does not depend on `@srelens/core` — `tokens-only.test.ts` asserts
 * that kit-wide — so this is the same four fields, structurally, rather than
 * a reference to the type `diffTextLines` returns. Anything that already
 * produces a `core` `DiffRow[]` is assignable here without a cast.
 */
export interface DiffRow {
  tag: "same" | "insert" | "delete" | "replace";
  left: string | null;
  right: string | null;
}

export interface DiffLinesProps {
  rows: DiffRow[];
  className?: string;
}

interface Line {
  key: string;
  tag: "same" | "insert" | "delete";
  text: string;
}

/**
 * One `DiffRow` becomes one printed line, except `replace`.
 *
 * `replace` is the row-shape trap this component exists to get right: it is
 * ONE row with both `left` and `right` present, emitted when a line changed
 * rather than being purely added or purely removed. Read only for its tag —
 * `left`/`right`'s mere presence is not enough, because a `same` row also
 * carries both — it becomes its own deletion line (from `left`) immediately
 * followed by its own addition line (from `right`), matching exactly how a
 * changed line already reads when `diffTextLines`' ordinary path hands it
 * over as an adjacent delete/insert pair instead. Two lines come out, but
 * from the one row that carried them; a delete row and an insert row
 * elsewhere in the same list are left as the single line each already is.
 */
function toLines(rows: DiffRow[]): Line[] {
  const lines: Line[] = [];
  rows.forEach((row, i) => {
    if (row.tag === "same") {
      lines.push({ key: `${i}`, tag: "same", text: row.left ?? row.right ?? "" });
    } else if (row.tag === "delete") {
      lines.push({ key: `${i}`, tag: "delete", text: row.left ?? "" });
    } else if (row.tag === "insert") {
      lines.push({ key: `${i}`, tag: "insert", text: row.right ?? "" });
    } else {
      if (row.left !== null) lines.push({ key: `${i}-del`, tag: "delete", text: row.left });
      if (row.right !== null) lines.push({ key: `${i}-ins`, tag: "insert", text: row.right });
    }
  });
  return lines;
}

const MARK: Record<Line["tag"], string> = { same: " ", delete: "-", insert: "+" };

/**
 * A text diff — `DiffRow[]` from a revision-to-revision compare — drawn in
 * the design's tones: additions on `--ok-wash` in `--ok`, deletions on
 * `--sev-wash` in `--sev`, context in `--ink-soft`, monospace at 11px. (§16)
 *
 * This is not classic's `DiffView`. That component takes a `DiffDoc` and
 * draws a manifest dry-run — a resource heading, a `New resource` badge, a
 * `No changes` state — none of which a revision diff wants, and it is styled
 * in classic's own classes rather than these tokens. This one knows nothing
 * beyond the rows it is handed.
 *
 * An empty list renders nothing rather than an empty frame, the same rule
 * `RawError` and `PairList` follow: a diff pane with nothing to say should
 * not leave a blank box behind for its caller to notice and hide.
 */
export function DiffLines({ rows, className }: DiffLinesProps) {
  const lines = toLines(rows);
  if (lines.length === 0) return null;

  return (
    <div className={cx("flex flex-col font-mono text-[0.6875rem] leading-relaxed", className)}>
      {lines.map((l) => (
        <div
          key={l.key}
          data-slot="line"
          data-tag={l.tag}
          className="flex gap-2 whitespace-pre-wrap break-all px-2 py-px"
          style={
            l.tag === "same"
              ? { color: "var(--ink-soft)" }
              : {
                  color: toneColor(l.tag === "insert" ? "ok" : "sev"),
                  background: toneWash(l.tag === "insert" ? "ok" : "sev"),
                }
          }
        >
          <span className="shrink-0 select-none opacity-70">{MARK[l.tag]}</span>
          <span className="min-w-0 flex-1">{l.text}</span>
        </div>
      ))}
    </div>
  );
}
