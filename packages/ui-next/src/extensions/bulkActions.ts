import type { BulkOutcome } from "@srelens/core";

/**
 * What a bulk run over an app's resource table is about, and what it reports
 * back. The execution itself is `runBulk` in `@srelens/core` — one engine for
 * core's resource bulk bar and this one, extended in #553 with cancellation
 * and a progress list rather than forked.
 */

/** One row of an app's resource table, as a bulk selection knows it. */
export interface BulkResource {
  namespace: string;
  name: string;
}

/**
 * The key a table selects by and the label a progress list draws.
 *
 * Raw: it is escaped where it is drawn (`boundedPlainText`), never here, so a
 * key is a key and a name is never silently two different strings depending on
 * which side of the selection it came from.
 */
export function bulkResourceKey(resource: BulkResource): string {
  return resource.namespace ? `${resource.namespace}/${resource.name}` : resource.name;
}

/** A caller-supplied availability decision for one resource. */
export type BulkAvailability = (resource: BulkResource) => boolean;

export interface BulkApplicability {
  /** The subset the action will actually be run against. */
  applicable: BulkResource[];
  /** How many were selected, including the ones it does not apply to. */
  total: number;
  /** `applies to 9 of 12`, or `null` when it applies to the whole selection. */
  note: string | null;
}

/**
 * How much of a selection an action applies to.
 *
 * The note is absent when it applies to everything, because "applies to 12 of
 * 12" beside a button is noise that trains a reader to skip the line that
 * matters. It is present at zero — "applies to 0 of 2" says the host checked
 * and found none, which an empty space does not.
 */
export function bulkApplicability(
  selection: readonly BulkResource[],
  available: BulkAvailability = () => true,
): BulkApplicability {
  const applicable = selection.filter((resource) => available(resource));
  const total = selection.length;
  return {
    applicable,
    total,
    note: applicable.length === total ? null : `applies to ${applicable.length} of ${total}`,
  };
}

/** One resource whose operation failed, with the reason reported by the failing call. */
export interface BulkFailure {
  resource: string;
  reason: string;
}

/**
 * What a finished bulk run reports.
 *
 * `status` is the headline and it is never generous: anything short of every
 * selected resource being accepted is `partial` or `failed`. An operator who
 * asked for twelve reconciles and got nine has not had a success, and the
 * whole point of #553 is that the screen says so.
 */
export interface BulkActionResult {
  status: "success" | "partial" | "failed";
  /** The resources the cluster accepted, in the selection's order. */
  succeeded: string[];
  failed: BulkFailure[];
  /**
   * The resources the run never sent, because it was cancelled.
   *
   * Its own list, not folded into `failed`: the cluster was never asked about
   * these, so reporting them as refusals would be a claim about the cluster
   * that nothing backs. They still cost the run its `success`.
   */
  cancelled: string[];
}

/** What a failure with no message says, without claiming a cluster rejection. */
const NO_REASON = "The operation failed without a reason.";

/**
 * A finished run's outcomes as the result the screen reports.
 *
 * - `success` only when every selected resource was sent AND accepted.
 * - `failed` when nothing was accepted.
 * - `partial` for everything else — including a run that was cancelled with
 *   every sent item accepted.
 */
export function bulkActionResult(outcomes: readonly BulkOutcome<BulkResource>[]): BulkActionResult {
  const succeeded: string[] = [];
  const failed: BulkFailure[] = [];
  const cancelled: string[] = [];
  for (const outcome of outcomes) {
    const resource = bulkResourceKey(outcome.item);
    if (outcome.status === "ok") succeeded.push(resource);
    else if (outcome.status === "cancelled") cancelled.push(resource);
    else failed.push({ resource, reason: outcome.error?.trim() || NO_REASON });
  }
  const status =
    failed.length === 0 && cancelled.length === 0
      ? "success"
      : succeeded.length === 0
        ? "failed"
        : "partial";
  return { status, succeeded, failed, cancelled };
}
