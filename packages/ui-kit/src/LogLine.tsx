import type { ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";
import { toneColor, type Tone } from "./tone";

export interface LogLineProps {
  /** Already formatted. The kit does not own the clock or the locale. */
  ts: ReactNode;
  /** The stream the line came from — a container, a subsystem, a file. */
  source?: ReactNode;
  /** Printed as it arrives; only the tone is derived from it. */
  level?: string;
  message: ReactNode;
  /** Overrules the tone read off `level`, for a line singled out by something else. */
  tone?: Tone;
  /** Tints the source, for telling two interleaved streams apart. */
  sourceTone?: Tone;
  /** Pinned to the end of the line — an ask chip, a copy button. */
  children?: ReactNode;
  className?: string;
}

/**
 * The levels a log stream actually uses, and what each one means. Written here
 * rather than taken from a logging library, because the kit knows nothing about
 * where the lines came from — and a stream that spells its own levels
 * differently can still pass `tone` per line.
 */
const LEVEL_TONE: Record<string, Tone> = {
  fatal: "sev",
  critical: "sev",
  crit: "sev",
  error: "sev",
  err: "sev",
  panic: "sev",
  dpanic: "sev",
  emergency: "sev",
  emerg: "sev",
  alert: "sev",
  warning: "warn",
  warn: "warn",
  notice: "info",
  info: "info",
  debug: "muted",
  trace: "muted",
};

/**
 * One line of a log stream: when, where from, how bad, and what it said.
 *
 * The mock took `sourceColor?: string`, which is a hole straight through the
 * token rule — any caller could hand it `#ff0000` and the line would stop
 * following the theme. It becomes a {@link Tone}, so the palette stays in one
 * place and a source tint is chosen from the same six colours as everything
 * else.
 *
 * `tone` was required, which sounds strict and is the opposite: it made every
 * call site responsible for mapping "error" to red, so the same word arrived
 * red on one screen and grey on the next. It is derived from the level here and
 * the prop is left as an override, for the line that is singled out by
 * something other than its level.
 *
 * The four columns are fixed widths, and they stay fixed even when they are
 * empty — the one place in this kit where an empty slot keeps its box. Every
 * other component drops an empty wrapper to avoid a band of dead space; here
 * the boxes are the gutters of a grid a thousand lines deep, and a line that
 * omits its level shunts its message out of line with every message above it.
 * The trailing slot is the exception to the exception: it sits after the
 * message rather than before it, so nothing lines up against it, and it goes
 * through `filled` like any other.
 *
 * An empty message says so rather than rendering a blank row, which in a stream
 * is indistinguishable from a rendering fault. Nothing here is interactive and
 * nothing carries a `title`: the text is all in the DOM, so a screen reader
 * reads the source in full even where the column truncates it on screen. (#320)
 */
export function LogLine({
  ts,
  source,
  level,
  message,
  tone,
  sourceTone = "accent",
  children,
  className,
}: LogLineProps) {
  const resolved = tone ?? LEVEL_TONE[(level ?? "").trim().toLowerCase()] ?? "muted";

  return (
    <div className={cx("logline flex items-start gap-3 px-2.5 py-[2px]", className)}>
      <span data-slot="ts" className="w-[86px] shrink-0 truncate text-faint">
        {ts}
      </span>
      <span
        data-slot="source"
        className="w-[104px] shrink-0 truncate"
        style={{ color: toneColor(sourceTone) }}
      >
        {source}
      </span>
      <span
        data-slot="level"
        className="w-[44px] shrink-0 truncate uppercase"
        style={{ color: toneColor(resolved) }}
      >
        {level}
      </span>
      <span data-slot="message" className="min-w-0 flex-1 break-all text-soft">
        {filled(message) ? message : <span className="text-faint">(no message)</span>}
      </span>
      {filled(children) && (
        <span data-slot="trailing" className="shrink-0">
          {children}
        </span>
      )}
    </div>
  );
}
