import type { ReactNode } from "react";
import { cx } from "./cx";
import { toneColor, type Tone } from "./tone";

export interface EyebrowProps {
  children: ReactNode;
  /**
   * Tint the label with a tone's colour. Left off, it keeps the muted colour
   * `.eyebrow` gives it, which is what nearly every one of them wants.
   */
  tone?: Tone;
  className?: string;
}

/**
 * The small tracked uppercase label — the design's quietest voice, used above a
 * figure, beside a control and anywhere a word is naming something rather than
 * saying it.
 *
 * A component rather than the bare class because the class is worn in a dozen
 * places and the voice is a decision the design gets to change once. The mock
 * took a `style` prop, which existed only to colour the odd warning label; that
 * is a hole straight through the token rule, so it becomes `tone` and the
 * colour comes from {@link toneColor}. Absent a tone there is no inline style
 * at all — an untinted eyebrow leaves the stylesheet in charge of it. (#320)
 */
export function Eyebrow({ children, tone, className }: EyebrowProps) {
  return (
    <div
      className={cx("eyebrow", className)}
      data-tone={tone}
      style={tone ? { color: toneColor(tone) } : undefined}
    >
      {children}
    </div>
  );
}
