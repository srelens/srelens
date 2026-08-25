import { useState, type ReactNode } from "react";
import {
  copyKubectlCommand,
  cronjobSetSuspend,
  cronjobTriggerNow,
  deleteResource,
  describeError,
  evictPod,
  notify,
  rolloutRestart,
  scaleResource,
  toKubectl,
  type KubectlInput,
} from "@srelens/core";
import { ConfirmDialog, KubectlPreview, TextInput, type ContextMenuItem } from "@srelens/ui-kit";
import { detailRoute } from "../lib/detailRoute";
import { FailureLine } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import { ROW_ACTION_LABEL } from "../lib/kinds/rowActions";
import type { KindActions, ListRow } from "../lib/kinds/types";
import { openTab } from "../lib/tabsStore";
import { NewForwardDialog } from "./forwards/NewForwardDialog";
import { logsRoute } from "./Logs";

export interface UseRowMenuArgs {
  /** The kubeconfig context name — what every core action call is scoped to. */
  context: string;
  /** The Kubernetes kind this list is showing, e.g. "Pod", "CronJob". */
  kind: string;
  actions: KindActions;
}

/** What a picked entry is waiting to do, once the confirm is taken. */
type Pending =
  | { type: "delete"; row: ListRow }
  | { type: "scale"; row: ListRow }
  | { type: "restart"; row: ListRow }
  | { type: "evict"; row: ListRow }
  /** `suspend: true` sets the CronJob suspended; `false` resumes it. */
  | { type: "suspend"; row: ListRow; suspend: boolean };

/** `CronJobSummary` carries `suspended`; a bare `ListRow` doesn't promise it. */
function isSuspended(row: ListRow): boolean {
  const value = (row as { suspended?: unknown }).suspended;
  return typeof value === "boolean" && value;
}

/**
 * The row menu's shell tab — a placeholder for a screen that does not exist
 * yet. Deliberately NOT `detailRoute`: designing its real route model is a
 * later step's job, not this one's.
 *
 * Neither Logs nor Port forward comes through here any more. This shape
 * carries a name and nothing else, so it cannot say which kind or namespace
 * the subject is in — which was survivable while every one of these rendered a
 * Placeholder, and stopped being so the moment those screens became real: the
 * front door opened onto an empty pane. Logs mints {@link logsRoute}; Port
 * forward opens §A.4's dialog on the row itself, through the same `dialog`
 * slot every write action here uses.
 */
const nav = (name: string, suffix: string) => `/resources/${encodeURIComponent(name)}/${suffix}`;

/**
 * The row menu and the one dialog every write action in it shares.
 *
 * One hook rather than two exports: the items close over the row that was
 * picked, the dialog renders wherever the caller puts it (outside the table,
 * so it isn't clipped by a scrolling body), and a caller wiring only one of
 * the two would get a menu that opens nothing or a dialog nothing opens.
 *
 * Every destructive pick — Scale, Restart rollout, Evict, Delete — sets
 * `pending` rather than acting; only `onConfirm` calls core. That is what
 * makes "no write happens without a confirm" true by construction rather than
 * by every call site remembering to check. Suspend/Resume also goes through
 * `pending` (it is a real write with a kubectl equivalent) but is never
 * `danger`, and Run now skips `pending` entirely — it is a call, not a
 * mutation of anything already running.
 */
