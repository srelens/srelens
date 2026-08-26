import { useState } from "react";
import {
  deleteResource,
  evictPod,
  rolloutRestart,
  runBulk,
  summarize,
  type BulkOutcome,
} from "@srelens/core";
import { ActionBar, ConfirmDialog, type ActionBarAction } from "@srelens/ui-kit";
import { useClusterGate } from "../lib/clusterMoved";
import { FailureLine, FailureWord } from "../lib/errorCopy";
import type { KindDescriptor, ListRow } from "../lib/kinds/types";

export interface ResourceBulkProps {
  /** Row keys the table's own checkbox column reports — `Table`'s
   *  `selection.selected`, verbatim. Never a second key scheme. */
  selected: Set<string>;
  /** Lowercase, plural — "pods", "deployments" — for the count and the confirm. */
  kind: string;
  descriptor: KindDescriptor;
  context: string;
  /** The rows the selection was drawn from; keys are resolved back through
   *  the same formula `Table`'s `getRowKey` uses, never parsed apart. */
  rows: ListRow[];
  /** Called once the run finishes, whatever its outcome — the caller clears
   *  the selection. */
  onDone: () => void;
}

type ActionType = "delete" | "evict" | "restart";

/** The same key `Resources.tsx` hands `Table` as `getRowKey`. Kept as one
 *  literal formula rather than two, so a namespace can never go missing on
 *  the way from a checkbox to a write. */
function keyOf(row: ListRow): string {
  return `${row.namespace ?? ""}/${row.name}`;
}

/** `namespace/name` — every target this screen writes to is qualified, so an
 *  all-namespaces view showing two `web-0`s never confuses which one a bulk
 *  action reaches. */
function rowLabel(row: ListRow): string {
  return row.namespace ? `${row.namespace}/${row.name}` : row.name;
}

const VERB: Record<ActionType, string> = { delete: "Delete", evict: "Evict", restart: "Restart" };
const PAST: Record<ActionType, string> = { delete: "deleted", evict: "evicted", restart: "restarted" };

/** What's waiting on a confirm: the action and the exact rows it was opened
 *  for — a snapshot, so a selection change under an open dialog can't retarget it. */
interface Pending {
  type: ActionType;
  rows: ListRow[];
  /**
   * The cluster the batch was opened on, captured with the rows rather than
   * read live at confirm time.
   *
   * **The snapshot above was only half a snapshot.** Since #357 a dialog covers
   * only its own tab, so the cluster rail is live behind it;
   * `setActiveCluster` switches the active cluster in place and nothing
   * remounts. `Resources.tsx` keeps this bar mounted across the switch
   * whenever the cluster being moved TO already has that kind in the row cache
   * — which is the ordinary case for a reader moving between two clusters —
   * so `pending` survived with one cluster's rows in it while `context`
   * became another's. A confirmed bulk delete then ran production's row names
   * against staging. Executed, not theorised: see this task's report.
   */
  context: string;
}

/** The finished run's per-row detail, once at least one row failed. A full
 *  success needs no report — it just closes. */
interface Report {
  type: ActionType;
  outcomes: BulkOutcome<ListRow>[];
}

function opFor(type: ActionType, context: string, kind: string) {
  return (row: ListRow) => {
    const ns = row.namespace ?? "";
    switch (type) {
      case "delete":
        return deleteResource(context, kind, row.namespace ?? null, row.name);
      case "evict":
        return evictPod(context, ns, row.name);
      case "restart":
        return rolloutRestart(context, kind, ns, row.name);
    }
  };
}

/**
 * The bulk action bar over a resource list's checkbox selection: absent when
 * nothing is selected, one confirm for the whole batch (never one per row),
 * and — because a partial failure is a fact about the cluster's actual state,
 * not a detail to swallow — a report naming exactly which rows succeeded and
 * which did not when the run comes back mixed.
 *
 * Every write goes through `row.namespace` off the resolved row, never a
 * substring of the selection key: two same-named resources in different
 * namespaces are two different targets, and only the row itself (not a
 * reparsed string) can say which is which.
 */
