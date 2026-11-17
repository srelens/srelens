import { useId, type ReactNode } from "react";
import { Button } from "./Button";
import { cx } from "./cx";
import { toneColor, toneWash, type Tone } from "./tone";
import { filled } from "./slot";

export interface MetricTileProps {
  label: ReactNode;
  /** Left out while there is no figure yet — loading, or a read that failed — rather than drawn as a blank or a zero. */
  value?: ReactNode;
  description?: ReactNode;
  tone?: Tone;
  action?: ReactNode;
  /** Opens what the figure counts: the label becomes a button, named "Open" and the label. */
  onOpen?: () => void;
  /** More under the figure: a breakdown, the rows behind it, a loading or failure state. */
  children?: ReactNode;
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
 *
 * The tile is named by its label, so a row of them reads as a list of names
 * rather than of numbers. An app's dashboard card is one (#1001): it names
 * itself, opens the list behind its figure, and its label and description
 * wrap, since one unbroken word in either is enough to run out of the tile.
 */
export function MetricTile({
  label,
  value,
  description,
  tone = "muted",
  action,
  onOpen,
  children,
  className,
}: MetricTileProps) {
  const id = useId();
  const name = <span id={id}>{label}</span>;
  return (
    <article
      aria-labelledby={id}
      className={cx("stat flex items-start justify-between gap-3", className)}
      data-tone={tone}
      style={{ background: toneWash(tone) }}
    >
      <div className="min-w-0 flex-1">
        <div className="eyebrow [overflow-wrap:anywhere]" style={{ color: toneColor(tone) }}>
          {onOpen ? (
            // A button resets text-transform; the label keeps the eyebrow's case.
            <Button variant="link" className="[text-transform:inherit]" onClick={onOpen}>
              <span className="sr-only">Open </span>
              {name}
            </Button>
          ) : (
            name
          )}
        </div>
        {filled(value) && <div className="stat-value">{value}</div>}
        {filled(children) && <div className="mt-1">{children}</div>}
        {filled(description) && <p className="path mt-0.5 [overflow-wrap:anywhere]">{description}</p>}
      </div>
      {filled(action) && <div className="shrink-0">{action}</div>}
    </article>
  );
}
