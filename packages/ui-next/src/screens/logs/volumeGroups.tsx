import type { VolumeBucket } from "@srelens/core";
import { statusTone, toneColor, type StatusKind } from "@srelens/ui-kit";

/**
 * A bar's segments, bottom to top, and the legend's entries in the same order.
 * Errors sit on the baseline, so a spike of them compares across bars; what has
 * no severity goes last and is drawn faint. The colour is core's health tone
 * for `kind`, never a local one; `word` is only what the legend calls it.
 */
export const GROUPS = [
  { key: "danger", kind: "danger", word: "error" },
  { key: "warning", kind: "warning", word: "warn" },
  { key: "info", kind: "info", word: "info" },
  { key: "other", kind: "neutral", word: "other" },
] as const satisfies readonly { key: keyof VolumeBucket; kind: StatusKind; word: string }[];

/** One of the groups, with how many lines a bar holds of it. */
export type BarGroup = (typeof GROUPS)[number] & { n: number };

/** How much of its colour the unlevelled lines keep. */
export const OTHER_OPACITY = 0.4;

/** A group's colour as a dot: the legend's and the tooltip's, filled as the bars are. */
export function Dot({ kind, faint }: { kind: StatusKind; faint: boolean }) {
  return (
    <svg data-slot="dot" aria-hidden="true" width={8} height={8} viewBox="0 0 8 8" className="shrink-0">
      <circle cx={4} cy={4} r={4} fill={toneColor(statusTone(kind))} fillOpacity={faint ? OTHER_OPACITY : undefined} />
    </svg>
  );
}
