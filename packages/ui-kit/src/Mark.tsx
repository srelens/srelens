import { useState } from "react";
import { cx } from "./cx";
import type { IconComponent } from "./IconButton";
import { filled } from "./slot";

export type MarkSize = "sm" | "md" | "lg";

/** The rail draws marks at three sizes; the geometry below is derived from these. */
const SIZES: Record<MarkSize, number> = { sm: 26, md: 30, lg: 38 };

/** Longer than this and the text spills out of the smallest square. */
const MAX_SHORT = 3;

export interface MarkProps {
  /** What the mark stands for. Its accessible name, and the source of the initials. */
  name: string;
  /** Up to three characters drawn as the mark, or ridden under a glyph or image. */
  short?: string;
  /** Any CSS colour. The palette belongs to the caller; the default is a token. */
  color?: string;
  /** Drawn instead of the initials. Structural, so the kit needs no icon set. */
  icon?: IconComponent;
  /** Drawn instead of either — a data URL or a path. Falls back if it will not load. */
  imageSrc?: string;
  size?: MarkSize;
  /** Rings the mark. Presentation only; see the note below. */
  active?: boolean;
  /** Ride the short text under a glyph or image mark. On by default. */
  withBadge?: boolean;
  /** Hide the mark from assistive technology, for when the row around it is already named. */
  decorative?: boolean;
  className?: string;
}

/**
 * A square mark standing for one thing in a rail: initials, a glyph or an
 * image, on a colour the caller chooses, optionally with the short text riding
 * along the bottom edge so a glyph can still say which one it is.
 *
 * The version this came from took a resolved override straight out of the app's
 * hotbar store — an object of eleven fields, half of them about persistence —
 * and looked its glyph up in a map of sixteen lucide icons keyed by preset id.
 * Neither crosses into the kit: the store is the app's, and the kit takes no
 * dependency on an icon set. What is left is four values and a glyph passed in,
 * which is the call {@link NavIcon} made when its own icon map did not come
 * across. The name stays for the sake of the rail it was drawn for; nothing in
 * the API knows what a cluster is. (#320)
 *
 * Four states the mock had no answer for. It read `chip.short` straight, so an
 * override with the text cleared drew an empty coloured square — the initials
 * are now derived from the name instead, and a chip with nothing at all to draw
 * stops claiming to be an image rather than announcing an anonymous "image". It
 * drew `<img>` with no error handling, so a truncated data URL or a moved file
 * left the browser's broken-image glyph sitting in the rail; a failed image now
 * falls back to the glyph or the initials underneath. It was a bare `<span>`
 * with `alt=""` on the image, so the whole mark was silent — it now carries a
 * name, or is hidden outright when the control around it already says one. And
 * it wrote `#fff`, `text-white` and a hard-coded `#16151d` for the badge, which
 * are three colours that do not follow the theme; they are tokens now, the ink
 * being the one {@link Badge} already uses over a solid tone.
 *
 * `active` draws a ring and sets an attribute, and deliberately claims no ARIA
 * state: nothing here is pressable, so whichever control owns the selection is
 * the thing that has to say it is selected.
 */
