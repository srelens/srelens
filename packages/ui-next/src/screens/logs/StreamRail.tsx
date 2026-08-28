import { useMemo, type ReactNode } from "react";
import { logLineHealth, tallyLogTerms, type HealthKind, type LogLine } from "@srelens/core";
import { Badge, Checkbox, Section, StatusPill } from "@srelens/ui-kit";

/**
 * §15's rail width, and this screen's alone — see `SideRail`'s note on why the
 * width is a number per screen rather than a scale. Exported so the screen and
 * this file cannot hold two different answers to the same question.
 */
export const STREAM_RAIL_WIDTH = 272;

/** One pod the stream is following, as the `Sources` section draws it. */
export interface StreamPod {
  /** The pod's name — the same string the log rows carry as their source. */
  name: string;
  /**
   * `rev 119`. Absent for a subject whose revision nothing has told us; the
   * row then omits the label rather than drawing it blank, which is the rule
   * `AboutKind` follows for a CRD field the cluster did not report.
   */
  revision?: string;
  /** Whether this pod's lines are in view. */
  checked: boolean;
  /**
   * How this pod is doing, on core's one severity vocabulary.
   *
   * **A PROP, NOT SOMETHING THIS FILE WORKS OUT.** Two reasons, and the first
   * is the plan's: every status word and tone comes from core, and a rail that
   * paired "has errors" with a colour of its own would be the eleventh
   * hand-made label/tone table found in this migration. The second is that a
   * pod's health is not a property of the window the reader happens to be
   * looking at — a pod that failed two minutes ago and has since been filtered
   * out is still the pod that failed, and §15 draws `mk3wl` green because it
   * "never fails", which is a fact about the pod rather than about nineteen
   * lines. The screen holds both the buffer and the pods, so it decides once,
   * through core, and the body's colouring and this dot cannot disagree.
   */
  tone: HealthKind;
}

export interface StreamRailProps {
  /** The pods the stream is following, in the order the screen wants them. */
  pods: readonly StreamPod[];
  /**
   * **The lines the body is drawing** — every filter applied, the pod
   * checkboxes among them, and each line's RFC3339 stamp already stripped so
   * `text` is the message.
   *
   * The stamp matters: it opens with a digit, which the tally reads as a
   * value and stops the term run on, so a buffer handed over still stamped
   * tallies to nothing at all.
   *
   * One input drives the badge and the terms both, so the two cannot be
   * counted over different sets — which is precisely what a separate `terms`
   * prop would allow.
   */
  lines: readonly LogLine[];
  /** Given the pod and the state its box is moving to, not the event. */
  onTogglePod: (pod: string, checked: boolean) => void;
}

/** `1 error`, `12 errors` — never `0 errors`; see {@link StreamRail}. */
function errorLabel(n: number): string {
  return `${n} ${n === 1 ? "error" : "errors"}`;
}

/**
 * A term or a pod name with its tone beside it, and a figure right-aligned
 * after it — the one row shape both sections of this rail are made of.
 *
 * `StatusPill` rather than a coloured word: §15 tints the term text and draws
 * a bare dot on the source rows, and each of those tells a colour-blind reader
 * nothing and a screen reader less. The pill carries the dot, the word and —
 * on a bad state — the weight, so the tone is never the only channel, and the
 * accessible name is the words already on the row rather than a second string
 * to drift from them. Same finding `ReasonRail` was built on.
 */
function ToneRow({
  slot,
  tone,
  label,
  figure,
  lead,
}: {
  slot: string;
  tone: HealthKind;
  label: string;
  figure?: string;
  lead?: ReactNode;
}) {
  return (
    <div data-slot={slot} className="flex items-center gap-2 px-0.5 py-[3px]">
      {lead}
      <span className="flex min-w-0 flex-1">
        {/* `min-w-0` as well as `truncate`: the label is itself a flex item of
            the pill, and a flex item's default `min-width: auto` makes
            `overflow: hidden` unreachable — the ellipsis never appears and the
            text runs on. A 29-character pod name is the normal case here, not
            the edge one. */}
        <StatusPill status={<span className="min-w-0 truncate">{label}</span>} kind={tone} tinted />
      </span>
      {figure !== undefined && <span className="path shrink-0 text-faint">{figure}</span>}
    </div>
  );
}

