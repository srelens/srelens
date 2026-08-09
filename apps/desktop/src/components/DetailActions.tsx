import React, { useState } from "react";
import {
  ArrowLeftRight,
  Bot,
  LogOut,
  Logs,
  Pause,
  Pencil,
  Play,
  RotateCw,
  Scaling,
  SquareTerminal,
  Trash2,
  Zap,
} from "lucide-react";
import { deletePod, evictPod, type PodSummary } from "../lib/workloads";
import {
  deleteResource,
  scaleResource,
  rolloutRestart,
  cronjobSetSuspend,
  cronjobTriggerNow,
  debugPod,
} from "../lib/actions";
import { notify } from "../lib/notify";
import { useAccess, rbac, kindToResource, denyReason, reportActionError, type AccessCheck } from "../lib/access";
import { IconButton, ConfirmDialog, TextInput, KubectlPreview } from "../ui";
import { ForwardDialog } from "./ForwardDialog";
import { CopyAsKubectlButton } from "./CopyAsKubectlButton";
import { toKubectl } from "../lib/kubectlMapper";
import { copyKubectlCommand } from "../lib/copyKubectl";
import type { AssistantContext } from "./AssistantDrawer";

type Opener = (s: { context: string; namespace: string; pod: string; container?: string }) => void;
/** Opens the assistant drawer with the current resource attached as context. */
export type AskAssistant = (s: AssistantContext) => void;

const SCALABLE = ["Deployment", "StatefulSet", "ReplicaSet"];
const RESTARTABLE = ["Deployment", "StatefulSet", "DaemonSet"];
// Workloads whose pods are reachable via spec.selector.matchLabels.
const LOGGABLE = ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"];

