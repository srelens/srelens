import { useEffect, useState } from "react";
import {
  createNodeDebugPod,
  defaultContainer,
  execCandidates,
  getObject,
  isTauri,
  listNamespaces,
  listNodes,
  listPods,
  podContainerChoices,
  type ContainerChoice,
} from "@srelens/core";
import { Alert, Button, Dialog, Field, Select } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";
import { startLocalSession, startPodSession } from "../../lib/sessions";

/** §14's dialog width — narrow, three fields deep at most. */
const WIDTH = 420;

/**
 * Enter the host's namespaces from the privileged debug pod for a real node
 * shell — run via `exec`, the pod itself just stays alive holding it open.
 * Mirrors classic's `NodeCordonAction.NODE_SHELL_COMMAND`, which this
 * replaces: that component is `apps/desktop`'s, and this screen does not
 * reach across into it for one array.
 */
const NODE_SHELL_COMMAND = [
  "nsenter",
  "--target",
  "1",
  "--mount",
  "--uts",
  "--ipc",
  "--net",
  "--pid",
  "--",
  "/bin/sh",
];

/** The container name `k8s.createNodeDebugPod` always attaches under. */
const NODE_DEBUG_CONTAINER = "debug";

type Kind = "pod" | "node" | "local";

const KIND_LABEL: Record<Kind, string> = { pod: "Pod", node: "Node", local: "Local shell" };

export interface NewSessionMenuProps {
  /** The cluster to start a session in — a kubeconfig context name. */
  context: string;
  /** Where the namespace select starts, when the caller knows a good one. */
  namespace?: string;
  /** A session came up: its id, so the caller can make it the active one. */
  onStarted: (id: number) => void;
  /** Cancel, escape, the header's control, and a session that came up. */
  onClose: () => void;
}

/**
 * §14's `New session` — what can be started here, and where.
 *
 * **The local shell is drawn only on the desktop.** `WEB_DENIED_COMMANDS` is
 * `["start_terminal"]` — the local PTY is the one command the web surface
 * refuses, because it is a shell on the SHARED HOST, running as the server
 * process's own UID, with no cluster and no RBAC between the reader and it.
 * Pod and node exec are both a shell inside something the cluster already
 * decided the reader may enter, so both stay offered everywhere. Rather than
 * draw the control and let every click fail, the kind selector simply does
 * not offer `local` in the browser (see {@link kindsFor}), and the one
 * sentence explaining why is said once, near the top of the menu — not
 * discovered per click, and not repeated once per kind. Same rule as the
 * Toolbox's absent install column.
 *
 * **A node shell needs a debug pod before it needs a session.**
 * `k8s.createNodeDebugPod` is the privileged pod srelens creates to reach the
 * host namespaces via `nsenter`; the session store's own note on
 * {@link import("../../lib/sessions").endSession} is what deletes it again
 * when the shell ends. Starting a node session is therefore two calls where
 * a pod session is one, and only the first of those can fail before there is
 * anything to hand the store — that failure is shown here, in the dialog,
 * because no row exists yet to carry it.
 *
 * **A pod session never fails here.** `startPodSession` never throws: a shell
 * RBAC refused, or a container with no shell in it, becomes a `closed` row in
 * the rail rather than an error in this dialog. So picking a pod and a
 * container closes the menu immediately and hands the new (possibly already
 * closed) id to the caller — the rail is where that failure is read.
 *
 * **`execCandidates` decides which containers are offered**, not this file: a
 * finished init container is listed by `podContainerChoices` but dropped by
 * `execCandidates`, and the container select only ever sees what survives
 * that filter.
 */
