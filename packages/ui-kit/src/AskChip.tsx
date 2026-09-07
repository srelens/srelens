import { cx } from "./cx";

export interface AskChipProps {
  /** What gets asked. Also what tells this chip apart from every other one. */
  question: string;
  /** The word on the chip. Short, because it sits on a row. */
  label?: string;
  /**
   * Handed the question. A prop rather than a context hook: the console is
   * product knowledge, and the kit does not import the service layer.
   */
  onAsk: (question: string) => void;
  /** For while an earlier question is still in flight. */
  disabled?: boolean;
  className?: string;
}

/**
 * Ask-in-place: a small chip on a row, a log line or an event that hands what
 * you are looking at to the agent.
 *
 * Three things the mock got wrong, all of them from where this stands rather
 * than from what it is. It was a bare `<button>`, so inside a form — a table of
 * rows in a filter form is the ordinary case — it submitted on the click that
 * was meant to ask a question. It named itself "Ask", and there is one of these
 * per row, so a screen reader reading the page gets "Ask, Ask, Ask, Ask" and no
 * way to tell which row it is on; the question goes in the accessible name,
 * where it distinguishes them, and the visible word stays short. And `.row-ask`
 * is `opacity: 0` until its row is hovered, which a keyboard user cannot do —
 * so it was a focusable control nobody could see, and the focus ring landed on
 * nothing. It shows itself on focus.
 *
 * It stops both the click and the Enter that produced it from reaching the row
 * underneath, because that row selects, and asking about a row is not selecting
 * it. The mock stopped the click only, which left a list that moves on keydown
 * acting on the same keystroke.
 *
 * An empty question renders nothing at all. The alternative is a control that
 * appears on hover, takes a tab stop, and does nothing when used. (#320)
 */
export function AskChip({ question, label = "Ask", onAsk, disabled, className }: AskChipProps) {
  if (!question.trim()) return null;

  return (
    <button
      type="button"
      className={cx("row-ask focus-visible:opacity-100", className)}
      // The visible word is the same on every chip; the name is not.
      aria-label={`${label}: ${question}`}
      title={`${label}: ${question}`}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onAsk(question);
      }}
      onKeyDown={(e) => {
        // Enter and Space are what activate a button, and the click they
        // produce is already stopped above. This stops the keystroke itself,
        // which a list navigating on keydown would otherwise act on too.
        if (e.key === "Enter" || e.key === " ") e.stopPropagation();
      }}
    >
      {/* Inline rather than an icon-set import: the kit takes no dependency on
          lucide, and this is the only glyph it needs. */}
      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M12 2.5 14.2 9.8 21.5 12l-7.3 2.2L12 21.5l-2.2-7.3L2.5 12l7.3-2.2z" fill="currentColor" />
      </svg>
      {label}
    </button>
  );
}
