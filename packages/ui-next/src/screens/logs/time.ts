import type { TimeRange } from "@srelens/core";

const pad = (n: number) => String(n).padStart(2, "0");

/** `10:33:05`, local: the reader's clock, which is what the time column shows too. */
export function clockTime(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

const dateOf = (ms: number) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

export const DAY = 86_400_000;

/** `2026-10-12 10:33`, local: a stamp that names its day, to the second only when asked. */
export function dateTime(ms: number, seconds = false): string {
  const clock = clockTime(ms);
  return `${dateOf(ms)} ${seconds ? clock : clock.slice(0, 5)}`;
}

/**
 * Whether a clock reading alone would be ambiguous for `[from, to)`: it covers
 * a day or more, or crosses a midnight. `to` is exclusive, so a span ending
 * exactly at midnight stays on its day unless it is a whole day long.
 */
export function spansDays(from: number, to: number): boolean {
  return to - from >= DAY || dateOf(from) !== dateOf(to - 1);
}

/** `10:33:00 – 10:34:30`, dated when the window covers a day or crosses midnight. */
export function rangeLabel(range: TimeRange): string {
  if (!spansDays(range.from, range.to)) return `${clockTime(range.from)} – ${clockTime(range.to)}`;
  return `${dateOf(range.from)} ${clockTime(range.from)} – ${dateOf(range.to)} ${clockTime(range.to)}`;
}

/** `30s`, `5m`, `1h`, `1d`. */
export function stepLabel(step: number): string {
  if (step % DAY === 0) return `${step / DAY}d`;
  if (step % 3_600_000 === 0) return `${step / 3_600_000}h`;
  if (step % 60_000 === 0) return `${step / 60_000}m`;
  return `${step / 1000}s`;
}
