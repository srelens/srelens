import { groupNumber } from "../../lib/numbers";
import { Dot, type BarGroup } from "./volumeGroups";

/**
 * One bar in words, above it: its time range, how many lines, and a coloured
 * count per group that has any. HTML and instant, where a native `title` waits
 * a second and says nothing the chart's own colours don't. `at` is where the
 * bar's middle is, as a percentage of the plot's width; the box keeps inside
 * the plot at either end.
 */
export function VolumeTooltip({
  range,
  total,
  groups,
  at,
}: {
  range: string;
  total: string;
  groups: readonly BarGroup[];
  at: number;
}) {
  return (
    <div
      role="tooltip"
      data-slot="volume-tooltip"
      // Over the pointer's own events: it sits where the bar is, and must not take the hover from it.
      className="pointer-events-none absolute bottom-full z-10 mb-1 w-max rounded border border-rule bg-raised px-2 py-1 text-[0.6875rem] leading-snug shadow-md"
      style={{ left: `${at}%`, transform: `translateX(${at < 15 ? "0" : at > 85 ? "-100%" : "-50%"})` }}
    >
      <div data-slot="tooltip-range" className="text-ink">
        {range}
      </div>
      <div data-slot="tooltip-total" className="text-muted">
        {total}
      </div>
      {groups.map(({ key, kind, word, n }) => (
        <div key={key} data-slot="tooltip-group" className="flex items-center gap-1.5 text-muted">
          <Dot kind={kind} faint={key === "other"} />
          {word} <span className="tabular-nums text-ink">{groupNumber(n)}</span>
        </div>
      ))}
    </div>
  );
}
