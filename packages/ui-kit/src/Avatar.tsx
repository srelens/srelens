import { cx } from "./cx";
import { toneColor, toneWash, type Tone } from "./tone";

export interface AvatarProps {
  /** The person. Both what the circle is named by and where its initials come from. */
  name: string;
  tone?: Tone;
  className?: string;
}

/**
 * The initials of whoever did something, in a tinted circle.
 *
 * It is `role="img"` named by the full name, because the initials are a picture
 * of a person rather than text worth reading: "DK" spoken aloud is not a name,
 * and the role makes the letters presentational so they are not read twice. The
 * mock hung the name off `title` on a plain span, which is announced
 * inconsistently, never appears on touch, and — set alongside an `aria-label` —
 * becomes the accessible description, so the name is read out twice. There is
 * no `title` here at all: a hover tooltip that repeats the accessible name buys
 * a mouse user nothing that the row beside it does not already say. (#320)
 *
 * The initials come from characters, not code units. `name.split(" ")` yields
 * an empty segment for a double space and `p[0]` on it is `undefined`, which
 * React prints as the word; `p[0]` on an astral character is half a surrogate
 * pair, which renders as a replacement box. Splitting on runs of whitespace and
 * taking the first character of the first two words fixes both. (#320)
 *
 * With no name there is nothing to be named by, and an ARIA img with no
 * accessible name is a defect rather than an anonymous picture, so the circle
 * drops out of the accessibility tree and stays as decoration. (#320)
 */
export function Avatar({ name, tone = "accent", className }: AvatarProps) {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const initials = words
    .slice(0, 2)
    .map((word) => Array.from(word)[0] ?? "")
    .join("")
    .toUpperCase();

  return (
    <span
      className={cx(
        "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[0.625rem] font-semibold",
        className,
      )}
      data-tone={tone}
      style={{ background: toneWash(tone), color: toneColor(tone) }}
      {...(initials ? { role: "img", "aria-label": name.trim() } : { "aria-hidden": true })}
    >
      {initials}
    </span>
  );
}
