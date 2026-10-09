import type { NativeTimeseriesThreshold, NativeTimeseriesUnit } from "@srelens/core/lib/nativeComponents";

/**
 * Most points drawn per series. The catalog accepts up to 1,000 samples; the
 * plot draws at most this many and the values table still lists every sample.
 */
export const MAX_DRAWN_POINTS = 400;

/** One unbroken run of `[time, value]` points; a gap (null) starts the next. */
export type Segment = Array<[number, number]>;

/**
 * Min–max bucketing. Under the cap every sample is kept. Over it, the samples
 * are split into `floor(cap / 2)` contiguous buckets of equal count, and each
 * bucket keeps its lowest and highest real sample, in time order. Nothing is
 * averaged or interpolated, so every drawn point is a sample the provider sent,
 * and the series' global minimum and maximum always survive — a one-sample
 * spike is exactly what averaging-based reduction hides. A gap anywhere in a
 * bucket still breaks the line there.
 */
export function downsample(times: number[], values: Array<number | null>, cap: number): Segment[] {
  const keep: number[] = [];
  if (values.length <= cap) keep.push(...values.keys());
  else {
    const buckets = Math.max(1, Math.floor(cap / 2));
    for (let bucket = 0; bucket < buckets; bucket++) {
      const from = Math.floor(bucket * values.length / buckets);
      const to = Math.floor((bucket + 1) * values.length / buckets);
      let low = -1, high = -1;
      const picked = new Set<number>();
      for (let index = from; index < to; index++) {
        const value = values[index];
        if (value === null) { picked.add(index); continue; }
        if (low < 0 || value < values[low]!) low = index;
        if (high < 0 || value > values[high]!) high = index;
      }
      if (low >= 0) picked.add(low).add(high);
      keep.push(...[...picked].sort((a, b) => a - b));
    }
  }
  const segments: Segment[] = [];
  let current: Segment = [];
  for (const index of keep) {
    const value = values[index];
    if (value === null) {
      if (current.length) segments.push(current);
      current = [];
    } else current.push([times[index], value]);
  }
  if (current.length) segments.push(current);
  return segments;
}

/**
 * Rounded figures (axes, legend, plot labels) stay plain from 1e-4 up to 1e9
 * and switch to locale-aware scientific notation outside it: a plain 1e308 is
 * 309 digits, far past the axis's label margin. Exact figures (table,
 * threshold list) are never abbreviated; their cells scroll instead.
 */
const SCIENTIFIC_ABOVE = 1e9;
const SCIENTIFIC_BELOW = 1e-4;
function number(value: number, digits = 4, exact = false): string {
  // -0 (e.g. -0 * 1e-7) is zero; Intl would print it as "-0".
  if (value === 0) value = 0;
  const size = Math.abs(value);
  const scientific = !exact && size !== 0 && (size >= SCIENTIFIC_ABOVE || size < SCIENTIFIC_BELOW);
  return new Intl.NumberFormat(undefined, { maximumSignificantDigits: digits, ...(scientific ? { notation: "scientific" as const } : {}) }).format(value);
}

/** The fewest significant digits that read back as exactly this number (at most 17, enough for any double). */
function shortest(value: number): number {
  for (let digits = 1; digits < 17; digits++) if (Number(value.toPrecision(digits)) === value) return digits;
  return 17;
}

function scaled(value: number, base: number, names: string[]): [number, string] {
  let index = 0;
  while (index < names.length - 1 && Math.abs(value) >= base ** (index + 1)) index++;
  return [value / base ** index, names[index]];
}
const BYTES = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

/**
 * A value in its unit, formatted by the host. Apps never supply a format string.
 *
 * Axes and summaries round to four significant digits. `exact` is for the
 * values table and threshold text, where a rounded figure would contradict the
 * breach marker beside it (0.50004 printed as "0.5 · above 0.5"): it prints the
 * sample's own digits. Scaling that only moves the decimal point (%, ms, µs)
 * keeps the sample's digit count; byte prefixes divide by a power of two, which
 * is exact, so the quotient's own shortest form is used; minutes and hours
 * would round, so exact seconds stay in seconds.
 */
