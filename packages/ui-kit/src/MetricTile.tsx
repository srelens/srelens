import type { ReactNode } from "react";
import { cx } from "./cx";
import { toneColor, toneWash, type Tone } from "./tone";
import { filled } from "./slot";

export interface MetricTileProps {
  label: ReactNode;
  value: ReactNode;
  description?: ReactNode;
  tone?: Tone;
  action?: ReactNode;
  className?: string;
}

/**
 * One headline figure with the label that says what it counts — the row of
 * tiles across the top of a cluster overview.
 *
 * The tone tints the surface and the label, and deliberately leaves the number
 * in the body colour: the figure is what the reader came for, and a red 248 is
 * harder to read than a black one sitting on a red wash. Severity is context
 * around the number, not the number itself.
 *
 * The classic tile spoke the dashboard's own tone names — primary, success,
 * danger, neutral — and that vocabulary does not come across; the kit has one
 * tone set, and a second one only survives as long as someone keeps the two
 * translations in step. `neutral` becomes `muted`, whose wash is transparent,
 * so an untinted tile still takes its colour from the surface it lands on
 * rather than from a special case here. (#318)
 */
export function MetricTile({
  label,
  value,
  description,
  tone = "muted",
  action,
  className,
}: MetricTileProps) {
  return (
    <article
      className={cx("stat flex items-start justify-between gap-3", className)}
      data-tone={tone}
      style={{ background: toneWash(tone) }}
    >
      <div className="min-w-0">
        <div className="eyebrow" style={{ color: toneColor(tone) }}>
          {label}
        </div>
        <div className="stat-value">{value}</div>
        {filled(description) && <p className="path mt-0.5">{description}</p>}
      </div>
      {filled(action) && <div className="shrink-0">{action}</div>}
    </article>
  );
}
