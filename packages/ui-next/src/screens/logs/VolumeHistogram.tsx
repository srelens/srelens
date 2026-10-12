import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import {
  clauseState,
  inRange,
  volumeBuckets,
  type QueryClause,
  type QueryLine,
  type TimeRange,
  type VolumeBucket,
} from "@srelens/core";
import { statusTone, toneColor, toneWash, type StatusKind } from "@srelens/ui-kit";
import { groupNumber } from "../../lib/numbers";
import { clockTime, dateTime, DAY, niceCeil, spansDays, stepLabel, tickLabels, timeTicks } from "./time";

const BAR = 10;
const GAP = 2;
const HEIGHT = 96;
/** The least a non-empty segment is drawn, in viewBox units (about a pixel): a lone error beside a spike must not vanish. */
const MIN_SEGMENT = 1;
const MINUTE = 60_000;

/**
 * A bar's segments, bottom to top, and the legend's entries in the same order.
 * Errors sit on the baseline, so a spike of them compares across bars; what has
 * no severity goes last and is drawn faint. The colour is core's health tone
 * for `kind`, never a local one; `word` is only what the legend calls it.
 */
const GROUPS = [
  { key: "danger", kind: "danger", word: "error" },
  { key: "warning", kind: "warning", word: "warn" },
  { key: "info", kind: "info", word: "info" },
  { key: "other", kind: "neutral", word: "other" },
] as const satisfies readonly { key: keyof VolumeBucket; kind: StatusKind; word: string }[];
/** How much of its colour the unlevelled lines keep. */
const OTHER_OPACITY = 0.4;

/**
 * Per group, in `GROUPS` order: how many lines, and the level words they carry,
 * which is what a click on the entry puts in the query. Core decides which
 * group a line is in (`health`) and what its word is (`level`); a line whose
 * health is none of the three coloured ones, `success` included, is "other".
 */
function tally(lines: readonly QueryLine[]): { count: number; levels: string[] }[] {
  const groups = GROUPS.map(() => ({ count: 0, levels: new Set<string>() }));
  for (const line of lines) {
    const found = GROUPS.findIndex(({ kind }) => kind === line.health);
    const group = groups[found === -1 ? GROUPS.length - 1 : found];
    group.count += 1;
    if (line.level !== "") group.levels.add(line.level.toLowerCase());
  }
  // Sorted, so the query a click writes does not depend on which line arrived first.
  return groups.map(({ count, levels }) => ({ count, levels: [...levels].sort() }));
}

function stack(b: VolumeBucket, max: number): number[] {
  const heights = GROUPS.map(({ key }) => (b[key] > 0 ? Math.max(MIN_SEGMENT, (b[key] / max) * HEIGHT) : 0));
  // A segment lifted to the floor can push the stack past the top: the tallest gives up the difference.
  const over = heights.reduce((a, h) => a + h, 0) - HEIGHT;
  if (over > 0) heights[heights.indexOf(Math.max(...heights))] -= over;
  return heights;
}

/**
 * The stream's volume over time, stacked by core's severity (#898). A click
 * on a bar, or a drag across several, picks the window the body narrows to.
 *
 * It counts the lines the QUERY keeps but ignores the window itself, which it
 * shades rather than crops: the context around a spike is the point of
 * looking at one. The bars are the mouse's way in. The keyboard gets one tab
 * stop, arrows to move, Enter to pick and Escape to clear, rather than sixty
 * tab stops; a polite live region says which bar the arrows are on.
 */