export function formatValue(value: number, unit: NativeTimeseriesUnit, exact = false): string {
  const digits = exact ? shortest(value) : 4;
  switch (unit) {
    case "number": return number(value, digits, exact);
    case "percent": return `${number(value, digits, exact)}%`;
    case "ratio": return `${number(value * 100, digits, exact)}%`;
    case "bytes":
    case "bytesPerSecond": {
      const [amount, name] = scaled(value, 1024, BYTES);
      return `${number(amount, exact ? shortest(amount) : 4, exact)} ${name}${unit === "bytesPerSecond" ? "/s" : ""}`;
    }
    case "seconds": {
      const size = Math.abs(value);
      if (size === 0 || (size >= 1 && (exact || size < 60))) return `${number(value, digits, exact)} s`;
      if (size < 0.001) return `${number(value * 1e6, digits, exact)} µs`;
      if (size < 1) return `${number(value * 1000, digits, exact)} ms`;
      if (size < 3600) return `${number(value / 60)} min`;
      return `${number(value / 3600)} h`;
    }
    case "cores": return `${number(value, digits, exact)} ${value === 1 ? "core" : "cores"}`;
    case "perSecond": return `${number(value, digits, exact)}/s`;
  }
}

/** What the value axis is measured in, for its title; empty for a plain number. */
export function unitName(unit: NativeTimeseriesUnit): string {
  return { number: "", percent: "%", ratio: "%", bytes: "bytes", bytesPerSecond: "bytes/s",
    seconds: "seconds", cores: "cores", perSecond: "per second" }[unit];
}

/** Round ticks covering `[lo, hi]`, in binary multiples for byte units. */
export function valueTicks(lo: number, hi: number, unit: NativeTimeseriesUnit): number[] {
  const factor = unit === "bytes" || unit === "bytesPerSecond"
    ? 1024 ** BYTES.indexOf(scaled(Math.max(Math.abs(lo), Math.abs(hi)), 1024, BYTES)[1])
    : 1;
  // Quartered before subtracting: `hi - lo` overflows to Infinity for finite samples near ±1.8e308.
  const raw = hi / factor / 4 - lo / factor / 4;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const normalized = raw / magnitude;
  const step = (normalized < 1.5 ? 1 : normalized < 3 ? 2 : normalized < 7 ? 5 : 10) * magnitude;
  const first = Math.floor(lo / factor / step), last = Math.ceil(hi / factor / step);
  const ticks: number[] = [];
  // Rounded to the step's precision so 0.1 + 0.2 does not print as 0.30000000000000004.
  for (let index = first; index <= last; index++) ticks.push(Number((index * step).toPrecision(12)) * factor);
  // Rounding out past the largest double leaves no round tick to end on: span the samples themselves.
  return ticks.length >= 2 && ticks.every(Number.isFinite) ? ticks : [lo, hi];
}

/**
 * The value axis's ticks; the first and last are the range the plot spans.
 * That range covers every sample and every threshold and always includes zero
 * (the Sparkline's rule — a series of 90, 95 reads as high and steady, not as
 * a climb), and is never zero-width. Computed once: re-deriving ticks from the
 * rounded range can pick a coarser step whose outer ticks fall off the plot.
 */
export function yTicks(series: Array<Array<number | null>>, thresholds: NativeTimeseriesThreshold[], unit: NativeTimeseriesUnit): number[] {
  let lo = Infinity, hi = -Infinity;
  for (const values of series) for (const value of values) if (value !== null) { lo = Math.min(lo, value); hi = Math.max(hi, value); }
  for (const threshold of thresholds) { lo = Math.min(lo, threshold.value); hi = Math.max(hi, threshold.value); }
  if (!Number.isFinite(lo)) { lo = 0; hi = 1; }
  lo = Math.min(lo, 0);
  hi = Math.max(hi, 0);
  if (lo === hi) hi = lo + 1;
  return valueTicks(lo, hi, unit);
}

/** Whether one sample is strictly beyond a threshold, in its direction. */
export function breached(value: number | null, threshold: NativeTimeseriesThreshold): boolean {
  return value !== null && (threshold.direction === "above" ? value > threshold.value : value < threshold.value);
}

/** How many of a series' samples are strictly beyond a threshold. */
export function breaches(values: Array<number | null>, threshold: NativeTimeseriesThreshold): number {
  return values.filter(value => breached(value, threshold)).length;
}

const SECOND = 1000, MINUTE = 60 * SECOND, HOUR = 60 * MINUTE, DAY = 24 * HOUR;

/** Nominal lengths for choosing a calendar step; the ticks themselves walk real months and years. */
export const MONTH = 30 * DAY, YEAR = 365 * DAY;

