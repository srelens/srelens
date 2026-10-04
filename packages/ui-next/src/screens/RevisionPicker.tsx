import { useEffect, useState } from "react";
import { listReplicaSets, type ReplicaSetSummary } from "@srelens/core";
import { Combobox } from "@srelens/ui-kit";
import { FailureLine } from "../lib/errorCopy";

/** One revision as a reader tells it from the others (#389): its number, the
 *  images it ran, how long ago, and the change-cause it was recorded with. */
export function revisionLabel(rs: ReplicaSetSummary): string {
  const parts = [`rev ${rs.revision}`];
  if (rs.images?.length) parts.push(rs.images.join(", "));
  parts.push(`${rs.age} ago`);
  if (rs.changeCause) parts.push(`“${rs.changeCause}”`);
  return parts.join(" · ");
}

type Revisions =
  | { status: "loading" }
  | { status: "error"; error: string }
  | { status: "ready"; current?: ReplicaSetSummary; earlier: ReplicaSetSummary[] };

/**
 * The revisions a Deployment can be rolled back to, read when the dialog
 * opens (#389). Newest first — the backend's order. The one whose template the
 * Deployment runs (`currentTemplate`, judged by the backend against the
 * Deployment itself, not by number — PR #810 review) is shown, never offered:
 * rolling back to it is a no-op the capability refuses. The newest of the rest
 * is chosen to start with, which is `kubectl rollout undo`'s own default. When
 * none is current — a template change the controller has not caught up with —
 * every revision is offered.
 *
 * A searchable `Combobox`, as every cluster-supplied list is (AGENTS.md).
 *
 * Read on the cluster the dialog was opened ON (`context` is the pinned one,
 * not the live rail), like every other name in the dialog.
 */
export function RevisionPicker({
  context,
  namespace,
  name,
  value,
  onChange,
}: {
  context: string;
  namespace: string;
  name: string;
  value: string;
  onChange: (revision: string) => void;
}) {
  const [revisions, setRevisions] = useState<Revisions>({ status: "loading" });
  useEffect(() => {
    let live = true;
    void listReplicaSets(context, namespace, name).then((out) => {
      if (!live) return;
      if (out.error) {
        setRevisions({ status: "error", error: out.error });
        return;
      }
      // A ReplicaSet with no revision annotation is no revision to go back to.
      const rows = (out.replicasets ?? []).filter((rs) => rs.revision !== "");
      const current = rows.find((rs) => rs.currentTemplate);
      const earlier = rows.filter((rs) => !rs.currentTemplate);
      setRevisions({ status: "ready", current, earlier });
      if (earlier[0]) onChange(earlier[0].revision);
    });
    return () => {
      live = false;
    };
  }, [context, namespace, name, onChange]);

  if (revisions.status === "loading") return <p className="text-faint">Reading revisions…</p>;
  if (revisions.status === "error") return <FailureLine error={revisions.error} className="text-sev" />;
  return (
    <>
      {revisions.current && (
        <p>
          Current: <span className="font-mono">{revisionLabel(revisions.current)}</span>
        </p>
      )}
      {revisions.earlier.length === 0 ? (
        <p>This Deployment has no earlier revision to roll back to.</p>
      ) : (
        <Combobox
          value={value}
          onValueChange={onChange}
          options={revisions.earlier.map((rs) => ({ value: rs.revision, label: revisionLabel(rs) }))}
          ariaLabel="Revision"
          searchPlaceholder="Search revisions…"
        />
      )}
    </>
  );
}
