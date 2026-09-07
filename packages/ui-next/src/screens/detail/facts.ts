import type { ReactNode } from "react";
import type { K8sObject, ReplicaSetSummary } from "@srelens/core";

/**
 * One fact about a subject: what it is called, and what it says.
 *
 * DATA, not markup, and that is the seam between the two detail screens. The
 * peek reads a subject's facts down one column of label-beside-value rows in a
 * pane a few hundred pixels wide; the full tab reads the same facts across
 * three columns of label-above-value on a page. Both draw THIS list, each in
 * its own layout, and neither can see the other's.
 *
 * It replaces `FactGrid`, which was a wrapper restyling the peek's rendered
 * rows into the tab's grid — one screen's markup reshaped into another's,
 * described in terms of children it did not build, and needing a new exception
 * for every child that was not a fact row. What the two screens must never do
 * is derive a fact differently; what they were never obliged to do is draw it
 * the same way. (#331)
 *
 * `value` is a node because some facts are lists — a pod's IPs, an owner
 * chain, a selector's pairs — and a list is still one fact. Both screens seat
 * it in the value half of a name/value pair, so nothing about it assumes a
 * width.
 */
export interface DetailFact {
  /** The term, as a reader reads it. Unique within one list. */
  label: string;
  value: ReactNode;
  /** Set the value in the code face — for identifiers, not for prose. */
  mono?: boolean;
}

/**
 * Everything a kind's fact list is derived from.
 *
 * `kind` is the ROUTE's, never `object.kind`: the list was dispatched on the
 * route's kind, and reading another off the payload is a second source of
 * truth for the fact that dispatch turned on. (#331)
 *
 * `revisions` is the one piece of live data any fact list needs — a
 * Deployment's `Revision` reads "119 (6m ago)", and the age belongs to the
 * ReplicaSet carrying that number. It is fetched once by the shared layer and
 * handed to the fact list AND to the body's own revisions table, so the two
 * cannot show a rollout the other has not heard of.
 */
export interface FactInput {
  kind: string;
  object: K8sObject;
  revisions?: ReplicaSetSummary[];
}

/** A kind's fact list: pure, so both screens can be shown to derive one list. */
export type FactsFor = (input: FactInput) => DetailFact[];