export function ResourceBulk({ selected, kind, descriptor, context, rows, onDone }: ResourceBulkProps) {
  const [pending, setPending] = useState<Pending | null>(null);
  const [report, setReport] = useState<Report | null>(null);
  const [busy, setBusy] = useState(false);
  /**
   * Why the run did not start. The only thing that reaches it today is the
   * cluster gate below — a partial failure is the report dialog's job, and a
   * refused row carries its own words there.
   */
  const [refused, setRefused] = useState("");

  /**
   * The divergence banner, its acknowledgement and the refusal behind it.
   *
   * `pending.context` is the cluster the batch runs against; `context` is what
   * the reader has in FOCUS, which is the only thing a rail switch may change.
   * The gate re-arms rather than only stating the divergence: this confirm's
   * whole input is one click, so asking again costs the reader a tick — and a
   * batch is the one dialog here where a silent retarget takes out forty
   * objects rather than one.
   */
  const gate = useClusterGate({
    pinned: pending?.context ?? null,
    live: context,
    verb: pending ? VERB[pending.type].toLowerCase() : "act",
  });

  // Resolved against `rows` first, and counted from the result — `Table`
  // never prunes `selection.selected` when its data changes, so a key that
  // filtered out of `rows` (or a namespace switch that made it meaningless)
  // can still be in `selected`. Counting `selected.size` instead would show
  // a number this bar cannot actually act on: the "3 selected" bar for a
  // Delete that resolves to 0 rows, confirms, and silently does nothing.
  const selectedRows = rows.filter((row) => selected.has(keyOf(row)));

  if (selectedRows.length === 0 && !pending && !report) return null;

  function open(type: ActionType) {
    setReport(null);
    setRefused("");
    gate.reset();
    // The cluster these rows are rows OF, read once, here — with them, not
    // after them. See {@link Pending.context}.
    setPending({ type, rows: selectedRows, context });
  }

  function close() {
    setPending(null);
    setReport(null);
    setRefused("");
    gate.reset();
  }

  async function confirm() {
    if (!pending) return;
    // Asked before the run starts: it is the only question on screen whose
    // answer changes what every name in the list below refers to. The run
    // still goes to `pending.context` either way — this re-arms the
    // confirmation, it does not retarget it.
    if (gate.refusal) {
      setRefused(gate.refusal);
      return;
    }
    setRefused("");
    setBusy(true);
    // `pending.context`, never the live prop. See {@link Pending.context}.
    const outcomes = await runBulk(pending.rows, opFor(pending.type, pending.context, descriptor.k8sKind));
    setBusy(false);
    const { failed } = summarize(outcomes);
    const { type } = pending;
    onDone();
    if (failed === 0) {
      close();
      return;
    }
    setPending(null);
    setReport({ type, outcomes });
  }

  const actions: ActionBarAction[] = [];
  if (descriptor.actions.restart) {
    actions.push({ id: "restart", label: "Restart rollout", danger: true, onSelect: () => open("restart") });
  }
  if (descriptor.actions.evict) {
    actions.push({ id: "evict", label: "Evict", danger: true, onSelect: () => open("evict") });
  }
  if (descriptor.actions.delete !== false) {
    actions.push({ id: "delete", label: "Delete", danger: true, onSelect: () => open("delete") });
  }

  return (
    <>
      {selectedRows.length > 0 && (
        // The table runs flush to the panel now (f088d92); this bar sits in
        // the same container, so — like the stale-rows Alert next to it —
        // it carries its own inset instead of borrowing the container's.
        <div className="mx-3 mt-3 mb-3 flex items-center gap-2">
          <span className="text-muted text-[0.8125rem]">{selectedRows.length} selected</span>
          <ActionBar actions={actions} label={`${kind} actions`} />
        </div>
      )}
      {pending && (
        <ConfirmDialog
          title={`${VERB[pending.type]} ${pending.rows.length} ${kind}?`}
          danger
          busy={busy}
          confirmLabel={VERB[pending.type]}
          onConfirm={() => void confirm()}
          onCancel={close}
          message={
            <>
              {/* First, above the list: it changes what every name in it
                  refers to, and it is the only thing here the reader does not
                  already know. */}
              {gate.alert}
              <p style={{ marginTop: 0 }}>
                This will {VERB[pending.type].toLowerCase()} {pending.rows.length} {kind}
                {pending.type === "delete" ? " — this cannot be undone" : ""}:
              </p>
              <ul>
                {pending.rows.map((row) => (
                  <li key={keyOf(row)}>
                    <code>{rowLabel(row)}</code>
                  </li>
                ))}
              </ul>
              {/* The dialog stays open on a refusal rather than closing as if
                  the run had happened, so this line is the whole of what the
                  reader is told about why. */}
              {refused && <FailureLine error={refused} className="text-sev" />}
            </>
          }
        />
      )}
      {report && (
        <ConfirmDialog
          title={`${report.outcomes.length} ${kind}: ${summarize(report.outcomes).ok} ${PAST[report.type]}, ${
            summarize(report.outcomes).failed
          } failed`}
          // Both buttons do the same thing — there is nothing left to confirm
          // or cancel, only to acknowledge — but `ConfirmDialog` always
          // renders two, and a screen reader tells them apart by accessible
          // name, not by which side of the dialog they sit on. Two "Close"s
          // read as one control repeated, not two.
          confirmLabel="Close"
          cancelLabel="Dismiss"
          onConfirm={close}
          onCancel={close}
          message={
            <ul>
              {report.outcomes.map((outcome) => (
                <li key={keyOf(outcome.item)}>
                  {outcome.status === "ok" ? (
                    <>
                      <code>{rowLabel(outcome.item)}</code>
                      {" — "}
                      {PAST[report.type]}
                    </>
                  ) : (
                    // One line per row, so the headline is what fits — a
                    // paragraph beside forty rows is a report nobody reads.
                    // The row's own name leads it, and the cluster's words
                    // are one click under it.
                    <FailureWord
                      error={outcome.error}
                      lead={
                        <>
                          <code>{rowLabel(outcome.item)}</code>
                          {" — failed: "}
                        </>
                      }
                    />
                  )}
                </li>
              ))}
            </ul>
          }
        />
      )}
    </>
  );
}
