import { cx } from "./cx";

export interface AgentMarkProps {
  /** The edge of the square, in pixels. Clamped to a range it can be drawn at. */
  size?: number;
  /**
   * What the mark is called, for a screen reader. Given only where the mark
   * stands alone — beside the word "Agent" it is decoration, and naming it
   * there says the same thing twice.
   */
  label?: string;
  className?: string;
}

/** The range the glyph is legible in: below this it is a smudge, above it a poster. */
const MIN = 12;
const MAX = 64;

/**
 * The agent's mark — an accent square with a spark in it, sized to its context.
 *
 * The mock fed `size` straight into `width`, `height`, a radius and the icon's
 * own size, which makes four pieces of arithmetic out of one number nobody
 * validated. A zero from a collapsed layout drew an invisible mark, a negative
 * from arithmetic upstream drew nothing at all, and a stray large value drew a
 * square over the page — so the number is clamped to the range the glyph is
 * actually legible in, and rounded, because a fractional pixel box holds a
 * fractional pixel spark.
 *
 * The mock also said nothing to a screen reader either way: no `aria-hidden`,
 * so an unlabelled mark was an unnamed graphic, and no way to name it where it
 * is the only thing identifying the agent. It is decoration by default, since
 * nearly every one of them sits next to the word it illustrates, and takes a
 * name for the times it does not.
 *
 * The spark is drawn here rather than imported from lucide: the kit takes no
 * dependency on an icon set, and this is the only glyph it needs. (#320)
 */
export function AgentMark({ size = 19, label, className }: AgentMarkProps) {
  const px = Math.round(Math.min(Math.max(Number.isFinite(size) ? size : MIN, MIN), MAX));
  const glyph = Math.round(px * 0.58);

  return (
    <span
      className={cx("agent-mark", className)}
      style={{ width: px, height: px, borderRadius: px > 22 ? 6 : 5 }}
      // A named mark is a graphic; an unnamed one is furniture. Never both, and
      // never neither, which is what the mock left it as.
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : "true"}
    >
      <svg width={glyph} height={glyph} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M12 2.5 14.2 9.8 21.5 12l-7.3 2.2L12 21.5l-2.2-7.3L2.5 12l7.3-2.2z" fill="currentColor" />
      </svg>
    </span>
  );
}
