import { instantOf, wallClock, type TimeRange } from "@srelens/core";

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

const isMidnight = (ms: number) => {
  const d = new Date(ms);
  return d.getHours() === 0 && d.getMinutes() === 0 && d.getSeconds() === 0 && d.getMilliseconds() === 0;
};

/** `2026-10-12 10:33`, local: a stamp that names its day, to the second only when asked. */
export function dateTime(ms: number, seconds = false): string {
  const clock = clockTime(ms);
  return `${dateOf(ms)} ${seconds ? clock : clock.slice(0, 5)}`;
}

/**
 * Whether a clock reading alone would be ambiguous for `[from, to)`: it covers
 * a day or more, or crosses a midnight. `to` is exclusive, so a span ending
 * exactly at midnight stays on its day unless it is a whole day long. A day
 * the clocks changed in is 23 hours, so "whole" is read off the clock too: one
 * local midnight to a later one.
 */
export function spansDays(from: number, to: number): boolean {
  if (to - from >= DAY || dateOf(from) !== dateOf(to - 1)) return true;
  return isMidnight(from) && isMidnight(to) && dateOf(from) !== dateOf(to);
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

/** The mantissas a count axis may top out at: fine enough that a peak of 110 reads 120, not 200. */
const NICE = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];

/** The smallest of 1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 times a power of ten that is at least `n`: a round top for a count axis. Never under 1, as a count is not. */
export function niceCeil(n: number): number {
  if (!(n > 1)) return 1;
  const power = 10 ** Math.floor(Math.log10(n));
  return NICE.map((m) => m * power).find((c) => c >= n) ?? 10 * power;
}

const SECOND = 1000;
const MINUTE = 60_000;
const HOUR = 3_600_000;
const DEFAULT_TICKS = 6;
const TICK_STEPS = [
  ...[1, 2, 5, 10, 15, 30].map((n) => n * SECOND),
  ...[1, 2, 5, 10, 15, 30].map((n) => n * MINUTE),
  ...[1, 2, 3, 6, 12].map((n) => n * HOUR),
  DAY,
];

/** The instants in `[from, to]` on multiples of `step` of the local clock, or undefined past `cap`. */
function ticksEvery(from: number, to: number, step: number, cap: number): number[] | undefined {
  const ticks: number[] = [];
  for (let k = Math.ceil(wallClock(from) / step); ; k += 1) {
    const at = instantOf(k * step);
    if (at > to) return ticks;
    // Before `from`: the first copy of a repeated hour. Not past the last: a skipped hour maps every tick in it to the transition.
    if (at < from || (ticks.length > 0 && at <= ticks[ticks.length - 1])) continue;
    if (ticks.length === cap) return undefined;
    ticks.push(at);
  }
}

/**
 * Ticks for a time axis over `[from, to]`: the smallest round interval that
 * puts at most `maxTicks` of them inside, each on a multiple of the interval
 * on the READER'S clock (so +05:45 gets :00 and :30, not :15 and :45). A span
 * of weeks that even a day overfills is given whole days, as few as fit.
 */
export function timeTicks(from: number, to: number, maxTicks = DEFAULT_TICKS): number[] {
  if (!Number.isFinite(from) || !Number.isFinite(to) || !(to > from)) return [];
  // A cap that is NaN or infinite would never stop a fine interval from filling the axis.
  const cap = Number.isFinite(maxTicks) ? Math.max(1, Math.floor(maxTicks)) : DEFAULT_TICKS;
  for (const step of TICK_STEPS) {
    const ticks = ticksEvery(from, to, step, cap);
    if (ticks !== undefined) return ticks;
  }
  const days = Math.ceil((wallClock(to) - wallClock(from)) / DAY / Math.max(1, cap - 1));
  return ticksEvery(from, to, days * DAY, Infinity) ?? [];
}

/**
 * The labels for `timeTicks`: a clock reading, with seconds when the ticks are
 * closer than a minute, and the date on a tick that starts a local day, where a
 * bare `00:00` would not say which.
 */
export function tickLabels(ticks: readonly number[]): string[] {
  const seconds = ticks.some((t) => new Date(t).getSeconds() !== 0);
  return ticks.map((t) => {
    const clock = seconds ? clockTime(t) : clockTime(t).slice(0, 5);
    return isMidnight(t) ? `${dateOf(t).slice(5)} ${clock}` : clock;
  });
}
