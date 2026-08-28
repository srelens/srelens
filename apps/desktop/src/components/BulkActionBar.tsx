import { useState } from "react";
import { Trash2, RotateCw, LogOut } from "lucide-react";
import { Button } from "../ui";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { deleteResource, rolloutRestart } from "@srelens/core";
import { deletePod, evictPod } from "@srelens/core";
import { runBulk, summarize, type ActionOutcome } from "@srelens/core";
import { notify } from "@srelens/core";

export interface BulkRow {
  name: string;
  namespace?: string;
}

type Action = "delete" | "evict" | "restart";

const RESTARTABLE = new Set(["Deployment", "StatefulSet", "DaemonSet"]);

const VERB: Record<Action, string> = { delete: "Delete", evict: "Evict", restart: "Rollout-restart" };

const displayName = (r: BulkRow) => (r.namespace ? `${r.namespace}/${r.name}` : r.name);

/**
 * The bar shown above a resource table when rows are selected: offers the bulk
 * actions valid for the kind, gathers one confirm listing exactly what's
 * affected, then runs them with per-item results (partial failures reported,
 * never aborting the rest).
 */
export function BulkActionBar({
  context,
  kind,
  rows,
  onClear,
  onDone,
}: {
  context: string;
  /** The K8s kind (e.g. "Pod", "Deployment"). */
  kind: string;
  rows: BulkRow[];
  /** Clear the selection (also called after an action completes). */
  onClear: () => void;
  /** Refresh the list after a completed action. */
  onDone: () => void;
}) {
  const [pending, setPending] = useState<Action | null>(null);
  const [busy, setBusy] = useState(false);

  const canEvict = kind === "Pod";
  const canRestart = RESTARTABLE.has(kind);

  const opFor = (action: Action) => (row: BulkRow): Promise<ActionOutcome> => {
    const ns = row.namespace ?? "";
    if (action === "evict") return evictPod(context, ns, row.name);
    if (action === "restart") return rolloutRestart(context, kind, ns, row.name);
    return kind === "Pod" ? deletePod(context, ns, row.name) : deleteResource(context, kind, ns || null, row.name);
  };

  const run = async () => {
    if (!pending) return;
    setBusy(true);
    const outcomes = await runBulk(rows, opFor(pending));
    const { ok, failed } = summarize(outcomes);
    const verb = VERB[pending].toLowerCase();
    if (failed === 0) {
      notify.success(`${VERB[pending]}d ${ok} ${kind}${ok === 1 ? "" : "s"}`);
    } else {
      const detail = outcomes
        .filter((o) => o.status === "error")
        .map((o) => `${o.item.name}: ${o.error}`)
        .join("\n");
      notify.error(`${failed} of ${rows.length} failed to ${verb}`, detail);
    }
    setBusy(false);
    setPending(null);
    onClear();
    onDone();
  };

  const preview = rows.slice(0, 10);

  return (
    <div className="fl-bulk-bar" role="region" aria-label="Bulk actions">
      <span className="fl-bulk-bar__count">{rows.length} selected</span>
      <Button variant="danger" onClick={() => setPending("delete")}>
        <Trash2 data-icon="inline-start" /> Delete
      </Button>
      {canEvict && (
        <Button variant="secondary" onClick={() => setPending("evict")}>
          <LogOut data-icon="inline-start" /> Evict
        </Button>
      )}
      {canRestart && (
        <Button variant="secondary" onClick={() => setPending("restart")}>
          <RotateCw data-icon="inline-start" /> Rollout restart
        </Button>
      )}
      <Button variant="ghost" onClick={onClear}>
        Clear
      </Button>

      {pending && (
        <ConfirmDialog
          title={`${VERB[pending]} ${rows.length} ${kind}${rows.length === 1 ? "" : "s"}?`}
          danger={pending === "delete"}
          busy={busy}
          confirmLabel={VERB[pending]}
          message={
            <>
              <p style={{ marginTop: 0 }}>This will {VERB[pending].toLowerCase()}:</p>
              <ul className="fl-bulk-bar__list">
                {preview.map((r) => (
                  <li key={displayName(r)}>
                    <code>{displayName(r)}</code>
                  </li>
                ))}
              </ul>
              {rows.length > preview.length && <p>…and {rows.length - preview.length} more.</p>}
            </>
          }
          onConfirm={() => void run()}
          onCancel={() => setPending(null)}
        />
      )}
    </div>
  );
}