export function useRowMenu({ context, kind, actions }: UseRowMenuArgs): {
  items: (row: ListRow) => ContextMenuItem[];
  dialog: ReactNode;
} {
  const [pending, setPending] = useState<Pending | null>(null);
  /**
   * The row whose `Port forward` was picked, if any.
   *
   * A slot of its own rather than a `Pending` variant: every member of that
   * union is a question with a confirm behind it, and §A.4's dialog IS the
   * confirm — it has its own fields, its own equivalent command and its own
   * error line, none of which `PendingDialog` has a shape for.
   */
  const [forwarding, setForwarding] = useState<ListRow | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [replicas, setReplicas] = useState("");

  function open(next: Pending) {
    setError("");
    if (next.type === "scale") setReplicas("");
    setPending(next);
  }

  function close() {
    setPending(null);
    setError("");
  }

  async function runNow(row: ListRow) {
    const out = await cronjobTriggerNow(context, row.namespace ?? "", row.name);
    if (out.error) {
      // The same shape core's own `reportActionError` gives classic's toasts:
      // a toast is read once, in passing, and a Rust struct in one is noise
      // nobody can act on before it fades.
      notify.error(`Failed to run ${row.name}`, describeError(out.error).detail);
      return;
    }
    notify.success(`Triggered ${row.name}`, out.jobName ? `Created job ${out.jobName}` : undefined);
  }

  async function confirm() {
    if (!pending) return;
    const { row } = pending;
    const ns = row.namespace ?? "";

    if (pending.type === "scale") {
      const n = Number(replicas);
      if (replicas.trim() === "" || !Number.isInteger(n) || n < 0) {
        setError("Enter a non-negative replica count.");
        return;
      }
    }

    setBusy(true);
    setError("");
    const result = await (async () => {
      switch (pending.type) {
        case "delete":
          return deleteResource(context, kind, row.namespace ?? null, row.name);
        case "scale":
          return scaleResource(context, kind, ns, row.name, Number(replicas));
        case "restart":
          return rolloutRestart(context, kind, ns, row.name);
        case "evict":
          return evictPod(context, ns, row.name);
        case "suspend":
          return cronjobSetSuspend(context, ns, row.name, pending.suspend);
      }
    })();
    setBusy(false);

    // A rejected or error-returning call leaves the dialog up with the
    // message in it, rather than closing as if the write had happened.
    if (result.error) {
      setError(result.error);
      return;
    }
    close();
  }

  function items(row: ListRow): ContextMenuItem[] {
    const ns = row.namespace ?? "";
    const kubectlBase: Omit<KubectlInput, "action"> = { kind, name: row.name, namespace: ns || null, context };

    const list: ContextMenuItem[] = [
      {
        label: ROW_ACTION_LABEL.openTab,
        onPick: () => openTab(detailRoute(kind, row.namespace ?? null, row.name), { clusterName: context }),
      },
    ];
    if (actions.logs) {
      list.push({
        label: ROW_ACTION_LABEL.logs,
        icon: Icons.logs,
        onPick: () => openTab(logsRoute(kind, ns, row.name), { clusterName: context }),
      });
    }
    if (actions.shell) {
      list.push({ label: ROW_ACTION_LABEL.shell, icon: Icons.terminal, onPick: () => openTab(nav(row.name, "shell"), { clusterName: context }) });
    }
    if (actions.forward) {
      list.push({
        label: ROW_ACTION_LABEL.forward,
        icon: Icons.portforwards,
        // Opens §A.4's dialog on this row rather than minting
        // `/resources/<name>/forward`, which no screen is registered for and
        // which therefore rendered a Placeholder — the menu offering a way in
        // to a dead end. The same mistake Follow logs shipped with (#346).
        onPick: () => setForwarding(row),
      });
    }
    list.push({
      label: ROW_ACTION_LABEL.edit,
      icon: Icons.edit,
      onPick: () => openTab(`/edit/${encodeURIComponent(row.name)}`, { clusterName: context }),
    });
    list.push({
      label: ROW_ACTION_LABEL.copy,
      icon: Icons.copy,
      onPick: () => void copyKubectlCommand(toKubectl({ ...kubectlBase, action: "get", output: "yaml" })),
    });

    if (actions.suspend) {
      const suspended = isSuspended(row);
      list.push({
        label: suspended ? ROW_ACTION_LABEL.resume : ROW_ACTION_LABEL.suspend,
        icon: suspended ? Icons.play : Icons.pause,
        onPick: () => open({ type: "suspend", row, suspend: !suspended }),
      });
    }
    if (actions.trigger) {
      list.push({ label: ROW_ACTION_LABEL.trigger, onPick: () => void runNow(row) });
    }

    // The destructive group. Every entry in it sets `danger` — shipping only
    // Delete as danger was a real bug caught on the tab menu (#335); the same
    // review finding applies here, in a second menu.
    const destructive: ContextMenuItem[] = [];
    if (actions.scale) {
      destructive.push({ label: ROW_ACTION_LABEL.scale, icon: Icons.scale, danger: true, onPick: () => open({ type: "scale", row }) });
    }
    if (actions.restart) {
      destructive.push({
        label: ROW_ACTION_LABEL.restart,
        icon: Icons.restart,
        danger: true,
        onPick: () => open({ type: "restart", row }),
      });
    }
    if (actions.evict) {
      destructive.push({ label: ROW_ACTION_LABEL.evict, icon: Icons.evict, danger: true, onPick: () => open({ type: "evict", row }) });
    }
    if (actions.delete !== false) {
      destructive.push({ label: ROW_ACTION_LABEL.delete, icon: Icons.trash, danger: true, onPick: () => open({ type: "delete", row }) });
    }
    if (destructive.length) list.push({ kind: "sep" }, ...destructive);

    return list;
  }

  const dialog = pending ? (
    <PendingDialog pending={pending} kind={kind} context={context} busy={busy} error={error} replicas={replicas} onReplicasChange={setReplicas} onConfirm={() => void confirm()} onCancel={close} />
  ) : forwarding ? (
    // The row's own kind and identity, prefilled. A list row knows no ports,
    // so the remote one is the reader's to name — and the dialog stays fully
    // editable either way: a prefill is a starting point.
    <NewForwardDialog
      context={context}
      namespace={forwarding.namespace}
      target={{ kind, name: forwarding.name }}
      onClose={() => setForwarding(null)}
    />
  ) : null;

  return { items, dialog };
}

