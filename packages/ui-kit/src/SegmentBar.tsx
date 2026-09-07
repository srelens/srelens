import { cx } from "./cx";
import { toneColor, type Tone } from "./tone";

export interface SegmentBarProps {
  segments: Array<{ value: number; tone: Tone; label: string }>;
  ariaLabel: string;
  className?: string;
}

/**
 * One strip split proportionally into coloured runs: "18 running, 3 pending,
 * 1 failed" read as a single shape rather than three numbers to add up.
 *
 * Hardened over the version this came from in two ways. That version named
 * itself "Segmented status bar", which describes the widget's shape rather than
 * what it counts and is the same for every bar on a page — a screen reader
 * meeting three of them learns nothing about which is pods and which is nodes —
 * so the name is required from the caller instead, and given a role, since an
 * aria-label on a plain div is ignored. And that version clamped negatives when
 * summing the total but then took each width from the raw value, so one
 * negative count drew backwards against a total that had never counted it;
 * clamping now happens in both places, from the same numbers. Only the geometry
 * is clamped — the tooltip still reports the real figure, because a count that
 * has gone negative is a fault worth seeing rather than one worth hiding.
 *
 * The runs inside are presentational: under `role="img"` nothing below the
 * element is announced, and the tooltips are hover-only besides. So the name is
 * the whole of what a screen reader gets, and it should carry the breakdown —
 * "18 running, 3 pending, 1 failed" — rather than just naming the bar. (#318)
 */
export function SegmentBar({ segments, ariaLabel, className }: SegmentBarProps) {
  // A total that does not count what the widths are drawn from is the bug this
  // component came with; both sides read this one clamped number.
  const widths = segments.map((segment) => Math.max(0, segment.value));
  const total = widths.reduce((sum, value) => sum + value, 0);

  return (
    <div
      className={cx("flex h-2.5 w-full overflow-hidden rounded-full", className)}
      style={{ background: "var(--field)" }}
      role="img"
      aria-label={ariaLabel}
    >
      {segments.map((segment, index) => (
        <span
          key={segment.label}
          className="block h-full"
          // An empty cluster, or one that has not reported yet, sums to zero;
          // dividing by it puts NaN in the style and the run vanishes silently.
          style={{
            width: `${total > 0 ? (widths[index] / total) * 100 : 0}%`,
            background: toneColor(segment.tone),
          }}
          title={`${segment.label}: ${segment.value}`}
        />
      ))}
    </div>
  );
}
