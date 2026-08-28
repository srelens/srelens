import type { ReactNode } from "react";
import { cx } from "./cx";
import { Eyebrow } from "./Eyebrow";
import { filled } from "./slot";
import { toneColor, type Tone } from "./tone";

export interface LiveSignalProps {
  /**
   * What the signal is reporting. This is the meaning; the tone only colours
   * it, so a label that changes with the state ("Live signal" → "Stream lost")
   * is what makes the change legible to everyone.
   */
  label?: ReactNode;
  tone?: Tone;
  className?: string;
}

/**
 * A pulsing dot beside a word, saying that something on screen is arriving
 * rather than sitting still.
 *
 * The mock's `label` had a default and no floor under it, which reads as safe
 * until a caller writes `label={connected && "Streaming"}` — the ordinary way
 * to make a label conditional — and gets a bare coloured dot with no text
 * anywhere near it. A dot whose only content is its colour says nothing to a
 * screen reader and nothing to anyone who cannot separate red from green, so an
 * empty label falls back rather than disappearing.
 *
 * It is a `status` region because the thing it reports changes underneath the
 * reader: a stream that drops re-renders this component and nothing else, and
 * without a live region that is a silent failure. The dot itself is hidden,
 * since it repeats the label and nothing more. (#320)
 */
export function LiveSignal({ label, tone = "sev", className }: LiveSignalProps) {
  return (
    <div role="status" className={cx("flex items-center gap-2", className)}>
      <span
        aria-hidden="true"
        data-tone={tone}
        className="live-dot inline-block h-[7px] w-[7px] shrink-0 rounded-full"
        style={{ background: toneColor(tone) }}
      />
      <Eyebrow>{filled(label) ? label : "Live signal"}</Eyebrow>
    </div>
  );
}
