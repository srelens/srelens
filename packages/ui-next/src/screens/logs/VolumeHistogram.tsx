import { useMemo, useState, type KeyboardEvent } from "react";
import { inRange, volumeBuckets, type QueryLine, type TimeRange } from "@srelens/core";
import { statusTone, toneColor, toneWash } from "@srelens/ui-kit";
import { groupNumber } from "../../lib/numbers";
import { clockTime, stepLabel } from "./time";

const BAR = 10;
const GAP = 2;
const HEIGHT = 40;

/**
 * The stream's volume over time, stacked by core's severity (#898). A click
 * on a bar, or a drag across several, picks the window the body narrows to.
 *
 * It counts the lines the QUERY keeps but ignores the window itself, which it
 * shades rather than crops: the context around a spike is the point of
 * looking at one. The bars are the mouse's way in. The keyboard gets one tab
 * stop, arrows to move, Enter to pick and Escape to clear, rather than sixty
 * tab stops.
 */
export function VolumeHistogram({
  lines,
  range,
  onRangeChange,
}: {
  lines: readonly QueryLine[];
  range: TimeRange | undefined;
  onRangeChange: (range: TimeRange | undefined) => void;
}) {
  const volume = useMemo(() => volumeBuckets(lines), [lines]);
  const [drag, setDrag] = useState<{ from: number; to: number } | null>(null);
  const [focus, setFocus] = useState<number | null>(null);
  const { buckets } = volume;
  if (buckets.length === 0) return null;

  const focused = focus === null ? null : Math.min(focus, buckets.length - 1);
  const timed = lines.length - volume.untimed;
  // Timed lines minus those inside, so a line whose time can't be placed (it is
  // in `untimed`) is never also counted as outside.
  const outside = range === undefined ? 0 : timed - lines.reduce((n, l) => (inRange(l, range) ? n + 1 : n), 0);
  const peak = Math.max(1, ...buckets.map((b) => b.danger + b.warning + b.other));
  // Clamped: a live stream can re-bucket between the press and the release.
  const bucket = (i: number) => buckets[Math.min(i, buckets.length - 1)];
  const select = (a: number, b: number) =>
    onRangeChange({ from: bucket(Math.min(a, b)).start, to: bucket(Math.max(a, b)).end });
  const caption = [
    `${groupNumber(timed)} ${timed === 1 ? "line" : "lines"}`,
    `${clockTime(buckets[0].start)}–${clockTime(buckets[buckets.length - 1].end)}`,
    `${stepLabel(volume.step)} bars`,
    ...(volume.untimed > 0 ? [`${groupNumber(volume.untimed)} without a time`] : []),
    ...(outside > 0 ? [`${groupNumber(outside)} outside the window`] : []),
  ].join(" · ");

  function onKeyDown(e: KeyboardEvent) {
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
      <div
        role="group"
        tabIndex={0}
        aria-label={`Log volume over time: ${caption}. Arrow keys pick a bar, Enter narrows to it, Escape clears.`}
        onKeyDown={onKeyDown}
        onBlur={() => setFocus(null)}
        className="outline-none"
      >
        <svg
          viewBox={`0 0 ${buckets.length * (BAR + GAP)} ${HEIGHT}`}
          preserveAspectRatio="none"
          className="block h-10 w-full select-none"
          onPointerUp={finishDrag}
          onPointerLeave={finishDrag}
        >
          {buckets.map((b, i) => {
            const x = i * (BAR + GAP);
            const total = b.danger + b.warning + b.other;
            const lit =
              drag !== null
                ? i >= Math.min(drag.from, drag.to) && i <= Math.max(drag.from, drag.to)
                : range === undefined || (b.start < range.to && b.end > range.from);
            let y = HEIGHT;
            const segment = (count: number, color: string) => {
              const h = (count / peak) * HEIGHT;
              y -= h;
              return count > 0 ? <rect x={x} y={y} width={BAR} height={h} fill={color} /> : null;
            };
            return (
              <g
                // The slot, not the time: a skipped hour's bar has no width and
                // starts where the next one does, so `start` repeats.
                key={i}
                data-slot="bucket"
                opacity={lit ? 1 : 0.35}
                onPointerDown={() => setDrag({ from: i, to: i })}
                onPointerEnter={() => setDrag((d) => (d === null ? d : { ...d, to: i }))}
              >
                <title>
                  {`${clockTime(b.start)}–${clockTime(b.end)} · ${groupNumber(total)} ${total === 1 ? "line" : "lines"}${
                    b.danger > 0 ? ` · ${groupNumber(b.danger)} ${b.danger === 1 ? "error" : "errors"}` : ""
                  }`}
                </title>
                <rect
                  x={x}
                  y={0}
                  width={BAR}
                  height={HEIGHT}
                  fill={i === focused ? toneWash("accent") : "transparent"}
                  stroke={i === focused ? toneColor("accent") : "none"}
                />
                {segment(b.other, toneColor(statusTone("neutral")))}
                {segment(b.warning, toneColor(statusTone("warning")))}
                {segment(b.danger, toneColor(statusTone("danger")))}
              </g>
            );
          })}
        </svg>
      </div>
      <div data-slot="volume-caption" className="path mt-0.5 text-faint">
        {caption}
      </div>
    </div>
  );
}
