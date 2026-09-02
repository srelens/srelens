import type { CopyState } from "./useCopied";

const WORD: Record<Exclude<CopyState, "idle">, string> = {
  copied: "Copied to clipboard",
  failed: "Could not copy to clipboard",
};

/**
 * Says out loud what a copy control only shows.
 *
 * **For the controls with no word to change.** A control that swaps a glyph and
 * nothing else tells a sighted reader it worked and tells a screen-reader user
 * nothing: the change is a repaint. A control that swaps a WORD does not need
 * this — it already says what happened, its accessible name is that same word,
 * and a live region beside it delivers the news a second time. So exactly one
 * of the two per control: {@link CopyIconButton} announces here, while
 * {@link CopyCommand} and a confirming {@link ActionBar} action speak in their
 * own text. Pinning a name to the action and announcing separately is the other
 * consistent answer, and it is the one this kit does NOT take where there is a
 * visible word, because a button reading "Copied" under the name "Copy as
 * kubectl" is a word not in its own name — what WCAG 2.5.3 is about, and what
 * strands anyone driving the app by voice. (#413 review)
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
