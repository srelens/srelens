import type { ReactNode } from "react";
import { toneColor, toneWash, type Tone } from "./tone";

export type BadgeTone = Tone;

/**
 * A small uppercase label carrying semantic colour — a pod phase, a severity,
 * a count that matters.
 *
 * `solid` inverts it: the tone becomes the fill and the text becomes the
 * surface, for the one badge on screen that should read first.
 *
 * The colours are computed rather than written into `kit.css` because the tone
 * set is shared with Meter and Sparkline, and duplicating five variants across
 * three stylesheets is how they drift apart.
 */
export function Badge({
  children,
  tone = "muted",
  solid = false,
}: {
  children: ReactNode;
  tone?: BadgeTone;
  solid?: boolean;
}) {
  return (
    <span
      className="badge"
      data-tone={tone}
      style={{
        color: solid ? "var(--surface)" : toneColor(tone),
        borderColor: solid
          ? toneColor(tone)
          : `color-mix(in srgb, ${toneColor(tone)} 40%, transparent)`,
        background: solid ? toneColor(tone) : toneWash(tone),
      }}
    >
      {children}
    </span>
  );
}
