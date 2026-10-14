import type { VolumeBucket } from "@srelens/core";

/**
 * How far along the plot an instant is, in percent. The plot gives every bar
 * one slot of the same width, so an instant is placed in the slot of the bar
 * that holds it, as far along as it is along that bar. By elapsed time instead,
 * the labels after a bar that covers more than its slot (the repeated hour of a
 * fall-back is two hours in one slot) would sit under the wrong bars. A bar with
 * no width, a skipped hour, holds no instant. `to`, the last bar's exclusive end,
 * holds none either, and is the right-hand edge.
 */
function along(t: number, buckets: readonly VolumeBucket[]): number {
  const i = buckets.findIndex((b) => b.start <= t && t < b.end);
  if (i < 0) return 100;
  const b = buckets[i];
  return ((i + (t - b.start) / (b.end - b.start)) * 100) / buckets.length;
}

/**
 * The labels under the plot, each at its instant's place along the bars.
 * HTML, not SVG text: the plot's viewBox is stretched to the pane, and would
 * stretch the letters with it.
 */
export function TimeAxis({
  ticks,
  labels,
  buckets,
}: {
  ticks: readonly number[];
  labels: readonly string[];
  buckets: readonly VolumeBucket[];
}) {
  return (
    <div aria-hidden="true" className="path relative h-4">
      {ticks.map((t, i) => {
        const at = along(t, buckets);
        return (
          <span
            key={t}
            data-slot="x-tick"
            className="absolute top-0.5 whitespace-nowrap text-[0.6875rem] leading-none"
            // Centred on its tick, but kept inside the plot at either end.
            style={{ left: `${at}%`, transform: `translateX(${at < 5 ? "0" : at > 95 ? "-100%" : "-50%"})` }}
          >
            {labels[i]}
          </span>
        );
      })}
    </div>
  );
}
