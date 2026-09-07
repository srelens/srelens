import { eventVerdict } from "@srelens/core";
import { Button, StatusPill } from "@srelens/ui-kit";
import type { EventRow } from "../../lib/kinds/events";

export interface ReasonRailProps {
  /**
   * The events to rank: the screen's rows after the namespace selection and
   * the type control, but not after its search — see `Events.tsx`, which owns
   * that choice and the reasoning for it. Not the loaded set either; a rail
   * counting events from namespaces the reader is not looking at would
   * disagree with the table beside it.
   */
  rows: readonly EventRow[];
  /** What to do with a reason the reader clicked. The screen decides. */
  onPick: (reason: string) => void;
}

/** One line of the rail: a reason, how many events carry it, and its tone. */
interface Tally {
  reason: string;
  /**
   * HOW MANY EVENTS, NOT HOW MANY TIMES THEY FIRED. See {@link tally}.
   */
  events: number;
  /** The `type` of the first event carrying this reason — §8's own rule. */
  type: string;
}

/**
 * Every distinct reason in `rows`, most events first.
 *
 * **The count is a number of events, never the sum of their `count` fields**,
 * and the difference is the whole reason the rail exists. `count` is how many
 * times ONE event has fired: a probe failing every two seconds reaches 400
 * without anything new having happened. Summed, that single report outranks
 * four distinct failures that each happened once, and the rail — whose job is
 * to say how many KINDS of thing are going wrong — would answer a question
 * nobody asked, in a way that looks plausible. The table's Count column is
 * where the repeat count belongs, and it already shows it per event.
 *
 * The tone is the first event's, per §8. A reason is not reliably one type:
 * `Killing` is Normal on a rollout and a Warning on an eviction, and there is
 * no honest single tone for a mixed group — so the rail follows the design
 * rather than inventing a rule (say, "worst wins") the reader has no way to
 * know about. The type is carried, not the tone: `eventVerdict` is asked for
 * the tone at the point of drawing, so this file pairs no word with a colour.
 *
 * Insertion order is the tie-break, because `Map` keeps it and `sort` is
 * stable — two reasons with the same number of events stay in the order the
 * table has them, rather than swapping places on every re-render.
 */
function tally(rows: readonly EventRow[]): Tally[] {
  const byReason = new Map<string, Tally>();
  for (const row of rows) {
    const seen = byReason.get(row.reason);
    if (seen) seen.events += 1;
    else byReason.set(row.reason, { reason: row.reason, events: 1, type: row.type });
  }
  return [...byReason.values()].sort((a, b) => b.events - a.events);
}

/**
 * The Events screen's right rail (§8): what is actually going wrong, as a
 * ranked list of reasons.
 *
 * A reader scanning forty rows of a busy namespace cannot see that thirty of
 * them say `Pulled` and three say `FailedScheduling`. This says it in three
 * lines, and each line is a filter: clicking one puts that reason in the
 * screen's search box, which is how the rail turns a summary into a way in.
 *
 * **The design's rows carry a status dot with an empty label** — a bare
 * coloured dot, and the one thing `StatusPill` exists to prevent. Colour alone
 * is nothing to a colour-blind reader and silence to a screen reader, so the
 * dot here labels itself with the reason, and the count beside it completes
 * the button's name: "Unhealthy 3". No second string to drift, because the
 * name is the words already on screen.
 *
 * Nothing is drawn for an empty set. §8 leaves the rail blank when the filter
 * has emptied the table, and a heading over nothing, or an empty framed box,
 * would each be a thing the reader has to look at and dismiss.
 */
export function ReasonRail({ rows, onPick }: ReasonRailProps) {
  const reasons = tally(rows);
  if (reasons.length === 0) return null;

  return (
    // §8's flat list, at its own 6px inset. Not a `Section`: the rail has no
    // heading beyond the pane head that `SideRail` already draws, and a
    // section's inset is meant for prose blocks rather than a run of rows.
    <div className="p-1.5">
      {reasons.map(({ reason, events, type }) => {
        const { health, bad } = eventVerdict(type);
        return (
          <Button
            key={reason}
            type="button"
            variant="ghost"
            // `.ns-row` is the design's flat row; it follows `.btn` in the
            // stylesheet, so the row's own padding, width and alignment win
            // over the button's while `ghost` keeps the border off. `.btn`'s
            // weight is undone here rather than inherited: a row where every
            // reason is semi-bold has no weight left to spend on the warning,
            // which `.status[data-bad]` is what puts it back on.
            className="ns-row rounded font-normal"
            // Deliberately no `aria-label`: the accessible name is computed
            // from the words in the row, so it cannot drift from them.
            title={reason}
            onClick={() => onPick(reason)}
          >
            <span className="flex min-w-0 flex-1">
              <StatusPill
                status={<span className="truncate">{reason}</span>}
                kind={health}
                tinted={bad}
              />
            </span>
            <span className="path text-faint">{events}</span>
          </Button>
        );
      })}
    </div>
  );
}
