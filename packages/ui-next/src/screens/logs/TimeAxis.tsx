/**
 * The labels under the plot, each at its instant's place along `[from, to]`.
 * HTML, not SVG text: the plot's viewBox is stretched to the pane, and would
 * stretch the letters with it.
 */
export function TimeAxis({
  ticks,
  labels,
  from,
  to,
}: {
  ticks: readonly number[];
  labels: readonly string[];
  from: number;
  to: number;
}) {
  return (
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
            {labels[i]}
          </span>
        );
      })}
    </div>
  );
}