/**
 * The time axis's step ladder, in milliseconds: 1, 2, 5 … 500 ms; 1, 2, 5, 10,
 * 15, 30 s; 1, 2, 5, 10, 15, 30 min; 1, 2, 3, 6, 12 h; 1, 2, 7, 14 days;
 * 1, 2, 3, 6 months (on the 1st); then 1, 2 and 5 × 10ⁿ years (on 1 January),
 * as far as a valid range can reach.
 */
export const TIME_STEPS: readonly number[] = [
  1, 2, 5, 10, 20, 50, 100, 200, 500,
  ...[1, 2, 5, 10, 15, 30].map(step => step * SECOND),
  ...[1, 2, 5, 10, 15, 30].map(step => step * MINUTE),
  ...[1, 2, 3, 6, 12].map(step => step * HOUR),
  ...[1, 2, 7, 14].map(step => step * DAY),
  ...[1, 2, 3, 6].map(step => step * MONTH),
  ...[1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000, 50_000, 100_000, 200_000, 500_000].map(step => step * YEAR),
];

/** A local calendar day's number, stable across DST: the UTC day of its date components. */
const dayNumber = (date: Date) => Math.round(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / DAY);

/**
 * Every multiple of `step` in `[start, end]`, aligned in the reader's local
 * time so labels read 00:00, 06:00, 12:15 rather than wherever the range began.
 * Sub-hour steps divide an hour, so they stay aligned across a DST change
 * (which moves the clock by 30 or 60 minutes). Hour and day steps walk local
 * wall-clock dates; an hour a spring-forward skips resolves to the instant
 * that exists, and duplicates collapse.
 */
export function stepTicks(start: number, end: number, step: number): number[] {
  const ticks = new Set<number>();
  // `at(n)` is the n-th aligned candidate; stop past the end, past the last valid
  // date (NaN), or after a bounded number of candidates.
  const walk = (from: number, at: (n: number) => number) => {
    for (let n = from, tries = 0; tries < 200; n++, tries++) {
      const tick = at(n);
      if (!Number.isFinite(tick) || tick > end) break;
      if (tick >= start) ticks.add(tick);
    }
  };
  const first = new Date(start);
  if (step < HOUR) {
    const offset = -first.getTimezoneOffset() * MINUTE;
    const base = Math.ceil((start + offset) / step) * step - offset;
    walk(0, n => base + n * step);
  } else if (step < DAY) {
    const hours = step / HOUR;
    const from = Math.floor(first.getHours() / hours);
    walk(from, n => new Date(first.getFullYear(), first.getMonth(), first.getDate(), n * hours).getTime());
  } else if (step < MONTH) {
    const days = step / DAY;
    const from = Math.floor(dayNumber(first) / days) * days - dayNumber(first);
    walk(0, n => new Date(first.getFullYear(), first.getMonth(), first.getDate() + from + n * days).getTime());
  } else if (step < YEAR) {
    const months = step / MONTH;
    const from = Math.floor(first.getMonth() / months);
    walk(from, n => new Date(first.getFullYear(), n * months, 1).getTime());
  } else {
    const years = step / YEAR;
    const from = Math.floor(first.getFullYear() / years);
    // setFullYear, not the constructor: `new Date(y, …)` maps years 0–99 to 1900–1999.
    walk(from, n => { const date = new Date(0); date.setFullYear(n * years, 0, 1); date.setHours(0, 0, 0, 0); return date.getTime(); });
  }
  return [...ticks].sort((a, b) => a - b);
}

/**
 * The time axis: the smallest step on the ladder whose ticks fit, where
 * `fits(step)` says how many labels of that step's precision fit the plot.
 * If that leaves fewer than two ticks, the next finer step is used instead,
 * thinned to as many ticks as fit (evenly, keeping its first and last), so
 * every tick is still a multiple of that step.
 */
export function timeTicks(start: number, end: number, fits: (step: number) => number): { step: number; ticks: number[] } {
  const span = end - start;
  const index = TIME_STEPS.findIndex(step => Math.floor(span / step) + 1 <= Math.max(2, fits(step)));
  const chosen = index < 0 ? TIME_STEPS.length - 1 : index;
  const ticks = stepTicks(start, end, TIME_STEPS[chosen]);
  if (ticks.length >= 2 || chosen === 0) return { step: TIME_STEPS[chosen], ticks };
  const finer = TIME_STEPS[chosen - 1];
  const all = stepTicks(start, end, finer);
  const room = Math.max(2, fits(finer));
  if (all.length <= room) return { step: finer, ticks: all };
  return { step: finer, ticks: Array.from({ length: room }, (_, n) => all[Math.round(n * (all.length - 1) / (room - 1))]) };
}