export function Mark({
  name,
  short,
  color = "var(--accent)",
  icon: Icon,
  imageSrc,
  size = "md",
  active,
  withBadge = true,
  decorative,
  className,
}: MarkProps) {
  // An image that will not load is a state, not an error to report: the mark
  // beneath it is a perfectly good answer, so fall through to it silently.
  //
  // WHICH source failed, not a boolean: the mark outlives the source it is
  // drawing. {@link CustomizeMark} keeps three of these mounted across every
  // edit, so a flag would have latched on the first corrupt image the reader
  // picked and shown the fallback for every good one after it; the rail does
  // the same to a mark whose truncated stored data URL is later repaired. A
  // `key` on the `<img>` would remount the element but leave a flag set, so it
  // is no substitute.
  const [failed, setFailed] = useState<string>();
  const px = SIZES[size];

  // `filled` rather than a null check: `short={custom && override.short}` is how
  // a caller makes this conditional and it hands over `false`, and an empty
  // string is the case that drew a blank square.
  const text = (filled(short) ? String(short) : initials(name)).slice(0, MAX_SHORT).toUpperCase();
  const image = filled(imageSrc) && imageSrc !== failed ? String(imageSrc) : undefined;
  const label = name.trim() || text;

  // A glyph or an image says nothing about which one this is, so the short text
  // rides under it. On a text mark it would be the same characters twice.
  const badged = withBadge && text.length > 0 && (image !== undefined || Icon !== undefined);

  // An unnamed `role="img"` announces as "image" and tells the listener nothing,
  // which is worse than being skipped over.
  const named = !decorative && label.length > 0;

  return (
    <span
      className={cx("relative inline-flex shrink-0", className)}
      style={{ width: px, height: px }}
      data-active={active ? "true" : undefined}
      role={named ? "img" : undefined}
      aria-label={named ? label : undefined}
      aria-hidden={named ? undefined : "true"}
    >
      <span
        data-slot="chip-mark"
        className="flex h-full w-full items-center justify-center overflow-hidden rounded-[7px]"
        style={{
          // An image brings its own colours and is laid over the surface rather
          // than over the mark's colour, which would edge a transparent PNG.
          background: image !== undefined ? "var(--surface)" : color,
          // The ink over a solid colour, the same one Badge's `solid` uses.
          // Inherited, so the glyph's `currentColor` picks it up too.
          color: "var(--surface)",
          boxShadow: active
            ? `0 0 0 2px var(--canvas-deep), 0 0 0 3.5px ${color}`
            : undefined,
        }}
      >
        {image !== undefined ? (
          <img
            src={image}
            // Named by the wrapper: the mark stands for the name, and repeating
            // it here would announce it twice.
            alt=""
            className="h-full w-full object-cover"
            onError={() => setFailed(image)}
          />
        ) : Icon !== undefined ? (
          <Icon
            size={Math.round(px * (badged ? 0.46 : 0.52))}
            // The badge sits on the bottom edge; the glyph lifts off it.
            className={badged ? "-mt-[2px]" : undefined}
            aria-hidden="true"
          />
        ) : (
          <span
            className="font-semibold tracking-tight"
            style={{ fontSize: Math.round(px * (text.length > 2 ? 0.3 : 0.36)) }}
          >
            {text}
          </span>
        )}
      </span>

      {badged && (
        <span
          data-slot="chip-badge"
          className="pointer-events-none absolute left-1/2 -translate-x-1/2 rounded-[3px] font-bold uppercase leading-[1.4]"
          style={{
            bottom: -1,
            padding: `0 ${Math.max(2, Math.round(px * 0.08))}px`,
            fontSize: Math.max(7, Math.round(px * 0.26)),
            letterSpacing: "0.02em",
            // The mock pinned this to the dark theme's surface colour, so on a
            // light theme it was a black tab under every mark whatever the
            // theme said. Inverted against the surface instead, which reads on
            // both and needs no second decision.
            background: "var(--ink)",
            color: "var(--surface)",
            boxShadow: "0 0 0 1.5px var(--surface)",
          }}
        >
          {text}
        </span>
      )}
    </span>
  );
}

/**
 * Two letters standing for a name, for a mark whose short text was never set or
 * has been cleared.
 *
 * Split on the separators a context name actually uses — spaces, dots, dashes,
 * underscores and slashes — so `gke_acme-prod` gives "GA" rather than "GK".
 */
function initials(name: string): string {
  const words = name.split(/[\s._/-]+/).filter(Boolean);
  if (words.length === 0) return "";
  if (words.length === 1) return words[0].slice(0, 2);
  return words[0][0] + words[1][0];
}