/** Pod header actions: Logs, Shell, Delete (with a delete confirm). */
export function PodActions({
  context,
  pod,
  onDeleted,
  onOpenTerminal,
  onOpenLogs,
  onEdit,
  onAskAssistant,
}: {
  context: string;
  pod: PodSummary;
  onDeleted?: () => void;
  onOpenTerminal?: Opener;
  onOpenLogs?: Opener;
  onEdit?: () => void;
  onAskAssistant?: AskAssistant;
}) {
  const [dialog, setDialog] = useState<"delete" | "evict" | "forward" | "debug" | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [debugImage, setDebugImage] = useState("busybox");
  const [debugTarget, setDebugTarget] = useState("");

  const deleteCheck = rbac.deletePod(pod.namespace);
  const evictCheck = rbac.evictPod(pod.namespace);
  const editCheck = rbac.edit("", "pods", pod.namespace);
  const access = useAccess(context, [deleteCheck, evictCheck, editCheck]);

  const target = { context, namespace: pod.namespace, pod: pod.name };

  async function doDelete() {
    setBusy(true);
    setError("");
    const out = await deletePod(context, pod.namespace, pod.name);
    setBusy(false);
    if (out.error) {
      setError(out.error);
      reportActionError(context, `Failed to delete ${pod.name}`, out.error);
      return;
    }
    setDialog(null);
    notify.success(`Deleted pod ${pod.name}`);
    onDeleted?.();
  }

  async function doDebug() {
    setBusy(true);
    setError("");
    const out = await debugPod(context, pod.namespace, pod.name, debugImage.trim() || "busybox", debugTarget.trim() || null);
    setBusy(false);
    if (out.error || !out.container) {
      setError(out.error ?? "no container returned");
      reportActionError(context, `Failed to debug ${pod.name}`, out.error ?? "");
      return;
    }
    setDialog(null);
    notify.success(`Debug container ${out.container} added to ${pod.name}`);
    onOpenTerminal?.({ context, namespace: pod.namespace, pod: pod.name, container: out.container });
  }

  async function doEvict() {
    setBusy(true);
    setError("");
    const out = await evictPod(context, pod.namespace, pod.name);
    setBusy(false);
    if (out.error) {
      setError(out.error);
      reportActionError(context, `Failed to evict ${pod.name}`, out.error);
      return;
    }
    setDialog(null);
    notify.success(`Evicted pod ${pod.name}`);
    onDeleted?.();
  }

  const deleteCmd = toKubectl({ action: "delete", kind: "Pod", namespace: pod.namespace, name: pod.name, context });

  return (
    <>
      <IconButton icon={Logs} label="Logs" onClick={() => onOpenLogs?.(target)} />
      <IconButton icon={SquareTerminal} label="Shell" onClick={() => onOpenTerminal?.(target)} />
      <CopyAsKubectlButton kind="Pod" name={pod.name} namespace={pod.namespace} context={context} />
      <IconButton
        icon={Zap}
        label="Debug"
        title="Attach an ephemeral debug container"
        onClick={() => {
          setError("");
          setDialog("debug");
        }}
      />
      {onEdit && (
        <IconButton
          icon={Pencil}
          label="Edit"
          disabled={!access.allowed(editCheck)}
          title={denyReason(access, editCheck)}
          onClick={onEdit}
        />
      )}
      <IconButton icon={ArrowLeftRight} label="Forward" onClick={() => setDialog("forward")} />
      <IconButton
        icon={LogOut}
        label="Evict"
        disabled={!access.allowed(evictCheck)}
        title={denyReason(access, evictCheck)}
        onClick={() => {
          setError("");
          setDialog("evict");
        }}
      />
      <IconButton
        icon={Trash2}
        label="Delete"
        danger
        disabled={!access.allowed(deleteCheck)}
        title={denyReason(access, deleteCheck)}
        onClick={() => {
          setError("");
          setDialog("delete");
        }}
      />
      {onAskAssistant && (
        <IconButton
          icon={Bot}
          label="Ask assistant"
          title="Ask the assistant about this pod"
          onClick={() => onAskAssistant({ context, namespace: pod.namespace, kind: "Pod", name: pod.name })}
        />
      )}
      {dialog === "forward" && (
        <ForwardDialog
          context={context}
          namespace={pod.namespace}
          kind="Pod"
          name={pod.name}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog === "delete" && (
        <ConfirmDialog
          title="Delete pod?"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Delete <code>{pod.name}</code> in <code>{pod.namespace}</code>? This cannot be
                undone.
              </p>
              <KubectlPreview command={deleteCmd} onCopy={() => void copyKubectlCommand(deleteCmd)} />
              {error && <p className="text-destructive">Error: {error}</p>}
            </>
          }
          confirmLabel="Delete"
          danger
          busy={busy}
          onConfirm={() => void doDelete()}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog === "debug" && (
        <ConfirmDialog
          title="Attach debug container?"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Add an ephemeral debug container to <code>{pod.name}</code> and open a shell into
                it. Ephemeral containers cannot be removed until the pod restarts.
              </p>
              <label className="fl-debug-field">
                <span>Image</span>
                <TextInput value={debugImage} onValueChange={setDebugImage} placeholder="busybox" aria-label="Debug image" />
              </label>
              <label className="fl-debug-field">
                <span>Share process namespace with (optional)</span>
                <TextInput
                  value={debugTarget}
                  onValueChange={setDebugTarget}
                  placeholder="target container name"
                  aria-label="Target container"
                />
              </label>
              {error && <p className="text-destructive">Error: {error}</p>}
            </>
          }
          confirmLabel="Attach & open shell"
          danger
          busy={busy}
          onConfirm={() => void doDebug()}
          onCancel={() => setDialog(null)}
        />
      )}
      {dialog === "evict" && (
        <ConfirmDialog
          title="Evict pod?"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Gracefully evict <code>{pod.name}</code> in <code>{pod.namespace}</code> (respects
                disruption budgets)?
              </p>
              <KubectlPreview note="No single-line kubectl equivalent — eviction uses the pod's /eviction subresource, which respects PodDisruptionBudgets (a plain delete does not)." />
              {error && <p className="text-destructive">Error: {error}</p>}
            </>
          }
          confirmLabel="Evict"
          danger
          busy={busy}
          onConfirm={() => void doEvict()}
          onCancel={() => setDialog(null)}
        />
      )}
    </>
  );
}

