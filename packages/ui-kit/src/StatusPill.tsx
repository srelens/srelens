import type { ReactNode } from "react";
import { toneColor, type Tone } from "./tone";

export type StatusKind = "success" | "warning" | "danger" | "info" | "neutral";

/**
 * The classic kit named its own colours here — `bg-emerald-500`, `bg-amber-500`,
 * `bg-sky-500` — which is exactly what the tone system exists to stop: a raw
 * palette value does not follow the theme, and the failure is invisible until
 * someone switches to light. The kinds are the caller's vocabulary and survive;
 * they resolve to tones. (#318)
 */
const TONE: Record<StatusKind, Tone> = {
  success: "ok",
  warning: "warn",
  danger: "sev",
  info: "info",
  neutral: "muted",
};

/**
 * The tone a kind resolves to, for anything drawing a second channel of the
 * same fact beside a pill — `Inspector`'s flag dot is the one call site.
 *
 * Exported rather than let the caller keep its own copy: two maps of the same
 * five kinds drift, and the way they drift is exactly the bug this was added
 * for — a red dot beside an amber word, each side certain it was right. (#331)
 */
export function statusTone(kind: StatusKind): Tone {
  return TONE[kind];
}

/** Danger and warning are the states the design colours; the rest read plain. */
const BAD: Partial<Record<StatusKind, true>> = { danger: true, warning: true };

export interface StatusPillProps {
  status: ReactNode;
  kind?: StatusKind;
  /**
   * Colour the word itself when the state is bad — red `Degraded`, but plain
   * `Running`, and a plain `ReplicaFailure` beside its ok dot. Off by default.
   */
  tinted?: boolean;
}

/**
 * A status indicator: a coloured dot followed by a label.
 *
 * The label always carries the meaning in words. The dot is a second channel,
 * never the only one — a status told in colour alone is unreadable to a
 * colour-blind user and silent to a screen reader.
 *
 * `tinted` is the design's colouring rule, which is asymmetric on purpose: a
 * bad state is worth the ink, a good one is not, and a page where every row
 * says something in colour says nothing. It is opt-in rather than the default
 * because this component has some forty call sites — a Phase column, a
 * certificate row, a Helm release table — and turning the rule on for all of
 * them is a change to nearly every screen, not the one the detail pane asked
 * for. Callers that want it pass it. (#331)
 *
 * The colour is an inline style off the tone; the weight comes from
 * `.status[data-bad="true"]` in the stylesheet. Two channels again: the reader
 * who sees no red still sees the heavier word.
 */
export function StatusPill({ status, kind = "neutral", tinted = false }: StatusPillProps) {
  const bad = tinted && BAD[kind] === true;
  return (
    <span
      className="status"
      data-kind={kind}
      data-bad={bad ? "true" : undefined}
      style={bad ? { color: toneColor(TONE[kind]) } : undefined}
    >
      <span className="dot" style={{ background: toneColor(TONE[kind]) }} />
      {status}
    </span>
  );
}