/**
 * The Logs screen's right rail (§15): who is talking, and what they keep
 * saying.
 *
 * Two sections, and **no agent prose**. §15 heads the rail with an `Agent read`
 * paragraph; the agent is a Tauri command in the desktop shell rather than a
 * capability, so `ui-next` cannot reach it and web mode has no agent at all. A
 * section scaffolded now would be a box that never fills, which is worse than
 * one that arrives with the thing it holds. The four term rows §15 tucks under
 * that paragraph are the real content of it, so they get their own heading —
 * `Top terms` — instead of being orphaned by its removal.
 *
 * **The checkboxes drive the filter.** §15's are `defaultChecked` with no
 * handler, so its own pod filter exists in the screen and nothing reaches it.
 * The state is the screen's — this component keeps none — and the box reports
 * where it is going.
 *
 * **The tally runs here, over the lines in view.** It is a pure function of
 * the buffer either way, and the buffer is a new array on every line that
 * arrives, so running it here costs exactly what running it in the screen and
 * passing the answer down would. What it buys is that the badge, the terms and
 * the rows are all counted over one set: a `terms` prop would let a caller
 * hand terms tallied before the pod filter alongside a badge counted after it,
 * and the rail would quietly contradict itself. Memoised on the `lines`
 * identity, so a re-render that is not a new line does no work.
 *
 * **The badge counts core's danger verdict, not a word list.** `logLineHealth`
 * is the one rule in srelens for how bad a log line is, and it has always read
 * `panic` as danger — a badge counting the literal words `error` and `fatal`
 * would miss every panic, which is exactly the drift a second severity table
 * causes. At zero it is not drawn: `0 errors` is a number a reader will
 * believe they have been told something by, and an absent badge is the same
 * silence the design's clean stream has.
 *
 * **Nothing is drawn for a stream that has said nothing.** An empty buffer
 * leaves no terms and no counts, and two headed but empty boxes are two things
 * the reader has to look at and dismiss. The one exception is a Sources
 * section whose view is empty *because a box was ticked off* — hiding it then
 * would take away the only control that can undo it, and the reader would be
 * stuck looking at a blank stream with no way back.
 *
 * It renders its `Section`s and nothing around them: `SideRail` drops what it
 * is handed straight into one box and `.section + .section` is what rules
 * between them, so a wrapper per child would break that adjacency. The screen
 * supplies the frame — `SideRail` at {@link STREAM_RAIL_WIDTH}, head `Stream`.
 */
export function StreamRail({ pods, lines, onTogglePod }: StreamRailProps) {
  const terms = useMemo(() => tallyLogTerms(lines), [lines]);
  const errors = useMemo(
    () => lines.reduce((n, l) => (logLineHealth(l.text) === "danger" ? n + 1 : n), 0),
    [lines],
  );

  const filtered = pods.some((pod) => !pod.checked);
  const showSources = pods.length > 0 && (lines.length > 0 || filtered);

  if (!showSources && terms.length === 0) return null;

  return (
    <>
      {showSources && (
        <Section
          smallCaps
          title={
            <span className="flex items-center justify-between gap-2">
              <span>Sources</span>
              {errors > 0 && (
                // `tracking-normal` because the small-caps heading tracks its
                // letters out and a badge is not a heading; the badge's own
                // uppercase is the badge voice and stays.
                <span className="tracking-normal">
                  <Badge tone="sev">{errorLabel(errors)}</Badge>
                </span>
              )}
            </span>
          }
        >
          {pods.map((pod) => (
            <ToneRow
              key={pod.name}
              slot="pod"
              tone={pod.tone}
              label={pod.name}
              figure={pod.revision}
              lead={
                <Checkbox
                  checked={pod.checked}
                  onChange={(next) => onTogglePod(pod.name, next)}
                  // Named by the pod beside it. The name is on the row rather
                  // than on the box because the revision is not part of what
                  // the box toggles — it toggles the pod.
                  ariaLabel={pod.name}
                />
              }
            />
          ))}
        </Section>
      )}
      {terms.length > 0 && (
        <Section smallCaps title="Top terms">
          {terms.map((term) => (
            <ToneRow
              key={term.term}
              slot="term"
              tone={term.tone}
              label={term.term}
              figure={String(term.count)}
            />
          ))}
        </Section>
      )}
    </>
  );
}