/** Service header action: open a port-forward to the service. */
export function ServiceForwardAction({
  context,
  namespace,
  name,
}: {
  context: string;
  namespace: string | null;
  name: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <IconButton icon={ArrowLeftRight} label="Forward" onClick={() => setOpen(true)} />
      {open && (
        <ForwardDialog
          context={context}
          namespace={namespace ?? ""}
          kind="Service"
          name={name}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

/** Non-pod header actions: Scale (workloads), Restart, Delete — each gated. */
export function ResourceActions({
  context,
  kind,
  namespace,
  name,
  cronjobSuspended,
  onDeleted,
  onChanged,
  onOpenLogs,
  onEdit,
  onAskAssistant,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
  /** For CronJob details: current suspend state, to label Suspend/Resume. */
  cronjobSuspended?: boolean;
  onDeleted: () => void;
  /** Fired after a successful non-delete write action so the detail refreshes. */
  onChanged?: () => void;
  onOpenLogs?: (s: { context: string; namespace: string; kind: string; name: string }) => void;
  onEdit?: () => void;
  onAskAssistant?: AskAssistant;
}) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [confirmSuspend, setConfirmSuspend] = useState(false);
  const [scaling, setScaling] = useState(false);
  const [replicas, setReplicas] = useState("");
  const [triggering, setTriggering] = useState(false);
  const [busy, setBusy] = useState("");
  const [err, setErr] = useState("");
  const isCronJob = kind === "CronJob";

  // `res` is null for an unknown/CRD kind we don't have a GVR mapping for —
  // in that case leave controls enabled and let the API server stay the
  // source of truth, rather than guessing at RBAC.
  const res = kindToResource(kind);
  const ns = namespace ?? "";
  const deleteCheck = res ? rbac.deleteResource(res.group, res.resource, ns) : null;
  const scaleCheck = res && SCALABLE.includes(kind) ? rbac.scale(res.group, res.resource, ns) : null;
  const restartCheck =
    res && RESTARTABLE.includes(kind) ? rbac.rolloutRestart(res.group, res.resource, ns) : null;
  const editCheck = res && onEdit ? rbac.edit(res.group, res.resource, ns) : null;
  const cronSuspendCheck = res && isCronJob ? rbac.cronjobSuspend(ns) : null;
  const cronTriggerCheck = res && isCronJob ? rbac.cronjobTrigger(ns) : null;
  const checks: AccessCheck[] = [
    deleteCheck,
    scaleCheck,
    restartCheck,
    editCheck,
    cronSuspendCheck,
    cronTriggerCheck,
  ].filter((c): c is AccessCheck => c !== null);
  const access = useAccess(context, checks);

  async function doSetSuspend() {
    setBusy("suspend");
    setErr("");
    const resume = cronjobSuspended;
    const r = await cronjobSetSuspend(context, namespace ?? "", name, !cronjobSuspended);
    setBusy("");
    if (r.error) {
      setErr(r.error);
      reportActionError(context, `Failed to ${resume ? "resume" : "suspend"} ${name}`, r.error);
      return;
    }
    setConfirmSuspend(false);
    notify.success(`${resume ? "Resumed" : "Suspended"} ${name}`);
    onChanged?.();
  }

  async function doTrigger() {
    setBusy("trigger");
    setErr("");
    const r = await cronjobTriggerNow(context, namespace ?? "", name);
    setBusy("");
    if (r.error) {
      setErr(r.error);
      reportActionError(context, `Failed to run ${name}`, r.error);
      return;
    }
    setTriggering(false);
    notify.success(`Triggered ${name}`, r.jobName ? `Created job ${r.jobName}` : undefined);
    onChanged?.();
  }

  async function doDelete() {
    setBusy("delete");
    setErr("");
    const r = await deleteResource(context, kind, namespace, name);
    setBusy("");
    if (r.error) {
      setErr(r.error);
      reportActionError(context, `Failed to delete ${name}`, r.error);
      return;
    }
    setConfirmDelete(false);
    notify.success(`Deleted ${kind} ${name}`);
    onDeleted();
  }

  async function doScale() {
    const n = Number(replicas);
    if (!Number.isInteger(n) || n < 0) {
      setErr("Enter a non-negative replica count");
      return;
    }
    setBusy("scale");
    setErr("");
    const r = await scaleResource(context, kind, namespace ?? "", name, n);
    setBusy("");
    if (r.error) {
      setErr(r.error);
      reportActionError(context, `Failed to scale ${name}`, r.error);
      return;
    }
    setScaling(false);
    notify.success(`Scaled ${name} to ${n}`);
    onChanged?.();
  }

  async function doRestart() {
    setBusy("restart");
    setErr("");
    const r = await rolloutRestart(context, kind, namespace ?? "", name);
    setBusy("");
    if (r.error) {
      setErr(r.error);
      reportActionError(context, `Failed to restart ${name}`, r.error);
      return;
    }
    setRestarting(false);
    notify.success(`Rollout restart triggered for ${name}`);
    onChanged?.();
  }

  // Only a syntactically valid non-negative integer produces a meaningful
  // preview — mirrors the same guard `doScale` enforces before submitting.
  const replicasN = Number(replicas);
  const validReplicas = replicas.trim() !== "" && Number.isInteger(replicasN) && replicasN >= 0;

  const restartCmd = toKubectl({ action: "rollout-restart", kind, namespace: namespace ?? "", name, context });
  const suspendCmd = toKubectl({
    action: cronjobSuspended ? "cronjob-resume" : "cronjob-suspend",
    kind,
    namespace: namespace ?? "",
    name,
    context,
  });
  const triggerCmd = toKubectl({ action: "cronjob-trigger", kind, namespace: namespace ?? "", name, context });
  const scaleCmd = validReplicas
    ? toKubectl({ action: "scale", kind, namespace: namespace ?? "", name, context, replicas: replicasN })
    : null;
  const deleteCmd = toKubectl({ action: "delete", kind, namespace: namespace ?? "", name, context });

  return (
    <>
      {LOGGABLE.includes(kind) && onOpenLogs && (
        <IconButton
          icon={Logs}
          label="Logs"
          onClick={() => onOpenLogs({ context, namespace: namespace ?? "", kind, name })}
        />
      )}
      <CopyAsKubectlButton kind={kind} name={name} namespace={namespace} context={context} />
      {onEdit && (
        <IconButton
          icon={Pencil}
          label="Edit"
          disabled={editCheck ? !access.allowed(editCheck) : false}
          title={editCheck ? denyReason(access, editCheck) : undefined}
          onClick={onEdit}
        />
      )}
      {SCALABLE.includes(kind) && (
        <IconButton
          icon={Scaling}
          label="Scale"
          disabled={scaleCheck ? !access.allowed(scaleCheck) : false}
          title={scaleCheck ? denyReason(access, scaleCheck) : undefined}
          onClick={() => setScaling(true)}
        />
      )}
      {RESTARTABLE.includes(kind) && (
        <IconButton
          icon={RotateCw}
          label="Restart"
          disabled={busy === "restart" || (restartCheck ? !access.allowed(restartCheck) : false)}
          title={restartCheck ? denyReason(access, restartCheck) : undefined}
          onClick={() => {
            setErr("");
            setRestarting(true);
          }}
        />
      )}
      {isCronJob && (
        <IconButton
          icon={Zap}
          label="Run now"
          disabled={cronTriggerCheck ? !access.allowed(cronTriggerCheck) : false}
          title={cronTriggerCheck ? denyReason(access, cronTriggerCheck) : undefined}
          onClick={() => setTriggering(true)}
        />
      )}
      {isCronJob && (
        <IconButton
          icon={cronjobSuspended ? Play : Pause}
          label={cronjobSuspended ? "Resume" : "Suspend"}
          disabled={busy === "suspend" || (cronSuspendCheck ? !access.allowed(cronSuspendCheck) : false)}
          title={cronSuspendCheck ? denyReason(access, cronSuspendCheck) : undefined}
          onClick={() => {
            setErr("");
            setConfirmSuspend(true);
          }}
        />
      )}
      <IconButton
        icon={Trash2}
        label="Delete"
        danger
        disabled={deleteCheck ? !access.allowed(deleteCheck) : false}
        title={deleteCheck ? denyReason(access, deleteCheck) : undefined}
        onClick={() => setConfirmDelete(true)}
      />
      {onAskAssistant && (
        <IconButton
          icon={Bot}
          label="Ask assistant"
          title={`Ask the assistant about this ${kind}`}
          onClick={() => onAskAssistant({ context, namespace: namespace ?? undefined, kind, name })}
        />
      )}

      {restarting && (
        <ConfirmDialog
          title={`Restart ${kind}`}
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Trigger a rolling restart of <code>{name}</code>
                {namespace ? (
                  <>
                    {" "}
                    in <code>{namespace}</code>
                  </>
                ) : null}
                ? This reschedules all of its pods.
              </p>
              <KubectlPreview command={restartCmd} onCopy={() => void copyKubectlCommand(restartCmd)} />
              {err && <p style={{ color: "var(--fl-color-danger)" }}>Error: {err}</p>}
            </>
          }
          confirmLabel="Restart"
          busy={busy === "restart"}
          onConfirm={() => void doRestart()}
          onCancel={() => setRestarting(false)}
        />
      )}

      {confirmSuspend && (
        <ConfirmDialog
          title={cronjobSuspended ? "Resume CronJob" : "Suspend CronJob"}
          message={
            <>
              <p style={{ marginTop: 0 }}>
                {cronjobSuspended ? "Resume" : "Suspend"} <code>{name}</code>
                {namespace ? (
                  <>
                    {" "}
                    in <code>{namespace}</code>
                  </>
                ) : null}
                ?{" "}
                {cronjobSuspended
                  ? "Scheduled runs will resume."
                  : "Scheduled runs will be paused; already-running jobs are unaffected."}
              </p>
              <KubectlPreview command={suspendCmd} onCopy={() => void copyKubectlCommand(suspendCmd)} />
              {err && <p style={{ color: "var(--fl-color-danger)" }}>Error: {err}</p>}
            </>
          }
          confirmLabel={cronjobSuspended ? "Resume" : "Suspend"}
          busy={busy === "suspend"}
          onConfirm={() => void doSetSuspend()}
          onCancel={() => setConfirmSuspend(false)}
        />
      )}

      {triggering && (
        <ConfirmDialog
          title="Run CronJob now"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Create a one-off Job from <code>{name}</code> and run it immediately.
              </p>
              <KubectlPreview command={triggerCmd} onCopy={() => void copyKubectlCommand(triggerCmd)} />
              {err && <p style={{ color: "var(--fl-color-danger)" }}>Error: {err}</p>}
            </>
          }
          confirmLabel="Run"
          busy={busy === "trigger"}
          onConfirm={() => void doTrigger()}
          onCancel={() => setTriggering(false)}
        />
      )}

      {scaling && (
        <ConfirmDialog
          title={`Scale ${kind}`}
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Set the replica count for <code>{name}</code>.
              </p>
              <div style={{ width: 120 }}>
                <TextInput
                  value={replicas}
                  onValueChange={setReplicas}
                  placeholder="replicas"
                  aria-label="Replicas"
                />
              </div>
              {scaleCmd && (
                <KubectlPreview command={scaleCmd} onCopy={() => void copyKubectlCommand(scaleCmd)} />
              )}
              {err && <p style={{ color: "var(--fl-color-danger)" }}>Error: {err}</p>}
            </>
          }
          confirmLabel="Scale"
          busy={busy === "scale"}
          onConfirm={() => void doScale()}
          onCancel={() => setScaling(false)}
        />
      )}

      {confirmDelete && (
        <ConfirmDialog
          title={`Delete ${kind}?`}
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Delete <code>{name}</code>
                {namespace ? (
                  <>
                    {" "}
                    in <code>{namespace}</code>
                  </>
                ) : null}
                ? This cannot be undone.
              </p>
              <KubectlPreview command={deleteCmd} onCopy={() => void copyKubectlCommand(deleteCmd)} />
              {err && <p style={{ color: "var(--fl-color-danger)" }}>Error: {err}</p>}
            </>
          }
          confirmLabel="Delete"
          danger
          busy={busy === "delete"}
          onConfirm={() => void doDelete()}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </>
  );
}
