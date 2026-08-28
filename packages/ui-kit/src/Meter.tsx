import { toneColor, type Tone } from "./tone";

/**
 * A meter must have an accessible name: nearby text does not name a generic
 * ARIA meter, so several unnamed ones read as a list of numbers with nothing
 * saying which resource each belongs to. Required as a union rather than
 * checked at runtime, so the compiler asks for it. (#317 review)
 */
type MeterProps = { value: number; tone?: Tone } & (
  | { ariaLabel: string; ariaLabelledBy?: never }
  | { ariaLabelledBy: string; ariaLabel?: never }
);

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
 */
export function Meter({ value, tone, ...naming }: MeterProps) {
  const resolved: Tone = tone ?? (value > 80 ? "sev" : value > 65 ? "warn" : "ok");
  const width = Math.min(Math.max(value, 0), 100);
  return (
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
        aria-valuenow={width}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuetext={`${value}%`}
      >
        <div
          className="h-full rounded-full"
          style={{ width: `${width}%`, background: toneColor(resolved) }}
        />
      </div>
      <span className="num w-9 shrink-0 text-right text-[0.6875rem] text-muted">{value}%</span>
    </div>
  );
}
