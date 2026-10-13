import { instantOf, VOLUME_STEPS, wallClock, type TimeRange } from "@srelens/core";

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

export const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
export const DAY = 24 * HOUR;

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
  if (step % HOUR === 0) return `${step / HOUR}h`;
  if (step % MINUTE === 0) return `${step / MINUTE}m`;
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

const DEFAULT_TICKS = 6;

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
 * The ticks of a calendar grid inside `[from, to]`: `at(k)` is the instant of
 * the k-th, for k from `first` to `last`. Undefined past `cap`. An instant that
 * is not a number lies past the edge of what a Date holds, and is no tick.
 */
function gridTicks(
  from: number,
  to: number,
  cap: number,
  first: number,
  last: number,
  at: (k: number) => number,
): number[] | undefined {
  const ticks: number[] = [];
  for (let k = first; k <= last; k += 1) {
    const t = at(k);
    // Not past the last: a day the clocks skipped can map two grid points to one instant.
    if (!(t >= from && t <= to) || (ticks.length > 0 && t <= ticks[ticks.length - 1])) continue;
    if (ticks.length === cap) return undefined;
    ticks.push(t);
  }
  return ticks;
}

/** The local calendar day `ms` falls on, as a count of days since 1970-01-01: a 23-hour day is one. */
const dayNumber = (ms: number) => {
  const d = new Date(ms);
  return Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / DAY;
};

/** The first Monday after the epoch, as a `dayNumber`: 1970-01-05. */
const FIRST_MONDAY = 4;

/**
 * Local Mondays at midnight, `weeks` apart. The grid is counted from the
 * epoch's first Monday, not from `from`, so a live axis keeps its ticks on the
 * same Mondays as the window slides.
 */
function mondayTicks(from: number, to: number, weeks: number, cap: number): number[] | undefined {
  const days = 7 * weeks;
  return gridTicks(
    from,
    to,
    cap,
    Math.ceil((dayNumber(from) - FIRST_MONDAY) / days),
    Math.floor((dayNumber(to) - FIRST_MONDAY) / days),
    // A day past the end of the month rolls over into the next, so the count needs no calendar of its own.
    (k) => new Date(1970, 0, 1 + FIRST_MONDAY + k * days).getTime(),
  );
}

/** Local 1sts at midnight, `months` apart, counted from January 1970 (every step offered divides a year). */
function monthTicks(from: number, to: number, months: number, cap: number): number[] | undefined {
  const monthOf = (ms: number) => {
    const d = new Date(ms);
    return (d.getFullYear() - 1970) * 12 + d.getMonth();
  };
  return gridTicks(
    from,
    to,
    cap,
    Math.ceil(monthOf(from) / months),
    Math.floor(monthOf(to) / months),
    (k) => new Date(1970, k * months, 1).getTime(),
  );
}

/**
 * Ticks for a time axis over `[from, to]`: the smallest round interval that
 * puts at most `maxTicks` of them inside, each on a multiple of the interval
 * on the READER'S clock (so +05:45 gets :00 and :30, not :15 and :45). A span
 * that even a day overfills is given whole days, as few as fit, while that is
 * under a week a tick. Past that the ticks name the calendar instead: Mondays
 * one, two or four weeks apart, then the 1st of a month, one to twelve apart.
 */
export function timeTicks(from: number, to: number, maxTicks = DEFAULT_TICKS): number[] {
  if (!Number.isFinite(from) || !Number.isFinite(to) || !(to > from)) return [];
  // A cap that is NaN or infinite would never stop a fine interval from filling the axis.
  const cap = Number.isFinite(maxTicks) ? Math.max(1, Math.floor(maxTicks)) : DEFAULT_TICKS;
  for (const step of VOLUME_STEPS) {
    const ticks = ticksEvery(from, to, step, cap);
    if (ticks !== undefined) return ticks;
  }
  const days = Math.ceil((wallClock(to) - wallClock(from)) / DAY / Math.max(1, cap - 1));
  if (days >= 7) {
    for (const weeks of [1, 2, 4]) {
      const ticks = mondayTicks(from, to, weeks, cap);
      if (ticks !== undefined) return ticks;
    }
    for (const months of [1, 2, 3, 6, 12]) {
      const ticks = monthTicks(from, to, months, cap);
      if (ticks !== undefined) return ticks;
    }
  }
  // ponytail: a span over five years is back on whole days from the epoch; year-aligned ticks if a log ever spans that.
  return ticksEvery(from, to, days * DAY, Infinity) ?? [];
}

/** Whether `ms` opens its local day: midnight, or the first instant after clocks that skipped it. */
const startsDay = (ms: number) => dateOf(ms) !== dateOf(ms - 1);

/**
 * The labels for `timeTicks`: a clock reading, with seconds when the ticks are
 * closer than a minute, and the date on a tick that starts a local day, where a
 * bare `00:00` would not say which. A day whose midnight the clocks skipped
 * starts at the hour they jumped to, and is dated all the same. A date reads
 * `MM-DD` until the ticks run into a second year; then every one carries its
 * year, or `01-01` and `07-01` would each appear twice and say nothing of which.
 */
export function tickLabels(ticks: readonly number[]): string[] {
  const seconds = ticks.some((t) => new Date(t).getSeconds() !== 0);
  const yearOf = (t: number) => new Date(t).getFullYear();
  const withYear = yearOf(ticks[0]) !== yearOf(ticks[ticks.length - 1]);
  return ticks.map((t) => {
    const clock = seconds ? clockTime(t) : clockTime(t).slice(0, 5);
    return startsDay(t) ? `${withYear ? dateOf(t) : dateOf(t).slice(5)} ${clock}` : clock;
  });
}