// `suspend` isn't here: its title depends on direction (Suspend vs. Resume),
// decided per-instance in `PendingDialog` rather than by a fixed lookup.
const TITLES: Record<Exclude<Pending["type"], "suspend">, (kind: string) => string> = {
  delete: (kind) => `Delete ${kind}?`,
  scale: (kind) => `Scale ${kind}`,
  restart: (kind) => `Restart ${kind}`,
  evict: () => "Evict pod?",
};

function messageFor(pending: Pending): ReactNode {
  const { row } = pending;
  const where = row.namespace ? (
    <>
      {" "}
      in <code>{row.namespace}</code>
    </>
  ) : null;
  switch (pending.type) {
    case "delete":
      return (
        <>
          Delete <code>{row.name}</code>
          {where}? This cannot be undone.
        </>
      );
    case "scale":
      return (
        <>
          Set the replica count for <code>{row.name}</code>
          {where}.
        </>
      );
    case "restart":
      return (
        <>
          Trigger a rolling restart of <code>{row.name}</code>
          {where}? This reschedules all of its pods.
        </>
      );
    case "evict":
      return (
        <>
          Gracefully evict <code>{row.name}</code>
          {where} (respects disruption budgets)?
        </>
      );
    case "suspend":
      return (
        <>
          {pending.suspend ? "Suspend" : "Resume"} <code>{row.name}</code>
          {where}?{" "}
          {pending.suspend
            ? "Scheduled runs will be paused; already-running jobs are unaffected."
            : "Scheduled runs will resume."}
        </>
      );
  }
}

function kubectlFor(pending: Pending, kind: string, context: string, replicas: string): { command?: string; note?: string } {
  const { row } = pending;
  const namespace = row.namespace ?? null;
  switch (pending.type) {
    case "delete":
      return { command: toKubectl({ action: "delete", kind, name: row.name, namespace, context }) };
    case "restart":
      return { command: toKubectl({ action: "rollout-restart", kind, name: row.name, namespace, context }) };
    case "evict":
      return {
        note: "No single-line kubectl equivalent — eviction uses the pod's /eviction subresource, which respects PodDisruptionBudgets (a plain delete does not).",
      };
    case "suspend":
      return {
        command: toKubectl({
          action: pending.suspend ? "cronjob-suspend" : "cronjob-resume",
          kind,
          name: row.name,
          namespace,
          context,
        }),
      };
    case "scale": {
      const n = Number(replicas);
      if (replicas.trim() === "" || !Number.isInteger(n) || n < 0) return {};
      return { command: toKubectl({ action: "scale", kind, name: row.name, namespace, context, replicas: n }) };
    }
  }
}

function PendingDialog({
  pending,
  kind,
  context,
  busy,
  error,
  replicas,
  onReplicasChange,
  onConfirm,
  onCancel,
}: {
  pending: Pending;
  kind: string;
  context: string;
  busy: boolean;
  error: string;
  replicas: string;
  onReplicasChange: (value: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const title = pending.type === "suspend" ? (pending.suspend ? "Suspend CronJob" : "Resume CronJob") : TITLES[pending.type](kind);
  const confirmLabel =
    pending.type === "delete"
      ? "Delete"
      : pending.type === "scale"
        ? "Scale"
        : pending.type === "restart"
          ? "Restart"
          : pending.type === "evict"
            ? "Evict"
            : pending.suspend
              ? "Suspend"
              : "Resume";
  const { command, note } = kubectlFor(pending, kind, context, replicas);

  return (
    <ConfirmDialog
      title={title}
      // Suspend/resume is a write with no danger styling (R-4 of the CronJob
      // ruling); every other entry that reaches this dialog is destructive.
      danger={pending.type !== "suspend"}
      busy={busy}
      confirmLabel={confirmLabel}
      onConfirm={onConfirm}
      onCancel={onCancel}
      message={
        <>
          <p style={{ marginTop: 0 }}>{messageFor(pending)}</p>
          {pending.type === "scale" && (
            <TextInput
              value={replicas}
              onValueChange={onReplicasChange}
              placeholder="replicas"
              aria-label="Replica count"
              invalid={Boolean(error)}
              autoFocus
            />
          )}
          <KubectlPreview command={command} note={note} onCopy={command ? () => void copyKubectlCommand(command) : undefined} />
          {/* The dialog stays open on a refusal rather than closing as if the
              write had happened, so this line is the whole of what the reader
              is told about why. A validation message this component wrote
              itself ("Enter a non-negative replica count.") matches no
              classification and arrives exactly as written. */}
          {error && <FailureLine error={error} className="text-sev" />}
        </>
      }
    />
  );
}
