import type { ReactNode } from "react";
import { cx } from "./cx";
import { Eyebrow } from "./Eyebrow";
import { filled } from "./slot";
import { toneColor, type Tone } from "./tone";

export interface StatProps {
  label: ReactNode;
  value: ReactNode;
  /** The reading behind the figure: a change, a share, a count of what is wrong. */
  delta?: ReactNode;
  tone?: Tone;
  className?: string;
}

/**
 * A labelled figure in a divided row — the strip of numbers across the top of a
 * cluster, an incident list or a workload.
 *
 * The tone is spent on the delta alone. That line is the judgement — "3 not
 * ready", "82%", "−6 min" — and colouring it is how a row of five figures shows
 * which one to look at, while the figures themselves stay in the body colour so
 * they can still be compared with each other. It also reaches the root as
 * `data-tone`, so a stat with no delta still says what it was given.
 *
 * Deliberately not {@link MetricTile}, which reads much the same — both are a
 * label over a figure on the same two classes. What separates them is what a
 * tone does: it tints a tile's surface, and it colours a stat's delta. Folding
 * them, as this kit did with SectionPanel and StatusMeter, would mean a prop
 * whose only job is to say which of the two you meant, and a row of five
 * washed cells reads as stripes. Two components until a third one arrives and
 * says otherwise. (#320)
 *
 * The mock baked `flex-1` into the root. Two utilities that both set `flex` are
 * resolved by stylesheet order rather than attribute order, so a baked one
 * cannot be undone through `className`; the row that lays these out sizes them
 * instead. (#320)
 */
export function Stat({ label, value, delta, tone = "muted", className }: StatProps) {
  return (
    <div className={cx("stat", className)} data-tone={tone}>
      <Eyebrow>{label}</Eyebrow>
      <div className="stat-value">{value}</div>
      {filled(delta) && (
        <div className="num mt-1 text-[0.6875rem]" style={{ color: toneColor(tone) }}>
          {delta}
        </div>
      )}
    </div>
  );
}