export function VolumeHistogram({
  lines,
  range,
  onRangeChange,
  clauses,
  onToggleLevels,
}: {
  lines: readonly QueryLine[];
  range: TimeRange | undefined;
  onRangeChange: (range: TimeRange | undefined) => void;
  /** The query as typed, for which legend entries are on. */
  clauses: readonly QueryClause[];
  /** A legend click: put these `level:` words in the query, or take them out. */
  onToggleLevels: (words: string[], include: boolean) => void;
}) {
  const volume = useMemo(() => volumeBuckets(lines), [lines]);
  const { buckets } = volume;
  const timed = lines.length - volume.untimed;
  // The axis tops out at a round number above the tallest bar, so its labels read as counts.
  const axisMax = useMemo(
    () => niceCeil(Math.max(1, ...buckets.map((b) => b.danger + b.warning + b.info + b.other))),
    [buckets],
  );
  const groups = useMemo(() => tally(lines), [lines]);
  // Timed lines minus those inside, so a line whose time can't be placed (it is
  // in `untimed`) is never also counted as outside. Memoised: a pointer move or
  // an arrow re-renders this, and the pass is over every line.
  const outside = useMemo(
    () => (range === undefined ? 0 : timed - lines.reduce((n, l) => (inRange(l, range) ? n + 1 : n), 0)),
    [lines, range, timed],
  );
  const [drag, setDrag] = useState<{ from: number; to: number } | null>(null);
  const [focus, setFocus] = useState<number | null>(null);
  const dragging = drag !== null;
  // A drag that loses the window (alt-tab, a context menu) never sees its pointerup.
  useEffect(() => {
    if (!dragging) return;
    const drop = () => setDrag(null);
    window.addEventListener("blur", drop);
    return () => window.removeEventListener("blur", drop);
  }, [dragging]);
  if (buckets.length === 0) return null;

  const focused = focus === null ? null : Math.min(focus, buckets.length - 1);
  // Clamped: a live stream can re-bucket between the press and the release.
  const bucket = (i: number) => buckets[Math.min(i, buckets.length - 1)];
  const select = (a: number, b: number) =>
    onRangeChange({ from: bucket(Math.min(a, b)).start, to: bucket(Math.max(a, b)).end });
  // A clock reading is ambiguous across days, so the edges name their date then,
  // and their seconds only when a bar is narrower than a minute.
  const dated = volume.step >= DAY || spansDays(buckets[0].start, buckets[buckets.length - 1].end);
  const edge = (ms: number) => (dated ? dateTime(ms, volume.step < MINUTE) : clockTime(ms));
  const titleOf = (b: VolumeBucket) => {
    const total = b.danger + b.warning + b.info + b.other;
    return `${edge(b.start)}–${edge(b.end)} · ${groupNumber(total)} ${total === 1 ? "line" : "lines"}${
      b.danger > 0 ? ` · ${groupNumber(b.danger)} ${b.danger === 1 ? "error" : "errors"}` : ""
    }`;
  };
  const from = buckets[0].start;
  const to = buckets[buckets.length - 1].end;
  const ticks = timeTicks(from, to);
  const tickText = tickLabels(ticks);
  const caption = [
    `${groupNumber(timed)} ${timed === 1 ? "line" : "lines"}`,
    `${edge(from)}–${edge(to)}`,
    `${stepLabel(volume.step)} bars`,
    ...(volume.untimed > 0 ? [`${groupNumber(volume.untimed)} without a time`] : []),
    ...(outside > 0 ? [`${groupNumber(outside)} outside the window`] : []),
  ].join(" · ");

  function onKeyDown(e: KeyboardEvent) {
    // Alt+Arrow is history, Ctrl+Arrow a word jump: not ours to take.
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    if (e.key === "ArrowLeft") setFocus(focused === null ? buckets.length - 1 : Math.max(0, focused - 1));
    else if (e.key === "ArrowRight") setFocus(focused === null ? 0 : Math.min(buckets.length - 1, focused + 1));
    else if (e.key === "Enter" && focused !== null) select(focused, focused);
    else if (e.key === "Escape" && range !== undefined) onRangeChange(undefined);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function finishDrag() {
    if (drag === null) return;
    select(drag.from, drag.to);
    setDrag(null);
  }

  return (
    <div data-slot="volume" className="rule-b shrink-0 px-3 py-1.5">
      <div className="flex gap-1">
        {/* HTML, not SVG text: the viewBox is stretched to the pane, and would stretch the digits with it. */}
        <div aria-hidden="true" className="path relative w-9 shrink-0 text-right" style={{ height: HEIGHT }}>
          {[axisMax, axisMax / 2].map((n, i) => (
            <span
              key={i}
              data-slot="y-label"
              className="absolute right-1 -translate-y-1/2 text-[0.6875rem] leading-none"
              style={{ top: i === 0 ? 0 : "50%" }}
            >
              {groupNumber(n)}
            </span>
          ))}
        </div>
        <div className="min-w-0 flex-1">
          <div
            role="group"
            tabIndex={0}
            aria-label={`Log volume over time: ${caption}. Arrow keys pick a bar, Enter narrows to it, Escape clears.`}
            onKeyDown={onKeyDown}
            onBlur={() => setFocus(null)}
            className="volume-bars"
          >
            <svg
              viewBox={`0 0 ${buckets.length * (BAR + GAP)} ${HEIGHT}`}
              preserveAspectRatio="none"
              className="block w-full select-none"
              // Visible, so a ring or a gridline on the edge of the viewBox is not cut in half.
              style={{ height: HEIGHT, overflow: "visible" }}
              onPointerUp={finishDrag}
              onPointerLeave={finishDrag}
              onPointerCancel={() => setDrag(null)}
            >
              {[HEIGHT, HEIGHT / 2, 0].map((y) => (
                <line
                  key={y}
                  data-slot="gridline"
                  x1={0}
                  x2={buckets.length * (BAR + GAP)}
                  y1={y}
                  y2={y}
                  stroke="var(--rule)"
                  strokeWidth={1}
                  vectorEffect="non-scaling-stroke"
                />
              ))}
              {/* Behind the bars: the wash tints the slot, the segments stay their own colour. */}
              {focused !== null && (
                <rect
                  data-slot="bucket-focus-wash"
                  x={focused * (BAR + GAP)}
                  y={0}
                  width={BAR}
                  height={HEIGHT}
                  fill={toneWash("accent")}
                  pointerEvents="none"
                />
              )}
              {buckets.map((b, i) => {
                const x = i * (BAR + GAP);
                const lit =
                  drag !== null
                    ? i >= Math.min(drag.from, drag.to) && i <= Math.max(drag.from, drag.to)
                    : range === undefined || (b.start < range.to && b.end > range.from);
                const heights = stack(b, axisMax);
                let y = HEIGHT;
                return (
                  <g
                    // The slot, not the time: a skipped hour's bar has no width and
                    // starts where the next one does, so `start` repeats.
                    key={i}
                    data-slot="bucket"
                    opacity={lit ? 1 : 0.35}
                    // Primary button only: a right-click is the context menu's, and
                    // its pointerup may never come.
                    onPointerDown={(e) => {
                      if (e.button === 0) setDrag({ from: i, to: i });
                    }}
                    onPointerEnter={() => setDrag((d) => (d === null ? d : { ...d, to: i }))}
                  >
                    <title>{titleOf(b)}</title>
                    <rect x={x} y={0} width={BAR} height={HEIGHT} fill="transparent" />
                    {GROUPS.map(({ key, kind }, g) => {
                      y -= heights[g];
                      return heights[g] > 0 ? (
                        <rect
                          key={key}
                          data-slot="segment"
                          data-group={key}
                          x={x}
                          y={y}
                          width={BAR}
                          height={heights[g]}
                          fill={toneColor(statusTone(kind))}
                          fillOpacity={key === "other" ? OTHER_OPACITY : undefined}
                        />
                      ) : null;
                    })}
                  </g>
                );
              })}
              {/* Outside the bars' groups: a dimmed bar would dim its own ring. The
                  stroke is a constant width, as the viewBox is stretched to the pane. */}
              {focused !== null && (
                <rect
                  data-slot="bucket-focus"
                  x={focused * (BAR + GAP)}
                  y={0}
                  width={BAR}
                  height={HEIGHT}
                  fill="none"
                  stroke={toneColor("accent")}
                  strokeWidth={2}
                  vectorEffect="non-scaling-stroke"
                  pointerEvents="none"
                />
              )}
            </svg>
          </div>
          <div aria-hidden="true" className="path relative h-4">
            {ticks.map((t, i) => {
              const at = ((t - from) * 100) / (to - from);
              return (
                <span
                  key={t}
                  data-slot="x-tick"
                  className="absolute top-0.5 whitespace-nowrap text-[0.6875rem] leading-none"
                  // Centred on its tick, but kept inside the plot at either end.
                  style={{ left: `${at}%`, transform: `translateX(${at < 5 ? "0" : at > 95 ? "-100%" : "-50%"})` }}
                >
                  {tickText[i]}
                </span>
              );
            })}
          </div>
        </div>
      </div>
      <div data-slot="volume-legend" className="mt-0.5 flex flex-wrap items-center gap-x-1 gap-y-0.5">
        {GROUPS.map(({ key, kind, word }, g) => {
          const { count, levels } = groups[g];
          if (count === 0) return null;
          const dot = (
            <svg data-slot="legend-dot" aria-hidden="true" width={8} height={8} viewBox="0 0 8 8">
              <circle
                cx={4}
                cy={4}
                r={4}
                fill={toneColor(statusTone(kind))}
                fillOpacity={key === "other" ? OTHER_OPACITY : undefined}
              />
            </svg>
          );
          const label = (
            <>
              {dot}
              {word} <span className="tabular-nums text-faint">{groupNumber(count)}</span>
            </>
          );
          const cls = "inline-flex items-center gap-1.5 rounded px-1.5 py-0.5 text-[0.6875rem] text-muted";
          if (key === "other") {
            // A line with no level can't be named in the query, so there is nothing to click.
            return (
              <span key={key} data-slot="legend-item" className={cls}>
                {label}
              </span>
            );
          }
          const pressed =
            levels.length > 0 && levels.every((w) => clauseState(clauses, { field: "level", value: w }) === "include");
          return (
            <button
              key={key}
              type="button"
              data-slot="legend-item"
              aria-pressed={pressed}
              onClick={() => onToggleLevels(levels, !pressed)}
              className={`${cls} hover:bg-[var(--field)] aria-pressed:bg-[var(--field)] aria-pressed:text-ink`}
            >
              {label}
            </button>
          );
        })}
      </div>
      <div aria-live="polite" aria-atomic="true" data-slot="volume-live" className="sr-only">
        {focused === null ? "" : titleOf(buckets[focused])}
      </div>
      <div data-slot="volume-caption" className="path mt-0.5 text-faint">
        {caption}
      </div>
    </div>
  );
}
