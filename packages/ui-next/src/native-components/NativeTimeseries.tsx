import { useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Badge, Button } from "@srelens/ui-kit";
import type { NativeTimeseriesData, NativeTimeseriesThreshold } from "@srelens/core/lib/nativeComponents";
import { NativeRows } from "./NativeRows";
import { breached, breaches, downsample, formatValue, MAX_DRAWN_POINTS, MONTH, stepTicks, timeTicks, unitName, YEAR, yTicks, type Segment } from "./timeseries";

/*
 * Host-owned series styling. Colour comes from the theme's mark palette (each
 * token has a light and a dark value); every series also gets its own dash
 * pattern, repeated in the legend swatch, so no series is told apart by colour
 * alone. Red is left out: it reads as a critical threshold.
 */
const COLORS = ["--mark-blue", "--mark-orange", "--mark-teal", "--mark-purple", "--mark-pink", "--mark-green", "--mark-amber", "--mark-slate"];
const DASHES = ["none", "6 3", "2 2", "8 3 2 3", "12 3", "3 3 1 3", "1 3", "10 2 2 2 2 2"];
/*
 * An isolated sample (a point between gaps) has no line to carry its dash
 * pattern, so each series also has a marker shape. It is drawn at such points
 * and in the legend swatch. Markers are outlines, never filled: two series
 * with a sample at the same time and value draw different shapes at one
 * point, and an unfilled shape cannot paint over the one beneath it.
 */
const MARKERS = ["circle", "square", "triangle", "diamond", "triangleDown", "plus", "cross", "asterisk"] as const;
type Marker = typeof MARKERS[number];
function markerPath(shape: Marker, x: number, y: number, r = 3.5): string {
  const at = (dx: number, dy: number) => `${(x + dx).toFixed(1)},${(y + dy).toFixed(1)}`;
  switch (shape) {
    case "circle": return `M${at(-r, 0)} A${r},${r} 0 1,0 ${at(r, 0)} A${r},${r} 0 1,0 ${at(-r, 0)} Z`;
    case "square": return `M${at(-r, -r)} L${at(r, -r)} L${at(r, r)} L${at(-r, r)} Z`;
    case "triangle": return `M${at(0, -r)} L${at(r, r)} L${at(-r, r)} Z`;
    case "diamond": return `M${at(0, -r)} L${at(r, 0)} L${at(0, r)} L${at(-r, 0)} Z`;
    case "triangleDown": return `M${at(0, r)} L${at(r, -r)} L${at(-r, -r)} Z`;
    case "plus": return `M${at(-r, 0)} L${at(r, 0)} M${at(0, -r)} L${at(0, r)}`;
    case "cross": return `M${at(-r, -r)} L${at(r, r)} M${at(-r, r)} L${at(r, -r)}`;
    case "asterisk": return `M${at(-r, 0)} L${at(r, 0)} M${at(0, -r)} L${at(0, r)} M${at(-r * 0.7, -r * 0.7)} L${at(r * 0.7, r * 0.7)} M${at(-r * 0.7, r * 0.7)} L${at(r * 0.7, -r * 0.7)}`;
  }
}
function MarkerShape({ series, index, x, y }: { series: string; index: number; x: number; y: number }) {
  const shape = MARKERS[index];
  return <path data-series={series} data-marker={shape} d={markerPath(shape, x, y)} fill="none" strokeWidth="1.5"
    strokeLinejoin="round" style={{ stroke: `var(${COLORS[index]})` }}/>;
}
const THRESHOLD_DASH = "4 3";
const TONE = { info: "--info", warn: "--warn", sev: "--sev" } as const;
const TONE_WORD = { info: "info", warn: "warning", sev: "critical" } as const;

const HEIGHT = 180;
const MARGIN = { right: 12, top: 10, bottom: 22 };
/** Tick labels are 11px monospace; about 7px a character, plus a gap before the plot. */
const labelWidth = (labels: string[]) => Math.min(120, 12 + 7 * Math.max(...labels.map(label => label.length)));

