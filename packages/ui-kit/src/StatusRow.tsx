import type { ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";
import { StatusPill, type StatusKind } from "./StatusPill";

export interface StatusRowProps {
  /**
   * The kind's own status word — "Degraded", "CrashLoopBackOff",
   * "Progressing". Whatever the resource's status derivation called the state
   * it is in; this component never invents or rewords one.
   */
  status: ReactNode;
  /**
   * The verdict's severity, on the same five names the pills use. There is
   * deliberately **no `tone` prop**: `Tone` is the palette's vocabulary, and a
   * row that took one would let a caller hand-pair a word with a colour —
   * a red word beside an amber dot, each side certain it was right. The kind
   * comes off the status derivation with the word, and the pill owns the
   * mapping to a colour. (#331)
   */
  kind: StatusKind;
  /**
   * Whether the subject needs attention. **Data, not a derivation**: badness
   * is not a function of the tone, which is why the status derivation
   * enumerates the (tone, dot) pairs rather than computing them. A running
   * Job is amber and not flagged; a Pending pod is amber and is. A row that
   * tinted every amber word would shout at the first and be right about the
   * second only by accident.
   *
   * Required rather than defaulted: a default of `false` quietly draws a
   * crash-looping pod as plain grey text for any caller who forgets, and a
   * default of `true` shouts at a healthy one. The caller has the answer.
   */
  flagged: boolean;
  /** What the row is about. Takes the slack between the verdict and the facts. */
  name: ReactNode;
  /**
   * Right-aligned trailing facts, drawn in the order given — the design's
   * namespace and ready ratio. The order is the caller's column order, so
   * nothing here sorts or reverses it.
   */
  facts?: ReactNode[];
  /** Opens what the row names. Without it the row is not a target at all. */
  onActivate?: () => void;
  className?: string;
}

/**
 * One row of a list that is not a table: a toned dot, the status word in that
 * tone, the name, then right-aligned trailing facts.
 *
 * The cluster overview's `NOT READY` section is the shape this exists for —
 * unhealthy workloads and pods mixed in one list, ordered by severity rather
 * than by kind. `Table` is too much for it (there is no header, no sort and no
 * selection to hang off one) and `KV` is the wrong shape (a name and its
 * value, not a verdict, a subject and its facts).
 *
 * **The left-hand half is {@link StatusPill}, not a redrawing of it.** The
 * design's most consistent rule is asymmetric: the dot is always tone-coloured,
 * while the word is coloured and given more weight only when the state is bad —
 * a healthy value is a coloured dot beside plain grey text. `StatusPill` calls
 * that `tinted` and already implements both channels, the colour off the tone
 * and the weight off `.status[data-bad="true"]`. A second implementation of the
 * same asymmetry is precisely the duplication this project has spent the most
 * time removing; eight hand-paired label/tone tables have been deleted so far.
 * So the row passes the verdict through and draws no dot of its own.
 *
 * **The whole row is one activation target, and its accessible name is its own
 * text.** An earlier screen's design drew a status dot with an empty label
 * beside a separately-linked name: that announces itself as a link called
 * "checkout-api" with the verdict nowhere in the name, so a reader who cannot
 * see the dot never learns the pod is degraded. Here the button contains the
 * word, the name and the facts, so the name is computed from what is on
 * screen — one string, and one that cannot drift from the row it labels.
 *
 * Without `onActivate` it is a plain `div`: a row that looks pressable and does
 * nothing is worse than one that never offered.
 */
export function StatusRow({ status, kind, flagged, name, facts, onActivate, className }: StatusRowProps) {
  const body = (
    <>
      {/* The verdict keeps a minimum width so the names line up down the
          list, the way `LogLine`'s gutters keep a stream's messages in line.
          A minimum and not a fixed size: `CrashLoopBackOff` is longer than the
          column and pushes rather than being cut in half. */}
      <span className="status-row-verdict">
        <StatusPill status={status} kind={kind} tinted={flagged} />
      </span>
      <span className="status-row-name">{name}</span>
      {filled(facts) && (
        <span className="status-row-facts">
          {facts?.map((fact, i) => (
            // Indexed because the facts are a positional list — the namespace
            // slot and the ratio slot — with no identity of their own, and
            // nothing here reorders or removes one.
            <span key={i} className="status-row-fact">
              {fact}
            </span>
          ))}
        </span>
      )}
    </>
  );

  if (!onActivate) return <div className={cx("status-row", className)}>{body}</div>;

  return (
    // Explicitly `type="button"`: these stand inside whatever the screen puts
    // them in, and a button with no type submits a surrounding form.
    <button type="button" className={cx("status-row", className)} onClick={onActivate}>
      {body}
    </button>
  );
}
