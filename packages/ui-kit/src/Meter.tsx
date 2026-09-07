import type { ReactNode } from "react";
import { loadTone, toneColor, type Tone } from "./tone";
import { filled } from "./slot";

/**
 * A meter must have an accessible name: nearby text does not name a generic
 * ARIA meter, so several unnamed ones read as a list of numbers with nothing
 * saying which resource each belongs to. Required as a union rather than
 * checked at runtime, so the compiler asks for it. (#317 review)
 *
 * `label` does not satisfy this and is not an alternative to it. It is a
 * visible caption; the accessible name is a separate obligation, and a meter
 * can want a short visible label ("CPU") alongside a fuller spoken one ("Node
 * CPU"). Anyone tempted to collapse the two should note that dropping the union
 * silently un-names every meter rendered without a caption. (#318)
 */
type MeterProps = {
  value: number;
  tone?: Tone;
  /** Visible caption above the bar. Not the accessible name. */
  label?: ReactNode;
  /** A line under the bar giving the figure behind the proportion. */
  detail?: ReactNode;
} & ({ ariaLabel: string; ariaLabelledBy?: never } | { ariaLabelledBy: string; ariaLabel?: never });

/**
 * A proportion bar with its percentage beside it.
 *
 * Picks its own tone from the value unless told otherwise, so a row of meters
 * reads as a heat map without every caller repeating the thresholds.
 *
 * Hardened over the version this came from: a pod over its limit reports more
 * than 100%, and a bar that runs past its track looks like a rendering fault
 * rather than the reading it is. The number keeps the real value; only the bar
 * is clamped.
 *
 * `label` and `detail` come from the classic `StatusMeter`, which was this
 * component with a caption. Carrying it as a second component would have left
 * the kit with two meters differing by one line, so the line moved here — and
 * with it that version's layout, which puts the caption and the number in a row
 * above the bar. The percentage moves rather than doubling: a captioned meter
 * shows it above, a bare one beside, never both. (#318)
 */
export function Meter({ value, tone, label, detail, ...naming }: MeterProps) {
  const resolved: Tone = tone ?? loadTone(value);
  const width = Math.min(Math.max(value, 0), 100);
  // These figures are ratios — a third of three pods is 33.33333333333333 —
  // and fourteen decimal places crowd the bar on screen and are worse read
  // aloud. Rounded for reading only; the bar keeps the full precision, and
  // rounding does not hide an over-limit pod the way clamping would. (#325 review)
  const shown = Math.round(value);
  const bar = (
    <div className="flex items-center gap-2">
      <div
        className="h-[5px] w-full overflow-hidden rounded-full"
        style={{ background: "var(--field)" }}
        role="meter"
        aria-label={"ariaLabel" in naming ? naming.ariaLabel : undefined}
        aria-labelledby={"ariaLabelledBy" in naming ? naming.ariaLabelledBy : undefined}
        // Clamped to the range this element declares: 150 against a max of 100
        // is an invalid meter value, and assistive technology may clamp it
        // silently or skip the element. The real figure goes in aria-valuetext,
        // which is free text, so nothing is hidden from a screen reader that a
        // sighted reader can see. (#317 review)
        aria-valuenow={Math.round(width)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuetext={`${shown}%`}
      >
        <div
          className="h-full rounded-full"
          style={{ width: `${width}%`, background: toneColor(resolved) }}
        />
      </div>
      {!filled(label) && (
        <span className="num w-9 shrink-0 text-right text-[0.6875rem] text-muted">{shown}%</span>
      )}
    </div>
  );

  // An unadorned meter is exactly what it was — no wrapper, so it stays the
  // flex item its callers lay out. The wrapper appears only when there is
  // something to stack above or below the bar.
  if (!filled(label) && !filled(detail)) return bar;

  return (
    <div>
      {filled(label) && (
        <div
          data-slot="meter-head"
          className="flex items-baseline justify-between gap-2 text-[0.6875rem]"
        >
          <span className="truncate text-muted">{label}</span>
          <span className="num shrink-0">{shown}%</span>
        </div>
      )}
      {bar}
      {filled(detail) && (
        <p data-slot="meter-detail" className="path mt-0.5">
          {detail}
        </p>
      )}
    </div>
  );
}