/** The region's width; jsdom and the first paint get a sensible default. */
function useWidth(initial: number) {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(initial);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => { if (element.clientWidth > 0) setWidth(element.clientWidth); };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

/**
 * The reader's UTC offset, but only when the times shown do not all share one.
 * Local time repeats an hour when clocks fall back (01:30 EDT, then 01:30 EST);
 * the offset is what tells those apart, and it is noise everywhere else.
 */
function offsetIfAmbiguous(times: number[]) {
  return new Set(times.map(time => new Date(time).getTimezoneOffset())).size > 1 ? { timeZoneName: "shortOffset" as const } : {};
}

/**
 * The range in the plot's text summary: its exact ends. Ranges up to five
 * minutes show seconds, and milliseconds too when either end is off a whole
 * second, so a 1 ms range's ends never read the same.
 */
function rangeFormat(span: number, times: number[]) {
  const seconds = span <= 5 * 60_000 ? { second: "2-digit" as const,
    ...(times.some(time => time % 1000 !== 0) ? { fractionalSecondDigits: 3 as const } : {}) } : {};
  return new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", ...seconds, ...offsetIfAmbiguous(times) });
}

const MINUTE = 60_000, HOUR = 60 * MINUTE, DAY = 24 * HOUR;

/**
 * Axis label precision follows the step, and every label is its tick's own
 * instant: milliseconds for sub-second steps, seconds for second steps, hours
 * and minutes for minute and hour steps (a local midnight shows its date
 * instead, so a multi-day axis says which day), the date for day steps (with
 * the year once the ticks span more than one), month and year for month
 * steps, and the year for year steps. Every one of those formats adds the UTC
 * offset when the ticks span more than one, the same rule the table and the
 * range summary follow.
 */
function tickLabels(step: number, ticks: number[]): (time: number) => string {
  const offset = offsetIfAmbiguous(ticks);
  if (step >= YEAR) { const year = new Intl.DateTimeFormat(undefined, { year: "numeric", ...offset }); return time => year.format(time); }
  if (step >= MONTH) { const month = new Intl.DateTimeFormat(undefined, { month: "short", year: "numeric", ...offset }); return time => month.format(time); }
  const years = new Set(ticks.map(tick => new Date(tick).getFullYear())).size > 1 ? { year: "numeric" as const } : {};
  const date = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", ...years, ...offset });
  if (step >= DAY) return time => date.format(time);
  const time = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit",
    ...(step < MINUTE ? { second: "2-digit" as const } : {}), ...(step < 1000 ? { fractionalSecondDigits: 3 as const } : {}), ...offset });
  if (step < HOUR) return value => time.format(value);
  return value => {
    const local = new Date(value);
    return local.getHours() === 0 && local.getMinutes() === 0 ? date.format(value) : time.format(value);
  };
}

/**
 * How many labels of a step's precision the axis takes: as many as fit (11px
 * monospace is about 7px a character, plus a gap), but no more than six on a
 * wide plot and four in a narrow one: round steps land near, not exactly on,
 * the five and three evenly spaced labels the axis had before.
 */
function labelsThatFit(width: number, plotWidth: number, start: number, end: number) {
  const most = width >= 640 ? 6 : 4;
  return (step: number) => {
    // Far too many ticks at this step whatever their labels: no need to format them. The fallback in
    // timeTicks asks about a finer step with a few more ticks than fit, so those are still measured.
    if (Math.floor((end - start) / step) + 1 > 4 * most) return most;
    // Measure the labels this step would really draw, with any offset or year they carry.
    const ticks = stepTicks(start, end, step);
    const label = tickLabels(step, ticks);
    const widest = Math.max(1, ...ticks.map(tick => label(tick).length));
    return Math.max(2, Math.min(most, Math.floor(plotWidth / (widest * 7 + 16))));
  };
}

/**
 * A table row's time: every sample to the second, whatever the range, to the
 * millisecond when any sample carries one, and with its UTC offset when the
 * samples span more than one, so no two rows read the same.
 */
function sampleFormat(times: number[]) {
  const milliseconds = times.some(time => time % 1000 !== 0);
  return new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit", second: "2-digit", ...(milliseconds ? { fractionalSecondDigits: 3 as const } : {}), ...offsetIfAmbiguous(times) });
}

const segmentPath = (segment: Segment, x: (time: number) => number, y: (value: number) => number) =>
  segment.map(([time, value], index) => `${index ? "L" : "M"}${x(time).toFixed(1)},${y(value).toFixed(1)}`).join(" ");

/** A series' line pattern with its marker on it, exactly as the plot draws both. */
function Swatch({ series, index }: { series: string; index: number }) {
  const dash = DASHES[index];
  return <svg className="native-timeseries-swatch" width="24" height="10" aria-hidden="true">
    <line x1="0" y1="5" x2="24" y2="5" style={{ stroke: `var(${COLORS[index]})` }} strokeWidth="2" strokeDasharray={dash === "none" ? undefined : dash}/>
    <MarkerShape series={series} index={index} x={12} y={5}/>
  </svg>;
}

function breachText(threshold: NativeTimeseriesThreshold, count: number, total: number) {
  return `${threshold.direction === "above" ? "Above" : "Below"} ${threshold.label}: ${count} of ${total} samples`;
}

