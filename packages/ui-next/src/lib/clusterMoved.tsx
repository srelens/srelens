import { useCallback, useState, type ReactNode } from "react";
import { Alert, Checkbox } from "@srelens/ui-kit";

/**
 * What a dialog says, and asks again, when the cluster rail moves out from
 * under it.
 *
 * **Why this exists at all.** Until #357 a dialog was window-modal: its
 * overlay covered the cluster rail, so the reader could not switch clusters
 * while one was open, and every screen that read the live context inside a
 * confirm was accidentally right. b6dd228 scopes a dialog to its own tab, so
 * the rail, the tab strip and the status bar are all live behind one — and
 * `setActiveCluster` switches the active cluster in place, globally. No screen
 * carries `key={name}`, so nothing remounts: a dialog opened on one cluster
 * stays open, still naming that cluster's object, while every prop under it
 * quietly becomes another cluster's.
 *
 * `Helm.tsx` solved this first (7e50ea0) for the four helm operations. This is
 * the same shape, factored out because three more screens need it and three
 * hand-written copies of a sentence about a destructive write is how two of
 * them end up saying different things about the same event.
 *
 * **The rule, in three parts.**
 *
 * 1. The cluster is captured when the reader OPENS the dialog — not when the
 *    dialog mounts. Helm's case proved mount-time is too late: a dialog that
 *    opens after an async round trip mounts under whatever cluster is in focus
 *    by the time the answer lands.
 * 2. The pinned cluster is what the action runs against. Following the rail is
 *    the one outcome that must never happen — an operation that retargets
 *    itself silently is the worst thing on offer here.
 * 3. The divergence is STATED. Nothing is closed and nothing the reader typed
 *    is thrown away, which is `StaleSelectionAlert`'s documented rule and
 *    `Overview`'s own stale-reading banner.
 *
 * Where this parts company with Helm is the fourth part: {@link useClusterGate}
 * also RE-ARMS the confirmation. Helm justified leaving its gate armed by
 * saying that closing would discard a typed values body — an upgrade's
 * argument, applied to an uninstall's decision. The dialogs this module serves
 * are confirms whose whole input is a click (or, for Scale, one number that is
 * kept), so asking again costs the reader one tick and nothing else. A reader
 * who moved the rail BECAUSE they realised they were on the wrong cluster, and
 * then reached for the button they already meant to press, is the case a
 * banner alone does not catch.
 */
export function ClusterMovedAlert({
  pinned,
  live,
  children,
}: {
  /** The cluster the action was opened against, and will run against. */
  pinned: string;
  /** The cluster the reader has in FOCUS now — the only thing that moved. */
  live: string;
  /** The acknowledgement control, or a closing sentence, under the words. */
  children?: ReactNode;
}) {
  return (
    // Toned `warn`, so the kit gives it `role="status"` — a polite live region,
    // which is what announces a fact that appears while the dialog is already
    // open. A dialog's own destructive alert stays the assertive one, and this
    // goes above it: it changes what every name below it refers to.
    <Alert tone="warn" title={`This still runs against ${pinned}, not ${live}`}>
      {/* Deliberately number-neutral: one row, one node and forty rows all
          reach this sentence, and a singular one would have to be written
          three times to stay true. */}
      The cluster in focus changed while this was open. srelens is still
      talking to {pinned}, and what is named here belongs to that cluster — the
      same names on {live} are different objects.
      {children}
    </Alert>
  );
}

/** Why the write did not go ahead. One sentence, shared by every caller. */
export function clusterMovedRefusal(pinned: string, live: string): string {
  return `This runs on ${pinned}, not ${live}. Confirm the cluster above, or cancel.`;
}

export interface ClusterGate {
  /** Has the rail moved out from under the open dialog? */
  moved: boolean;
  /** What the dialog renders about it, or `null` when there is nothing to say. */
  alert: ReactNode;
  /**
   * Why the action must not run yet, or `null` when it may.
   *
   * Checked by the confirm handler and shown as the dialog's own error line —
   * the same path a validation message this side wrote takes today. It is
   * `null` in the ordinary case, so a dialog nobody moved the rail under
   * behaves exactly as it did before.
   */
  refusal: string | null;
  /** Forget any acknowledgement. Called when a dialog opens and when it closes. */
  reset: () => void;
}

/**
 * The divergence banner, its acknowledgement, and the refusal that keeps the
 * two honest — see this module's note.
 *
 * `pinned` is `null` while nothing is open, which is also how a caller says
 * "there is nothing to compare": `moved` is false, the alert is nothing, and
 * the refusal is nothing.
 *
 * The acknowledgement is stored as the cluster it was given AGAINST, not as a
 * boolean. A reader who ticks the box on `stage-eu` and then moves the rail
 * on to `dev` has confirmed a divergence that no longer exists, and the gate
 * re-arms itself rather than carrying their answer over to a question they
 * were never asked.
 */
export function useClusterGate({
  pinned,
  live,
  verb,
}: {
  pinned: string | null;
  live: string;
  /** What the button does, for the acknowledgement's own words: "delete", "drain". */
  verb: string;
}): ClusterGate {
  const [acknowledgedFor, setAcknowledgedFor] = useState<string | null>(null);
  const reset = useCallback(() => setAcknowledgedFor(null), []);

  const moved = pinned !== null && pinned !== live;
  const acknowledged = moved && acknowledgedFor === live;

  return {
    moved,
    alert:
      pinned !== null && moved ? (
        <ClusterMovedAlert pinned={pinned} live={live}>
          <Checkbox
            className="mt-2"
            checked={acknowledged}
            onChange={(next) => setAcknowledgedFor(next ? live : null)}
            label={`Yes, still ${verb} on ${pinned}.`}
          />
        </ClusterMovedAlert>
      ) : null,
    refusal: moved && !acknowledged ? clusterMovedRefusal(pinned, live) : null,
    reset,
  };
}
