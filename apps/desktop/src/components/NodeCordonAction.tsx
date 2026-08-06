import React, { useEffect, useState } from "react";
import { ArrowDownToLine, CircleCheck, CircleSlash2, SquareTerminal } from "lucide-react";
import { getObject } from "../lib/manifest";
import { cordonNode, drainNode, createNodeDebugPod } from "../lib/actions";
import { notify } from "../lib/notify";
import { useAccess, rbac, denyReason, reportActionError } from "../lib/access";
import { IconButton, ConfirmDialog, KubectlPreview } from "../ui";
import { toKubectl } from "../lib/kubectlMapper";
import { copyKubectlCommand } from "../lib/copyKubectl";

/** Enter the host's namespaces from the privileged debug pod for a real node
 *  shell — run via exec (the pod itself just stays alive). */
const NODE_SHELL_COMMAND = [
  "nsenter", "--target", "1", "--mount", "--uts", "--ipc", "--net", "--pid", "--", "/bin/sh",
];

/**
 * Header actions for a node: cordon/uncordon and drain. Reads the node's
 * current `spec.unschedulable` to label the cordon toggle. `getObjectFn`/
 * `cordonFn`/`drainFn` are injectable for testing.
 */
export function NodeCordonAction({
  context,
  name,
  onOpenShell,
  getObjectFn = getObject,
  cordonFn = cordonNode,
  drainFn = drainNode,
  createNodeDebugPodFn = createNodeDebugPod,
}: {
  context: string;
  name: string;
  /** Open a terminal, deleting `deleteOnClose.pod` when the session closes. */
  onOpenShell?: (s: {
    context: string;
    namespace: string;
    pod: string;
    container?: string;
    deleteOnClose?: { context: string; namespace: string; pod: string };
    execCommand?: string[];
  }) => void;
  getObjectFn?: typeof getObject;
  cordonFn?: typeof cordonNode;
  drainFn?: typeof drainNode;
  createNodeDebugPodFn?: typeof createNodeDebugPod;
}) {
  const [cordoned, setCordoned] = useState<boolean | null>(null);
  const [dialog, setDialog] = useState<"cordon" | "drain" | "shell" | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  async function doNodeShell() {
    setBusy(true);
    setErr("");
    const out = await createNodeDebugPodFn(context, name, null, null);
    setBusy(false);
    if (out.error || !out.pod || !out.namespace) {
      setErr(out.error ?? "no pod created");
      reportActionError(context, `Failed to open node shell on ${name}`, out.error ?? "");
      return;
    }
    setDialog(null);
    notify.success(`Node debug pod ${out.pod} created`);
    onOpenShell?.({
      context,
      namespace: out.namespace,
      pod: out.pod,
      container: "debug",
      execCommand: NODE_SHELL_COMMAND,
      deleteOnClose: { context, namespace: out.namespace, pod: out.pod },
    });
  }

  const cordonCheck = rbac.cordon();
  const drainCheck = rbac.drain();
  const access = useAccess(context, [cordonCheck, drainCheck]);

  useEffect(() => {
    let active = true;
    void getObjectFn(context, "Node", null, name).then((o) => {
      if (!active || !o.object) return;
      setCordoned((o.object.spec as { unschedulable?: boolean } | undefined)?.unschedulable === true);
    });
    return () => {
      active = false;
    };
  }, [context, name, getObjectFn]);

  if (cordoned === null) return null; // unknown until the node loads

  async function applyCordon() {
    setBusy(true);
    setErr("");
    const r = await cordonFn(context, name, !cordoned);
    setBusy(false);
    if (r.error) {
      setErr(r.error);
      reportActionError(context, `Failed to ${cordoned ? "uncordon" : "cordon"} ${name}`, r.error);
      return;
    }
    setDialog(null);
    notify.success(`${cordoned ? "Uncordoned" : "Cordoned"} ${name}`);
    setCordoned(!cordoned);
  }

  async function applyDrain() {
    setBusy(true);
    setErr("");
    const r = await drainFn(context, name);
    setBusy(false);
    if (r.error) {
      setErr(r.error);
      reportActionError(context, `Failed to drain ${name}`, r.error);
      return;
    }
    setDialog(null);
    notify.success(`Drained ${name}`, r.evicted != null ? `Evicted ${r.evicted} pod(s)` : undefined);
    setCordoned(true); // drain cordons the node
  }

  // ResourceActions already renders a "Copy as kubectl" affordance for Node
  // (ResourceBrowser.tsx mounts both alongside each other), so this component
  // only needs the cordon/drain previews below.
  const cordonCmd = toKubectl({ action: cordoned ? "uncordon" : "cordon", kind: "Node", name, context });
  const drainCmd = toKubectl({ action: "drain", kind: "Node", name, context });

  return (
    <>
      <IconButton
        icon={cordoned ? CircleCheck : CircleSlash2}
        label={cordoned ? "Uncordon" : "Cordon"}
        disabled={!access.allowed(cordonCheck)}
        title={denyReason(access, cordonCheck)}
        onClick={() => {
          setErr("");
          setDialog("cordon");
        }}
      />
      <IconButton
        icon={ArrowDownToLine}
        label="Drain"
        disabled={!access.allowed(drainCheck)}
        title={denyReason(access, drainCheck)}
        onClick={() => {
          setErr("");
          setDialog("drain");
        }}
      />
      {onOpenShell && (
        <IconButton
          icon={SquareTerminal}
          label="Node shell"
          title="Open a privileged shell on this node"
          onClick={() => {
            setErr("");
            setDialog("shell");
          }}
        />
      )}

      {dialog === "shell" && (
        <ConfirmDialog
          title="Open node shell?"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Create a <strong>privileged</strong> debug pod on <code>{name}</code> that enters
                the host namespaces, and open a root shell. The pod is deleted when you close the
                terminal.
              </p>
              {err && <p className="text-destructive">Error: {err}</p>}
            </>
          }
          confirmLabel="Open shell"
          danger
          busy={busy}
          onConfirm={() => void doNodeShell()}
          onCancel={() => setDialog(null)}
        />
      )}

      {dialog === "cordon" && (
        <ConfirmDialog
          title={cordoned ? "Uncordon node?" : "Cordon node?"}
          message={
            <>
              <p style={{ marginTop: 0 }}>
                {cordoned ? "Allow" : "Stop"} scheduling new pods on <code>{name}</code>?
              </p>
              <KubectlPreview command={cordonCmd} onCopy={() => void copyKubectlCommand(cordonCmd)} />
              {err && <p className="text-destructive">Error: {err}</p>}
            </>
          }
          confirmLabel={cordoned ? "Uncordon" : "Cordon"}
          busy={busy}
          onConfirm={() => void applyCordon()}
          onCancel={() => setDialog(null)}
        />
      )}

      {dialog === "drain" && (
        <ConfirmDialog
          title="Drain node?"
          message={
            <>
              <p style={{ marginTop: 0 }}>
                Cordon <code>{name}</code> and evict its pods (DaemonSet and static pods stay)?
              </p>
              <KubectlPreview command={drainCmd} onCopy={() => void copyKubectlCommand(drainCmd)} />
              {err && <p className="text-destructive">Error: {err}</p>}
            </>
          }
          confirmLabel="Drain"
          danger
          busy={busy}
          onConfirm={() => void applyDrain()}
          onCancel={() => setDialog(null)}
        />
      )}
    </>
  );
}