/** The drawing. Its text equivalent is the legend, threshold list and values table. */
function Plot({ data, width, summary }: { data: NativeTimeseriesData; width: number; summary: string }) {
  const { range, unit, thresholds = [] } = data;
  const ticks = yTicks(data.series.map(entry => entry.values), thresholds, unit);
  const lo = ticks[0], hi = ticks[ticks.length - 1];
  const labels = ticks.map(tick => formatValue(tick, unit));
  const left = labelWidth(labels);
  const plotWidth = Math.max(width - left - MARGIN.right, 40);
  const plotHeight = HEIGHT - MARGIN.top - MARGIN.bottom;
  const x = (time: number) => left + (time - range.start) / (range.end - range.start) * plotWidth;
  // The span overflows to Infinity for samples near ±1.8e308; only then halve before subtracting.
  // Halving always would underflow a subnormal span (0 to Number.MIN_VALUE) to 0 / 0.
  const valueSpan = hi - lo;
  const y = (value: number) => MARGIN.top + plotHeight * (Number.isFinite(valueSpan)
    ? (hi - value) / valueSpan
    : (hi / 2 - value / 2) / (hi / 2 - lo / 2));
  const span = range.end - range.start;
  // Whole milliseconds, the finest a label shows; a range of a few ms yields fewer, distinct ticks.
  const { step, ticks: axisTimes } = timeTicks(range.start, range.end, labelsThatFit(width, plotWidth, range.start, range.end));
  const label = tickLabels(step, axisTimes);
  const timeLabels = axisTimes.map(label);
  // Centred on its tick, unless that would push the label past the plot's edge.
  const anchor = (time: number, text: string) => {
    const half = text.length * 3.5;
    return x(time) - half < 0 ? "start" : x(time) + half > width ? "end" : "middle";
  };
  const right = left + plotWidth;
  return <svg className="native-timeseries-plot" width={width} height={HEIGHT} role="img" aria-label={summary}>
    {ticks.map((tick, index) => <g key={tick}>
      <line x1={left} x2={right} y1={y(tick)} y2={y(tick)} className="native-timeseries-grid"/>
      <text x={left - 6} y={y(tick)} dy="0.32em" textAnchor="end" data-axis="value" className="native-timeseries-tick">{labels[index]}</text>
    </g>)}
    {axisTimes.map((time, index) => <g key={time}>
      <line x1={x(time)} x2={x(time)} y1={MARGIN.top + plotHeight} y2={MARGIN.top + plotHeight + 4} className="native-timeseries-grid"/>
      <text x={x(time)} y={HEIGHT - 6} data-axis="time" data-time={new Date(time).toISOString()} className="native-timeseries-tick"
        textAnchor={anchor(time, timeLabels[index])}>{timeLabels[index]}</text>
    </g>)}
    {thresholds.map(threshold => <g key={threshold.label}>
      <line x1={left} x2={right} y1={y(threshold.value)} y2={y(threshold.value)} strokeWidth="1"
        strokeDasharray={THRESHOLD_DASH} style={{ stroke: `var(${TONE[threshold.tone]})` }}/>
      <text x={right - 4} y={y(threshold.value) - 4} textAnchor="end" className="native-timeseries-threshold-label">
        {threshold.label} {formatValue(threshold.value, unit)}
      </text>
    </g>)}
    {data.series.map((entry, index) => {
      const dash = DASHES[index];
      return <g key={entry.name}>{downsample(data.times, entry.values, MAX_DRAWN_POINTS).map((segment, part) => segment.length === 1
        ? <MarkerShape key={part} series={entry.name} index={index} x={x(segment[0][0])} y={y(segment[0][1])}/>
        : <path key={part} data-series={entry.name} data-dash={dash} d={segmentPath(segment, x, y)} fill="none" strokeWidth="1.5"
          strokeLinejoin="round" strokeDasharray={dash === "none" ? undefined : dash} style={{ stroke: `var(${COLORS[index]})` }}/>)}</g>;
    })}
  </svg>;
}

function stats(values: Array<number | null>, unit: NativeTimeseriesData["unit"]): string {
  const real = values.filter((value): value is number => value !== null);
  if (!real.length) return "No samples in this range";
  let low = real[0], high = real[0];
  for (const value of real) { low = Math.min(low, value); high = Math.max(high, value); }
  return `latest ${formatValue(real[real.length - 1], unit)} · min ${formatValue(low, unit)} · max ${formatValue(high, unit)}`;
}

