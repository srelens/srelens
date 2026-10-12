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

/** `10:33:00 – 10:34:30`, dated only when the window crosses midnight. */
export function rangeLabel(range: TimeRange): string {
  // `to` is exclusive, so a window ending exactly at midnight stays on its day.
  if (dateOf(range.from) === dateOf(range.to - 1)) return `${clockTime(range.from)} – ${clockTime(range.to)}`;
  return `${dateOf(range.from)} ${clockTime(range.from)} – ${dateOf(range.to)} ${clockTime(range.to)}`;
}

/** `30s`, `5m`, `1h`, `1d`. */
export function stepLabel(step: number): string {
  if (step % 86_400_000 === 0) return `${step / 86_400_000}d`;
  if (step % 3_600_000 === 0) return `${step / 3_600_000}h`;
  if (step % 60_000 === 0) return `${step / 60_000}m`;
  return `${step / 1000}s`;
}
