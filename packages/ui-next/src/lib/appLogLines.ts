/**
 * Turning srelens's own log file into lines a screen can draw.
 *
 * The file is written by tauri-plugin-log, one entry per line, in the shape
 * `[date][time][target][LEVEL] message`. The classic view (`AppLogView`)
 * printed each line whole and only coloured it; the new design's `LogLine` has
 * a column for the timestamp, one for the level and one for the message, so the
 * line has to be taken apart before it can be drawn — which is why this module
 * exists at all, and why it is pure: the taking-apart is the part worth testing,
 * and it has nothing to do with React.
 *
 * Everything here is deliberately forgiving. A log file is a tail of whatever
 * was on disk, so the first line is routinely half an entry, a panic writes a
 * backtrace whose continuation lines carry no prefix, and neither should
 * disappear from a diagnostic view because it did not match a regex.
 */

/** Log levels emitted by tauri-plugin-log, most→least severe. */
export const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;
export type Level = (typeof LEVELS)[number];

/** Cap rendered lines so a large log can't balloon the DOM. */
export const MAX_RENDERED = 5000;

/** One parsed entry: the columns `LogLine` draws, plus the text it came from. */
export interface AppLogLine {
  /**
   * The time from a `[date][time]` prefix — the clock only, no date. `LogLine`
   * renders `ts` into a fixed ~12-character column; the date would eat that
   * whole and truncate the one thing a triager reads. The date is not lost —
   * it stays in `raw`, which Copy and the text filter both expose. `""` for a
   * line with no prefix.
   */
  ts: string;
  /**
   * The log target — third bracket of `[date][time][target][LEVEL]`, e.g.
   * `srelens::cluster`. `LogLine`'s `source` gutter renders it; `""` for a
   * line with no prefix.
   */
  source: string;
  level: Level;
  /** The text after the level bracket — a line with no bracket in full. */
  message: string;
  /** The original line, which is what a text filter searches. */
  raw: string;
}

/**
 * Where the level bracket sits in a line, and which level it is.
 *
 * Anchored on the closing bracket of the preceding field rather than on the
 * start of the line, because the target between the timestamp and the level is
 * an arbitrary module path — carried over verbatim from the classic view so
 * both designs agree on what an ERROR line is.
 */
const LEVEL_RE = /\]\[(TRACE|DEBUG|INFO|WARN|ERROR)\]/;

/** `[date][time]` at the head of a line, if it has one. */
const TS_RE = /^\[([^\]]*)\]\[([^\]]*)\]/;

/** The third bracket — the log target — when the line opens with three. */
const SOURCE_RE = /^\[[^\]]*\]\[[^\]]*\]\[([^\]]*)\]/;

/** The level of a `[date][time][target][LEVEL] message` line (INFO if absent). */
export function logLineLevel(line: string): Level {
  const match = line.match(LEVEL_RE);
  return (match?.[1] as Level) ?? "INFO";
}

/**
 * The lines of a log file, split into columns.
 *
 * Blank lines are dropped — a trailing newline is not an entry — but anything
 * with text on it survives, prefix or no prefix.
 */
export function parseAppLog(raw: string): AppLogLine[] {
  if (!raw) return [];
  return raw
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => {
      const ts = line.match(TS_RE);
      const source = line.match(SOURCE_RE);
      const level = line.match(LEVEL_RE);
      // After the level bracket when there is one; otherwise after the
      // timestamp, so a prefixed line the level regex missed still loses its
      // prefix rather than repeating it in the message column.
      const from = level ? level.index! + level[0].length : (ts?.[0].length ?? 0);
      return {
        ts: ts ? ts[2] : "",
        source: source ? source[1] : "",
        level: (level?.[1] as Level) ?? "INFO",
        message: line.slice(from).trim(),
        raw: line,
      };
    });
}

/** The lines a filter leaves, capped, plus how many actually matched. */
export interface FilteredLines {
  /** Newest {@link MAX_RENDERED} of the matches, oldest dropped. */
  lines: AppLogLine[];
  /**
   * How many lines matched before the cap trimmed them — `lines.length` when
   * nothing was capped, larger than it when the cap bit. This is what lets a
   * screen tell "your filter matched nothing" apart from "your filter matched
   * more than we can render"; a caller that only sees `lines` cannot, because
   * both end up looking at an empty or a 5000-long array.
   */
  total: number;
}

/**
 * The lines a filter leaves, newest {@link MAX_RENDERED} of them, and the
 * pre-cap match count.
 *
 * The text is matched against the whole original line rather than the message,
 * so searching for a target (`srelens::cluster`) or a timestamp still works —
 * those columns are on screen, and a filter that ignores what the user can see
 * reads as broken.
 *
 * The cap keeps the *newest* lines, because a log is read from the end: the
 * thing that just went wrong is at the bottom, and a cap that kept the oldest
 * would hide exactly the entries someone opened this screen for.
 */
export function filterLines(
  lines: AppLogLine[],
  text: string,
  level: Level | "all",
): FilteredLines {
  const query = text.toLowerCase();
  const matches = lines.filter((line) => {
    if (level !== "all" && line.level !== level) return false;
    if (query && !line.raw.toLowerCase().includes(query)) return false;
    return true;
  });
  const capped =
    matches.length > MAX_RENDERED ? matches.slice(matches.length - MAX_RENDERED) : matches;
  return { lines: capped, total: matches.length };
}