export function NewSessionMenu({ context, namespace: initial, onStarted, onClose }: NewSessionMenuProps) {
  const desktop = isTauri();
  const [kind, setKind] = useState<Kind>("pod");

  const [namespaces, setNamespaces] = useState<string[] | null>(null);
  const [namespace, setNamespace] = useState(initial ?? "");
  const [pods, setPods] = useState<string[] | null>(null);
  const [pod, setPod] = useState("");
  const [containers, setContainers] = useState<ContainerChoice[] | null>(null);
  const [container, setContainer] = useState("");

  const [nodes, setNodes] = useState<string[] | null>(null);
  const [node, setNode] = useState("");

  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ title: string; error: unknown } | null>(null);

  // Namespaces, up front: the pod kind is where the menu opens, and a
  // namespace picked before the pod kind is even shown would waste the round
  // trip a `kind === "pod"` gate would only defer.
  useEffect(() => {
    if (!context) {
      setNamespaces([]);
      return;
    }
    let live = true;
    setNamespaces(null);
    void listNamespaces(context).then((out) => {
      if (!live) return;
      if (out.error) setFailure({ title: "Could not list this cluster's namespaces", error: out.error });
      setNamespaces(out.namespaces ?? []);
    });
    return () => {
      live = false;
    };
  }, [context]);

  useEffect(() => {
    if (!namespace) {
      setPods(null);
      setPod("");
      return;
    }
    let live = true;
    setPods(null);
    void listPods(context, namespace).then((out) => {
      if (!live) return;
      if (out.error) setFailure({ title: `Could not list pods in ${namespace}`, error: out.error });
      const names = (out.pods ?? []).map((p) => p.name);
      setPods(names);
      setPod((current) => (names.includes(current) ? current : ""));
    });
    return () => {
      live = false;
    };
  }, [context, namespace]);

  useEffect(() => {
    if (!namespace || !pod) {
      setContainers(null);
      setContainer("");
      return;
    }
    let live = true;
    setContainers(null);
    void getObject(context, "Pod", namespace, pod).then((out) => {
      if (!live) return;
      if (out.error || !out.object) {
        setFailure({ title: `Could not read ${pod}'s containers`, error: out.error ?? "no object returned" });
        setContainers([]);
        return;
      }
      const choices = execCandidates(podContainerChoices(out.object));
      setContainers(choices);
      setContainer(defaultContainer(out.object, choices) ?? choices[0]?.name ?? "");
    });
    return () => {
      live = false;
    };
  }, [context, namespace, pod]);

  useEffect(() => {
    if (kind !== "node" || !context) return;
    let live = true;
    setNodes(null);
    void listNodes(context).then((out) => {
      if (!live) return;
      if (out.error) setFailure({ title: "Could not list this cluster's nodes", error: out.error });
      setNodes((out.nodes ?? []).map((n) => n.name));
    });
    return () => {
      live = false;
    };
  }, [context, kind]);

  async function startPod() {
    if (!namespace || !pod) return;
    setBusy(true);
    const id = await startPodSession({ context, namespace, pod, container: container || undefined });
    setBusy(false);
    onStarted(id);
    onClose();
  }

  async function startNode() {
    if (!node) return;
    setFailure(null);
    setBusy(true);
    const created = await createNodeDebugPod(context, node);
    if (created.error || !created.pod || !created.namespace) {
      setBusy(false);
      setFailure({ title: `Could not open a shell on ${node}`, error: created.error ?? "no debug pod created" });
      return;
    }
    const id = await startPodSession({
      context,
      namespace: created.namespace,
      pod: created.pod,
      container: NODE_DEBUG_CONTAINER,
      command: NODE_SHELL_COMMAND,
      kind: "node",
      title: node,
    });
    setBusy(false);
    onStarted(id);
    onClose();
  }

  async function startLocal() {
    setBusy(true);
    const id = await startLocalSession({ context });
    setBusy(false);
    onStarted(id);
    onClose();
  }

  function start() {
    if (kind === "pod") void startPod();
    else if (kind === "node") void startNode();
    else void startLocal();
  }

  const ready =
    !busy &&
    (kind === "pod" ? Boolean(namespace && pod) : kind === "node" ? Boolean(node) : true);

  const namespaceOptions = (namespaces ?? []).map((n) => ({ value: n }));
  const podOptions = (pods ?? []).map((p) => ({ value: p }));
  const containerOptions = (containers ?? []).map((c) => ({
    value: c.name,
    label: c.running ? c.name : `${c.name} (not running)`,
  }));
  const nodeOptions = (nodes ?? []).map((n) => ({ value: n }));

  return (
    <Dialog
      title="New session"
      maxWidth={WIDTH}
      onClose={onClose}
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" disabled={!ready} onClick={start}>
            Start session
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 p-3">
        {failure && <FailureAlert tone="sev" title={failure.title} error={failure.error} />}

        <Field label="Kind">
          <div className="flex gap-1.5" role="group" aria-label="Session kind">
            {kindsFor(desktop).map((k) => (
              <Button
                key={k}
                type="button"
                variant={kind === k ? "primary" : "secondary"}
                size="sm"
                aria-pressed={kind === k}
                onClick={() => setKind(k)}
              >
                {KIND_LABEL[k]}
              </Button>
            ))}
          </div>
        </Field>

        {!desktop && (
          // Said once for the whole menu, not per kind: a local shell is not
          // drawn at all above (see `kindsFor`), so this is the one place the
          // reason lives rather than something discovered per click.
          <Alert tone="info" title="No local shell in the browser">
            A local shell runs as srelens's own process on the machine it is running on — outside the
            cluster and unbounded by its RBAC — so it is the only session kind the browser build does
            not offer. Pod and node shells stay available: both are bounded by what the cluster lets
            you do.
          </Alert>
        )}

        {kind === "pod" && (
          <div className="grid grid-cols-2 gap-x-3">
            <Field label="Namespace">
              <Select
                value={namespace}
                onValueChange={setNamespace}
                options={namespaceOptions}
                className="w-full"
                disabled={namespaces === null}
                placeholder={namespaces === null ? "Loading…" : "Choose a namespace"}
              />
            </Field>
            <Field label="Pod">
              <Select
                value={pod}
                onValueChange={setPod}
                options={podOptions}
                className="w-full"
                disabled={!namespace || pods === null}
                placeholder={
                  !namespace ? "Pick a namespace first" : pods === null ? "Loading…" : "Choose a pod"
                }
              />
            </Field>
            <Field label="Container" className="col-span-2">
              <Select
                value={container}
                onValueChange={setContainer}
                options={containerOptions}
                className="w-full"
                disabled={!pod || containers === null}
                placeholder={
                  !pod
                    ? "Pick a pod first"
                    : containers === null
                      ? "Loading…"
                      : containerOptions.length === 0
                        ? "Nothing here to shell into"
                        : "Choose a container"
                }
              />
            </Field>
          </div>
        )}

        {kind === "node" && (
          <Field label="Node">
            <Select
              value={node}
              onValueChange={setNode}
              options={nodeOptions}
              className="w-full"
              disabled={nodes === null}
              placeholder={nodes === null ? "Loading…" : "Choose a node"}
            />
          </Field>
        )}

        {kind === "node" && (
          <div className="text-[0.75rem] text-muted">
            Creates a privileged debug pod on the node and enters its host namespaces; the shell
            deletes it when the session ends.
          </div>
        )}

        {kind === "local" && (
          <div className="text-[0.75rem] text-muted">
            {`Opens a shell on this machine, with KUBECONFIG scoped to ${context || "the active cluster"}.`}
          </div>
        )}
      </div>
    </Dialog>
  );
}

/** The kinds this platform can actually start. `local` never appears in the
 *  browser — see the component doc for why drawing it there would be wrong
 *  even disabled. */
function kindsFor(desktop: boolean): Kind[] {
  return desktop ? ["pod", "node", "local"] : ["pod", "node"];
}
