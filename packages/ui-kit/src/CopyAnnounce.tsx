import type { CopyState } from "./useCopied";

const WORD: Record<Exclude<CopyState, "idle">, string> = {
  copied: "Copied to clipboard",
  failed: "Could not copy to clipboard",
};

/**
 * Says out loud what a copy control only shows.
 *
 * A control that swaps a glyph and a word tells a sighted reader it worked and
 * tells a screen-reader user nothing: the change is a repaint, and a name that
 * changes under a button the reader has just activated is not reliably spoken.
 *
 * `role="status"` and nothing else. It carries `aria-live="polite"` implicitly,
 * and the kit's {@link Toast} already settled that writing both is the same
 * instruction twice. Polite rather than assertive because a copy confirmation
 * is not worth interrupting a reader mid-sentence for — the same line `Toast`
 * draws between `status` and `alert`.
 *
 * **It is mounted only while it has something to say, and that is a
 * compromise.** The textbook shape is a region that exists from the start and
 * is filled later, because a region inserted with its message already in it can
 * be missed. The cost of that shape here is one permanently empty `status` node
 * per copy control — several per screen, in a tree where `status` already means
 * something else (a Secret pane's redaction notice, among others), and every
 * one of them ambiguous to anything looking for the real one. A live region
 * nobody can find is its own defect. The right end state is one announcer at
 * the window root that these controls speak through; there is none yet, and
 * inventing one is more than this change should carry. (#410)
 */
export function CopyAnnounce({ state }: { state: CopyState }) {
  if (state === "idle") return null;
  return (
    <span role="status" className="sr-only">
      {WORD[state]}
    </span>
  );
}