/** Each series by name, pattern and figures, with every threshold it crosses said in words. */
function Legend({ data }: { data: NativeTimeseriesData }) {
  const thresholds = data.thresholds ?? [];
  return <>
    <ul className="native-timeseries-legend" aria-label="Series">
      {data.series.map((entry, index) => {
        const total = entry.values.filter(value => value !== null).length;
        return <li key={entry.name}>
          <Swatch series={entry.name} index={index}/>
          <strong className="native-timeseries-name">{entry.name}</strong>
          <span className="native-timeseries-stats">{stats(entry.values, data.unit)}</span>
          {thresholds.map(threshold => {
            const count = breaches(entry.values, threshold);
            return count > 0 && <Badge key={threshold.label} tone={threshold.tone}>{breachText(threshold, count, total)}</Badge>;
          })}
        </li>;
      })}
    </ul>
    {thresholds.length > 0 && <ul className="native-timeseries-legend" aria-label="Thresholds">
      {thresholds.map(threshold => <li key={threshold.label}>
        <svg className="native-timeseries-swatch" width="24" height="8" aria-hidden="true">
          <line x1="0" y1="4" x2="24" y2="4" strokeWidth="1" strokeDasharray={THRESHOLD_DASH} style={{ stroke: `var(${TONE[threshold.tone]})` }}/>
        </svg>
        <span>{`${threshold.label}: ${threshold.direction} ${formatValue(threshold.value, data.unit, true)}`}</span>
        <span className="native-timeseries-stats">({TONE_WORD[threshold.tone]})</span>
      </li>)}
    </ul>}
  </>;
}

function cell(value: number | null, data: NativeTimeseriesData): string {
  if (value === null) return "No sample";
  const crossed = (data.thresholds ?? []).filter(threshold => breached(value, threshold))
    .map(threshold => ` · ${threshold.direction} ${threshold.label}`);
  return formatValue(value, data.unit, true) + crossed.join("");
}

/** Every sample, paged like every other catalog collection; the plot's text equivalent. */
function Values({ data, id }: { data: NativeTimeseriesData; id: string }) {
  const rows = useMemo(() => data.times.map((_, index) => index), [data.times]);
  const format = useMemo(() => sampleFormat(data.times), [data.times]);
  const unit = unitName(data.unit);
  return <div className="native-timeseries-values" id={id}>
    <NativeRows items={rows}>{visible => <table className="native-timeseries-table">
      <caption>{data.label} values{unit && `, in ${unit}`}</caption>
      <thead><tr><th scope="col">Time</th>{data.series.map(entry => <th scope="col" key={entry.name}>{entry.name}</th>)}</tr></thead>
      <tbody>{visible.map(index => <tr key={data.times[index]}>
        <th scope="row"><time dateTime={new Date(data.times[index]).toISOString()}>{format.format(data.times[index])}</time></th>
        {data.series.map(entry => <td key={entry.name}>{cell(entry.values[index], data)}</td>)}
      </tr>)}</tbody>
    </table>}</NativeRows>
  </div>;
}

/** A host-drawn chart of provider data: the app supplies samples, never drawing code. */
export function NativeTimeseries({ data }: { data: NativeTimeseriesData }) {
  const [ref, width] = useWidth(640);
  const [open, setOpen] = useState(false);
  const id = useId();
  const unit = unitName(data.unit);
  const title = `${data.label}${unit && ` (${unit})`}`;
  const format = rangeFormat(data.range.end - data.range.start, [data.range.start, data.range.end]);
  const thresholds = data.thresholds ?? [];
  const summary = `${title}: ${data.series.length} series from ${format.format(data.range.start)} to ${format.format(data.range.end)}.`
    + (thresholds.length ? ` Thresholds: ${thresholds.map(threshold => `${threshold.label} ${threshold.direction} ${formatValue(threshold.value, data.unit)}`).join(", ")}.` : "")
    + " Exact values are in the values table.";
  const reduced = data.times.length > MAX_DRAWN_POINTS;
  return <div className="native-timeseries">
    <div className="native-timeseries-title">{title}</div>
    <div className="native-timeseries-frame" ref={ref}><Plot data={data} width={width} summary={summary}/></div>
    <Legend data={data}/>
    {reduced && <p className="native-timeseries-note">
      Drawn from {data.times.length.toLocaleString()} samples per series, reduced to at most {MAX_DRAWN_POINTS} points each;
      the lowest and highest sample of every stretch is kept. The values table lists every sample.
    </p>}
    <div className="native-component-more">
      <Button type="button" variant="secondary" size="sm" aria-expanded={open} aria-controls={open ? id : undefined} onClick={() => setOpen(!open)}>
        {open ? "Hide values table" : `Show values table (${data.times.length.toLocaleString()} samples)`}
      </Button>
    </div>
    {open && <Values data={data} id={id}/>}
  </div>;
}
