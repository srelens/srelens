import type { ReactNode } from "react";
import { SubHead } from "./SubHead";
import { cx } from "./cx";
import { filled } from "./slot";

export interface SectionProps {
  /**
   * The small bold line naming the block. Left off for the first block in a
   * run, which the design heads with nothing — the pane's own header has
   * already said what the subject is.
   */
  title?: ReactNode;
  children: ReactNode;
  className?: string;
  /**
   * Whether the content is showing. Read only when the block is a disclosure
   * — see {@link SectionProps.onToggle} — and open is what every caller that
   * offers no toggle gets, which is what every call site had before there was
   * one.
   */
  open?: boolean;
  /**
   * Hand this in and the heading becomes a disclosure button, reporting the
   * state it is moving to.
   *
   * Controlled outright: this component keeps no state of its own and reads
   * no storage, so whether a block is open — and whether that outlives the
   * session — is entirely the app's. Same split as `Sidebar`/`ResizeHandle`
   * for the resizable width: the kit reports, the app persists. A section
   * that flipped itself would disagree with that memory the moment the app
   * said otherwise, and there would be two answers to one question.
   */
  onToggle?: (next: boolean) => void;
  /**
   * Whether the CONTENT keeps the block's horizontal inset. `false` runs it to
   * both edges of the surface — the design's own `padded: false`, which it
   * names for "tables and list rows" (§D): a table inside an inset draws its
   * hairlines short of the rules dividing the sections around it, and a run of
   * list rows loses the full-width hover fill.
   *
   * The HEADING keeps the inset either way. It is a label sitting over the
   * band rather than part of it, and a heading flush to the window edge lines
   * up with nothing.
   */
  padded?: boolean;
  /**
   * Head the block in the design's small-caps section voice rather than the
   * detail body's small bold line — `SubHead`'s two variants.
   *
   * Off by default because the detail pane, which is every call site this
   * component was built for, draws the bold one. The cluster overview's bands
   * are the frame that asks for the other.
   */
  smallCaps?: boolean;
}

/** Inline rather than an icon-set import: the kit takes no dependency on lucide. */
function Caret({ open }: { open: boolean }) {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className="section-caret shrink-0"
      data-open={open}
    >
      <path
        d="m9 18 6-6-6-6"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/**
 * A flat block of content with an optional heading, divided from the block
 * before it by a hairline rule, and able to fold away behind that heading.
 *
 * The other shape beside `Panel`, not a flag on it. A panel is a card: a
 * lifted surface, a border all the way round and a ruled head in small caps,
 * which is right for a section of a page standing on its own. A detail body is
 * the opposite — one subject read top to bottom, its parts separated rather
 * than boxed — and stacking cards inside a 352px peek spends most of the width
 * on borders and leaves the eye four frames to cross instead of one column to
 * read. Both call sites exist, so both shapes do. (#331)
 *
 * The divider is a sibling rule (`.section + .section`), which is what makes a
 * run of these read as divided rather than framed: no line above the first, none
 * below the last, and a caller that renders a block conditionally gets the
 * right answer without counting. Nothing to pass, nothing to keep in sync.
 * A CLOSED SECTION IS STILL A SECTION for exactly this reason — it renders its
 * heading and drops its content, rather than rendering nothing, which would
 * take a hairline away with it.
 *
 * The heading is `SubHead` — an `h3`, so the blocks of a peek appear in the
 * document outline under the peek's own `h2`. That is the same finding
 * `Panel`'s heading came from: a styled div names a block for people who can
 * see it and for nobody else. When the block folds, the button lives INSIDE
 * that heading rather than replacing it, so the outline is unchanged and the
 * control is reachable by Tab and by Enter with no key handling of ours.
 *
 * WHAT FOLDING MEANS HERE: the content is not rendered at all. Not hidden,
 * not `display: none` — absent. `ui-next`'s annotations gate depends on that
 * distinction (a `kubectl apply`-managed Secret carries its whole data map in
 * an annotation), and a block that merely hid its rows would put every one of
 * them back in the markup.
 *
 * An untitled section has nothing to hang the control on, so it ignores both
 * props and stays open. The design heads the first block of a detail with
 * nothing, and a pane that opens showing nothing at all is hostile.
 *
 * There is deliberately no `aria-controls`. The rows stay the section's own
 * direct children — a caller lays them out (`ui-next`'s full tab grids its
 * facts three across), and a panel element around them would be a box between
 * that caller's grid and the rows it is placing — which leaves no single
 * element for an id to point at. `aria-expanded` on a button inside the
 * heading that names the block is the disclosure pattern's own requirement;
 * `aria-controls` is its optional half, and it is the half almost no screen
 * reader acts on.
 */
export function Section({
  title,
  children,
  className,
  open = true,
  onToggle,
  padded = true,
  smallCaps = false,
}: SectionProps) {
  const headed = filled(title);
  const folds = headed && onToggle !== undefined;
  const showing = folds ? open : true;

  return (
    <section
      className={cx("section", className)}
      data-open={folds ? showing : undefined}
      data-padded={padded ? undefined : "false"}
    >
      {headed && (
        <SubHead className="section-title" variant={smallCaps ? "caps" : "bold"}>
          {folds ? (
            <button
              // A bare button inside a form is a submit button (bd24d1a).
              type="button"
              className="section-toggle"
              aria-expanded={showing}
              onClick={() => onToggle?.(!showing)}
            >
              <Caret open={showing} />
              <span className="truncate">{title}</span>
            </button>
          ) : (
            title
          )}
        </SubHead>
      )}
      {showing && children}
    </section>
  );
}
