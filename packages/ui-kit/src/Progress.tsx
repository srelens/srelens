import type { ReactNode } from "react";
import { cx } from "./cx";
import { Eyebrow } from "./Eyebrow";
import { filled } from "./slot";

/**
 * A progressbar must have an accessible name: neighbouring text does not name
 * one, and three unnamed bars read as three percentages with nothing saying
 * what is progressing. Required as a union rather than checked at runtime, so
 * the compiler asks for it — the same shape {@link Meter} settled on for the
 * same reason. (#317, #320)
 *
 * `label` is a visible caption and not an alternative to it: a bar can want a
 * short one on screen ("Rollout") and a fuller spoken one ("Rollout of
 * api-server").
 */
export type ProgressProps = {
  value: number;
  /** Visible caption above the bar, printed beside the figure. Not the accessible name. */
  label?: ReactNode;
  className?: string;
} & ({ ariaLabel: string; ariaLabelledBy?: never } | { ariaLabelledBy: string; ariaLabel?: never });

/**
 * How far along a task is: a rollout, an upload, a drain.
 *
 * Deliberately not {@link Meter}, which is the same bar drawn from the same
 * kind of number. What separates them is the ARIA role, and the roles are not
 * interchangeable: `meter` is a reading inside a known range — disk at 71%,
 * true until it changes and as likely to fall as to rise — while `progressbar`
 * is a task advancing toward completion, which only goes forward and then ends.
 * A screen reader says different things about the two, and folding them would
 * leave a prop whose only job is to choose between them. Hence also no `tone`:
 * a meter picks a colour from its value because a full disk is bad news, and a
 * rollout at 95% is not. (#320)
 *
 * Hardened over the mock, which set the bar's width straight from the value.
 * Anything over 100 ran the fill past its track, which reads as a rendering
 * fault rather than a reading, and `aria-valuenow` outside the min and max the
 * element itself declares is invalid — assistive technology may clamp it
 * quietly or skip the element. The bar and `aria-valuenow` are clamped; the
 * printed figure and `aria-valuetext`, which is free text, keep the real one,
 * so nothing is hidden from a screen reader that a sighted reader can see.
 * (#317, #320)
 */
export function Progress({ value, label, className, ...naming }: ProgressProps) {
  const width = Math.min(Math.max(value, 0), 100);
  // These come out of ratios — one of three pods updated is 33.33333333333333
  // — and fourteen decimal places crowd the caption and are worse read aloud.
  // Rounded for reading only; the fill keeps its precision.
  const shown = Math.round(value);

  const bar = (
    <div
      role="progressbar"
      aria-label={"ariaLabel" in naming ? naming.ariaLabel : undefined}
      aria-labelledby={"ariaLabelledBy" in naming ? naming.ariaLabelledBy : undefined}
      aria-valuenow={Math.round(width)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuetext={`${shown}%`}
      className={cx("h-[5px] w-full overflow-hidden rounded-full", !filled(label) && className)}
      style={{ background: "var(--field)" }}
    >
      <div
        data-slot="progress-fill"
        className="h-full rounded-full"
        style={{ width: `${width}%`, background: "var(--accent)" }}
      />
    </div>
  );

  // A bare bar is exactly that — no wrapper, so it stays the element its
  // callers lay out. The wrapper appears only when there is a row to stack
  // above it, and takes the caller's spacing with it. (#318)
  if (!filled(label)) return bar;

  return (
    <div className={className}>
      <div
        data-slot="progress-head"
        className="mb-1 flex items-baseline justify-between gap-2"
      >
        <Eyebrow>{label}</Eyebrow>
        <span className="path shrink-0">{shown}%</span>
      </div>
      {bar}
    </div>
  );
}
