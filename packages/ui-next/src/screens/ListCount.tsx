import { groupNumber } from "../lib/numbers";

export interface ListCountProps {
  /** The rows in the view: what the list holds for the namespaces selected. */
  total: number;
  /** The rows on screen: `total`, less whatever the filter box hid. */
  shown: number;
  /**
   * The list stopped at the backend's row cap, so `total` is a floor and not
   * the number there are. Said with a `+`, never passed off as a count.
   */
  truncated?: boolean;
  /** Lowercase and plural — "pods" — for the sentence a screen reader gets. */
  noun: string;
}

/**
 * How many rows a list holds, in its header — and, while a filter is on, how
 * many of them it is showing (#402).
 *
 * A list said nothing about its own size: the header read `Pods` and no more,
 * and typing in the filter box shrank the table with no word on how much had
 * been hidden or how much there was to begin with.
 *
 * **One element, in one of two states.** `426 items` with nothing narrowed,
 * and `3 / 426` — the same slot, not a second line — while the filter hides
 * rows. It switches on the same condition `AppLog` uses for its own
 * filtered-of-total line: only when fewer are shown than there are.
 *
 * **Both numbers are the table's own**, taken off the render that draws it.
 * `total` is the view's — narrowed by the namespace selection, so it moves
 * with the picker — and is not a cluster-wide figure: a number that disagreed
 * with the rows under it would be worse than a narrower one. A list is a
 * moving target, and a count fetched separately would be a second answer to
 * the question the table is already answering.
 *
 * **The caller decides WHETHER there is a count; this only says it.** A list
 * that is still loading, or that could not be read, has no number, and is to
 * be given no `ListCount` at all — never one reading `0 items`. A refused list
 * and an empty cluster are the same picture and opposite facts, and zero is
 * the one a reader believes. `0 items` is for a list that answered with none.
 *
 * Grouped with the app's thin space (`5 416`), like every other figure here.
 *
 * The visible text is terse by design, so the same fact is given in words to
 * a screen reader, in a polite live region: the filter narrows the table on
 * every keystroke, and "3 of 426 pods shown" is the only way a reader who
 * cannot see the table shrink learns that it did.
 */
export function ListCount({ total, shown, truncated = false, noun }: ListCountProps) {
  const all = `${groupNumber(total)}${truncated ? "+" : ""}`;
  const narrowed = shown < total;
  const visible = narrowed ? `${groupNumber(shown)} / ${all}` : `${all} ${total === 1 && !truncated ? "item" : "items"}`;
  const spoken = narrowed
    ? `${groupNumber(shown)} of ${truncated ? "more than " : ""}${groupNumber(total)} ${noun} shown`
    : `${truncated ? "More than " : ""}${groupNumber(total)} ${noun}`;
  return (
    <span data-slot="list-count" role="status" aria-live="polite" className="num mr-1 shrink-0 text-[0.75rem] text-muted">
      <span aria-hidden="true">{visible}</span>
      <span className="sr-only">{spoken}</span>
    </span>
  );
}
