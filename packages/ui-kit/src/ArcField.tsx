import { useId } from "react";
import { cx } from "./cx";

export interface ArcFieldProps {
  /**
   * Replaces the default placement. It is `fixed inset-0 -z-10` — the field
   * behind the whole workspace — and `absolute inset-0` scopes it to a
   * positioned ancestor instead.
   */
  className?: string;
}

/**
 * The ambient field: a slow orthogonal grid crossed by wide arcs. It reads as a
 * radar sweep over graph paper — the instrument the workspace is printed on.
 *
 * The mock named its pattern `id="grid"`, which is fine exactly once. An id is
 * document-wide, `url(#grid)` resolves against the whole document, and a second
 * field — a dialog over a screen, two of them in a gallery — leaves both rects
 * filled from whichever pattern the document happens to reach first. It is the
 * kind of fault that never shows up in the mock, where the field is rendered
 * once at the root, and always shows up in a design system, where a component
 * is by definition rendered more than once. `useId` gives each instance its
 * own.
 *
 * The punctuation is stripped from that id rather than used as React hands it
 * over. React's ids are shaped for `htmlFor` and `aria-labelledby`, which take
 * a bare id string; they are not valid CSS identifiers, so `#«r0»` cannot be
 * selected by a stylesheet or a test even though `url(#…)` happens to tolerate
 * it. A prefix keeps it starting with a letter, which a CSS identifier must.
 *
 * `aria-hidden` is explicit: this is a texture, and a screen reader that walks
 * into an unlabelled graphic the size of the viewport has nothing useful to say
 * about it. It takes no pointer events either, so the layer that covers
 * everything cannot swallow a click meant for what is underneath. (#320)
 */
export function ArcField({ className }: ArcFieldProps) {
  const grid = `arcfield-${useId().replace(/[^A-Za-z0-9]/g, "")}`;

  return (
    <svg
      aria-hidden="true"
      className={cx("pointer-events-none", className ?? "fixed inset-0 -z-10 h-full w-full")}
      style={{ color: "var(--rule)", opacity: 0.85 }}
    >
      <defs>
        <pattern id={grid} width="112" height="112" patternUnits="userSpaceOnUse">
          <path d="M112 0H0v112" fill="none" stroke="currentColor" strokeWidth="1" opacity="0.5" />
        </pattern>
      </defs>
      <rect width="100%" height="100%" fill={`url(#${grid})`} />
      <g fill="none" stroke="currentColor" strokeWidth="1.5" opacity="0.9">
        <circle cx="18%" cy="118%" r="520" />
        <circle cx="18%" cy="118%" r="700" />
        <circle cx="18%" cy="118%" r="900" />
        <circle cx="96%" cy="-10%" r="420" />
        <circle cx="96%" cy="-10%" r="640" />
      </g>
    </svg>
  );
}
