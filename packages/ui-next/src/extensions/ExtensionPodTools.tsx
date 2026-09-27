// Logs, exec and port-forwards from apps (#567), where a person meets them: on
// the resource whose pods an app's pod binding reaches. The host lists the pods
// (`extensions.pods`), holds every session to the binding's scope, runs only the
// manifest's command after the host confirmation, and picks the local port.
// Everything drawn here is host UI: the app supplies titles and a command, drawn
// as plain text; it supplies no markup, no style and no wording of its own.
//
// Each mounted panel is one app stream view. Unmounting closes it, and the host
// ends every stream it opened — a log follow, a command still running, and each
// forward with its port and connections (docs/extensions/streams.md).
import { useEffect, useMemo, useRef, useState } from "react";
import {
  POD_EXEC,
  POD_FORWARD,
  POD_LOGS,
  POD_TARGETS,
  contributionKind,
  describeStreamEnd,
  extensionEnabledFor,
  extensionPods,
  isExtensionExecEvent,
  isExtensionForwardEvent,
  isTauri,
  logLineLevel,
  openExtensionView,
  podNamespaces,
  renderConfirmTemplate,
  startExtensionLogStream,
  type ExtensionForwardEvent,
  type ExtensionManifest,
  type ExtensionStream,
  type ExtensionStreamEnd,
  type ExtensionView,
  type InstalledExtension,
  type LogTarget,
} from "@srelens/core";
import { Button, Combobox } from "@srelens/ui-kit";
import { HostConfirmation } from "../confirm/HostConfirmation";
import { useConfirmationApp } from "../confirm/confirmationApp";
import { confirmFields } from "../confirm/confirmRequest";
import { EXEC_CONFIRM } from "./ExtensionBindings";
import { useLogStream, type LogSourceOpener } from "../lib/logStream";
import { useResource } from "../lib/useResource";
import { useContextLookup } from "./contextIds";
import { commandArgument, plainText } from "./displayText";
import { ErrorNotice } from "./ExtensionResults";
import { extensionLabel, useExtensions } from "./inventoryStore";

type PodBinding = ExtensionManifest["capabilities"][number];

/** The qualified kind each built-in reader lists, as the host names it. */
const BUILTIN_KINDS: Record<string, string> = {
  "k8s.listDeployments": "apps/Deployment",
  "k8s.listStatefulSets": "apps/StatefulSet",
  "k8s.listDaemonSets": "apps/DaemonSet",
};

/** What a pod binding offers on one resource: its object's pods, or this one pod. */
export type PodPlace =
  | { kind: "object"; namespace: string; name: string }
  | { kind: "pod"; namespace: string; pod: string };

/** The qualified kind a reader binding lists, or `undefined` for one that lists none. */
function readerKind(reader: PodBinding | undefined): string | undefined {
  if (!reader) return undefined;
  const builtin = BUILTIN_KINDS[reader.target];
  if (builtin) return builtin;
  if (reader.target !== "k8s.listCustomResource") return undefined;
  const { group, kind } = reader.arguments;
  return typeof group === "string" && typeof kind === "string" ? `${group}/${kind}` : undefined;
}

/**
 * The pod bindings of `manifest` that reach pods from a resource of `kind` named
 * `name` in `namespace`: those scoped by a reader of that kind, for the object;
 * and, on a Pod in a namespace a permission grants, the namespace-scoped ones.
 */
export function podBindingsFor(
  manifest: ExtensionManifest,
  kind: string,
  namespace: string,
  name: string,
): Array<{ binding: PodBinding; place: PodPlace }> {
  return manifest.capabilities.flatMap((binding) => {
    if (!POD_TARGETS.includes(binding.target) || !namespace) return [];
    const resource = binding.arguments.resource;
    if (typeof resource === "string") {
      const reader = manifest.capabilities.find((candidate) => candidate.name === resource);
      return readerKind(reader) === kind ? [{ binding, place: { kind: "object", namespace, name } as PodPlace }] : [];
    }
    return kind === "/Pod" && podNamespaces(manifest, binding.target).includes(namespace)
      ? [{ binding, place: { kind: "pod", namespace, pod: name } as PodPlace }]
      : [];
  });
}

type SlotResource = { apiVersion: string; kind: string; metadata: { name: string; namespace?: string } };

function slotResource(value: unknown): value is SlotResource {
  if (!value || typeof value !== "object") return false;
  const resource = value as Partial<SlotResource>;
  return typeof resource.kind === "string" && typeof resource.apiVersion === "string"
    && !!resource.metadata && typeof resource.metadata.name === "string";
}

/**
 * The pod tools every enabled app offers on `resource`, after the host's own
 * sections — as `ExtensionPanelSlot` adds panels.
 */
export function ExtensionPodSlot({ context, resource }: { context: string; resource: unknown }) {
  const inventory = useExtensions();
  const lookup = useContextLookup(context);
  if (!slotResource(resource)) return null;
  const group = resource.apiVersion.includes("/") ? resource.apiVersion.split("/")[0] : "";
  const kind = contributionKind(resource.kind, group);
  const namespace = resource.metadata.namespace ?? "";
  const contextId = lookup.status === "found" ? lookup.id : undefined;
  const offered = (inventory.data?.plugins ?? [])
    .filter((plugin) => plugin.enabled && !plugin.quarantined && !plugin.policyBlocked)
    .filter((plugin) => extensionEnabledFor(plugin, contextId))
    .map((plugin) => ({ plugin, bindings: podBindingsFor(plugin.manifest, kind, namespace, resource.metadata.name) }))
    .filter(({ bindings }) => bindings.length > 0);
  return (
    <>
      {offered.map(({ plugin, bindings }) => (
        <ExtensionPodTools
          // The resource is part of the identity: moving the Inspector to another
          // one starts over, with a new view, and the old view's streams end.
          key={`${plugin.manifest.id}/${plugin.revision}/${kind}/${namespace}/${resource.metadata.name}`}
          plugin={plugin}
          context={context}
          bindings={bindings}
        />
      ))}
    </>
  );
}

/** Pods listed for one scope: every binding sharing a reader and a selector path. */
function scopeKey(binding: PodBinding): string {
  const { resource, selector } = binding.arguments;
  return typeof resource === "string" ? `${resource}|${String(selector ?? "")}` : `namespace|${binding.target}`;
}

type Session =
  | { id: number; kind: "logs"; binding: PodBinding; pod: string; container: string }
  | { id: number; kind: "exec"; binding: PodBinding; pod: string; container: string; command: string[] }
  | { id: number; kind: "forward"; binding: PodBinding; pod?: string; service?: string };
/** A session before it has an id: `Omit` over each member of the union, not over what they share. */
type NewSession = Session extends infer S ? (S extends Session ? Omit<S, "id"> : never) : never;

/** The facts a place names: the object, or the one pod, in its namespace. */
const objectName = (place: PodPlace) => (place.kind === "object" ? place.name : undefined);

/** One app's pod tools on one resource: its pods, each binding's control, and the sessions open. */
export function ExtensionPodTools({
  plugin,
  context,
  bindings,
}: {
  plugin: InstalledExtension;
  context: string;
  bindings: Array<{ binding: PodBinding; place: PodPlace }>;
}) {
  const app = extensionLabel(plugin);
  const viewRef = useRef<ExtensionView | null>(null);
  if (viewRef.current === null) viewRef.current = openExtensionView(plugin.manifest.id, "pods");
  const view = viewRef.current;
  // One view per mounted panel: closing it ends every stream and forward it opened.
  useEffect(() => () => void view.close(), [view]);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [pending, setPending] = useState<Omit<Extract<Session, { kind: "exec" }>, "id"> | null>(null);
  const next = useRef(1);
  // The control that opened the exec review, which gets focus back when it closes.
  const opener = useRef<HTMLElement | null>(null);
  const start = (session: NewSession) =>
    setSessions((open) => [...open, { ...session, id: next.current++ } as Session]);
  const stop = (id: number) => setSessions((open) => open.filter((session) => session.id !== id));
  const place = bindings[0].place;
  const groups = useMemo(() => {
    const byScope = new Map<string, PodBinding[]>();
    for (const { binding } of bindings) byScope.set(scopeKey(binding), [...(byScope.get(scopeKey(binding)) ?? []), binding]);
    return [...byScope.values()];
  }, [bindings]);
  if (!isTauri()) {
    return (
      <section className="section extension-pod-tools" aria-label={`${plainText(app)} pod tools`}>
        <h4 className="extension-detail-heading">{plainText(app)} · pods</h4>
        <p className="extension-message">
          Logs, commands and port-forwards from apps run in the desktop app. The web app does not run app streams yet.
        </p>
      </section>
    );
  }
  return (
    <section className="section extension-pod-tools" aria-label={`${plainText(app)} pod tools`}>
      <h4 className="extension-detail-heading">{plainText(app)} · pods</h4>
      {groups.map((group) => (
        <PodGroup
          key={scopeKey(group[0])}
          plugin={plugin}
          context={context}
          place={place}
          bindings={group}
          onLogs={(binding, pod, container) => start({ kind: "logs", binding, pod, container })}
          onRun={(binding, pod, container, command, trigger) => {
            opener.current = trigger;
            setPending({ kind: "exec", binding, pod, container, command });
          }}
          onForward={(binding, target) => start({ kind: "forward", binding, ...target })}
        />
      ))}
      {pending && (
        <ExecReview
          plugin={plugin}
          context={context}
          namespace={place.namespace}
          pending={pending}
          opener={opener}
          onCancel={() => setPending(null)}
          onRun={() => {
            start(pending);
            setPending(null);
          }}
        />
      )}
      {sessions.length > 0 && (
        <div className="extension-pod-sessions" aria-label={`${plainText(app)} sessions`} role="group">
          {sessions.map((session) => (
            <SessionPane
              key={session.id}
              session={session}
              view={view}
              plugin={plugin}
              context={context}
              place={place}
              onClose={() => stop(session.id)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

const verbs: Record<string, string> = { [POD_LOGS]: "Logs", [POD_EXEC]: "Run", [POD_FORWARD]: "Forward" };

/** The pods one scope reaches, as the host lists them, with a control per binding. */
function PodGroup({
  plugin,
  context,
  place,
  bindings,
  onLogs,
  onRun,
  onForward,
}: {
  plugin: InstalledExtension;
  context: string;
  place: PodPlace;
  bindings: PodBinding[];
  onLogs(binding: PodBinding, pod: string, container: string): void;
  onRun(binding: PodBinding, pod: string, container: string, command: string[], trigger: HTMLElement): void;
  onForward(binding: PodBinding, target: { pod?: string; service?: string }): void;
}) {
  // Asked through a binding that forwards through a Service when the group has one: its
  // answer carries the Services as well as the pods, and the pods are the same scope's.
  const first =
    bindings.find((binding) => binding.target === POD_FORWARD && binding.arguments.service === true) ?? bindings[0];
  const pods = useResource(
    () =>
      extensionPods({
        id: plugin.manifest.id,
        revision: plugin.revision,
        capability: first.name,
        context,
        namespace: place.namespace,
        name: objectName(place),
      }),
    [plugin.manifest.id, plugin.revision, first.name, context, place.namespace, objectName(place)],
    () => false,
  );
  const [containers, setContainers] = useState<Record<string, string>>({});
  if (pods.status === "loading") return <p className="extension-message">Listing the pods {plainText(first.title)} may reach…</p>;
  if (pods.status === "error" || !pods.data)
    return <ErrorNotice title="Could not list the pods this app may reach" message={pods.error} retry={pods.reload} cluster />;
  const shown = place.kind === "pod" ? pods.data.pods.filter((pod) => pod.name === place.pod) : pods.data.pods;
  const podBindings = bindings.filter((binding) => !(binding.target === POD_FORWARD && binding.arguments.service === true));
  const serviceBindings = bindings.filter((binding) => binding.target === POD_FORWARD && binding.arguments.service === true);
  return (
    <div className="extension-pod-group">
      <p className="extension-message">
        {/* The host's words for the scope, e.g. "pods selected by Deployment web". */}
        {plainText(pods.data.scope.charAt(0).toUpperCase() + pods.data.scope.slice(1))}
        {pods.data.truncated ? " (more than the host lists)" : ""}.
      </p>
      {shown.length === 0 ? (
        <p className="extension-message">
          {place.kind === "pod" ? "This pod is not one the app may reach." : `The cluster reports no such pods right now.`}
        </p>
      ) : (
        // One block per pod, not a table: the Inspector is narrow, and a row of a pod's
        // name, state, container and every binding's control does not fit across it.
        <ul className="extension-pod-list" aria-label={`Pods ${plainText(bindings[0].title)} may reach`}>
          {shown.map((pod) => {
            const chosen = containers[pod.name] ?? pod.containers[0] ?? "";
            return (
              <li key={pod.name} className="extension-pod" aria-label={`Pod ${plainText(pod.name)}`}>
                <div className="extension-pod-head">
                  <code>{plainText(pod.name)}</code>
                  <span>
                    {plainText(pod.phase ?? "Unknown")}
                    {pod.ready ? " · ready" : " · not ready"}
                  </span>
                </div>
                {podBindings.length > 0 && (
                  <div className="extension-pod-actions">
                    {pod.containers.length > 1 ? (
                      <Combobox
                        value={chosen}
                        onValueChange={(value) => setContainers((all) => ({ ...all, [pod.name]: value }))}
                        options={pod.containers.map((value) => ({ value }))}
                        ariaLabel={`Container of ${pod.name}`}
                      />
                    ) : (
                      <code>{plainText(chosen)}</code>
                    )}
                    {podBindings.map((binding) => {
                      const fixed = typeof binding.arguments.container === "string" ? binding.arguments.container : undefined;
                      const container = fixed ?? chosen;
                      const name = `${verbs[binding.target]} · ${binding.title}`;
                      return (
                        <Button
                          key={binding.name}
                          type="button"
                          variant="secondary"
                          size="sm"
                          aria-label={`${name} (${pod.name})`}
                          title={fixed ? `Runs in container ${fixed}` : undefined}
                          onClick={(event) => {
                            if (binding.target === POD_LOGS) onLogs(binding, pod.name, container);
                            else if (binding.target === POD_EXEC)
                              onRun(
                                binding,
                                pod.name,
                                container,
                                (binding.arguments.command as string[]) ?? [],
                                event.currentTarget,
                              );
                            else onForward(binding, { pod: pod.name });
                          }}
                        >
                          {plainText(name)}
                        </Button>
                      );
                    })}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {serviceBindings.length > 0 && (
        <p className="extension-message extension-pod-services-label">Through a Service:</p>
      )}
      {serviceBindings.length > 0 && (
        <ul className="extension-binding-readers" aria-label="Services to forward through">
          {(pods.data.services ?? []).length === 0 ? (
            <li>No Service sends to a pod this app may reach.</li>
          ) : (
            (pods.data.services ?? []).map((service) =>
              serviceBindings.map((binding) => (
                <li key={`${service.name}/${binding.name}`}>
                  <code>{plainText(service.name)}</code> port {service.port}{" "}
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => onForward(binding, { service: service.name })}
                  >
                    {plainText(`Forward · ${binding.title}`)}
                  </Button>
                </li>
              )),
            )
          )}
        </ul>
      )}
    </div>
  );
}

/**
 * The host confirmation for one exec session (#552): the level, the cluster, the
 * pod, the container and the exact command, and who asked. What is approved is
 * what the host is sent, field for field, and it refuses anything else.
 */
function ExecReview({
  plugin,
  context,
  namespace,
  pending,
  opener,
  onCancel,
  onRun,
}: {
  plugin: InstalledExtension;
  context: string;
  namespace: string;
  pending: Omit<Extract<Session, { kind: "exec" }>, "id">;
  opener: { current: HTMLElement | null };
  onCancel(): void;
  onRun(): void;
}) {
  const app = useConfirmationApp({ id: plugin.manifest.id, revision: plugin.revision });
  const region = useRef<HTMLDivElement>(null);
  // Focus moves in, and goes back to what opened it when the review closes —
  // on Cancel, Escape or Run alike — rather than falling to the page.
  useEffect(() => {
    // Named by the click, not read off `document.activeElement`: a click does not
    // focus a button in every browser.
    const back = opener.current;
    region.current?.focus();
    return () => back?.focus();
  }, [opener]);
  return (
    <div
      className="extension-review"
      role="region"
      aria-label="Review command"
      tabIndex={-1}
      ref={region}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <HostConfirmation
        // The host's sentence, from its template: nothing of the app's is in it.
        question={renderConfirmTemplate(
          EXEC_CONFIRM,
          confirmFields({ cluster: context, kind: "Pod", namespace, name: pending.pod }),
        )}
        impact="high"
        cluster={context}
        subject={{ kind: "object", namespace, name: pending.pod }}
        app={app}
        command={{ container: pending.container, argv: pending.command }}
        actions={
          <>
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="button" variant="danger" onClick={onRun}>
              Run command
            </Button>
          </>
        }
      />
    </div>
  );
}

/** One open session, and the control that ends it. */
function SessionPane({
  session,
  view,
  plugin,
  context,
  place,
  onClose,
}: {
  session: Session;
  view: ExtensionView;
  plugin: InstalledExtension;
  context: string;
  place: PodPlace;
  onClose(): void;
}) {
  const request = { id: plugin.manifest.id, revision: plugin.revision, context, namespace: place.namespace };
  const title =
    session.kind === "forward"
      ? `${plainText(session.binding.title)} · ${plainText(session.pod ?? `service ${session.service}`)}`
      : `${plainText(session.binding.title)} · ${plainText(session.pod)}/${plainText(session.container)}`;
  return (
    <article className="extension-pod-session" aria-label={title}>
      <header>
        <strong>{title}</strong>
        <Button type="button" variant="ghost" size="sm" onClick={onClose}>
          {session.kind === "forward" ? "Stop forwarding" : session.kind === "logs" ? "Stop following" : "Close"}
        </Button>
      </header>
      {session.kind === "logs" && (
        <LogsSession view={view} request={request} place={place} session={session} />
      )}
      {session.kind === "exec" && (
        <ExecSession view={view} request={request} place={place} session={session} />
      )}
      {session.kind === "forward" && (
        <ForwardSession view={view} request={request} place={place} session={session} />
      )}
    </article>
  );
}

type Request = { id: string; revision: number; context: string; namespace: string };

/** A container's logs, through the pod log view's own buffer and status counting. */
function LogsSession({
  view,
  request,
  place,
  session,
}: {
  view: ExtensionView;
  request: Request;
  place: PodPlace;
  session: Extract<Session, { kind: "logs" }>;
}) {
  const [end, setEnd] = useState<ExtensionStreamEnd | null>(null);
  const [dropped, setDropped] = useState(0);
  const label = `${session.pod}/${session.container}`;
  const targets = useMemo<LogTarget[]>(() => [{ pod: session.pod, container: session.container, label }], [session.pod, session.container, label]);
  const open: LogSourceOpener = (_targets, onLine, onStatus, options) =>
    startExtensionLogStream(
      view,
      {
        ...request,
        source: {
          kind: "logs",
          capability: session.binding.name,
          name: objectName(place),
          pod: session.pod,
          container: session.container,
        },
      },
      onLine,
      onStatus,
      { onDropped: (count) => setDropped((all) => all + count), onEnd: setEnd },
      options,
    );
  const stream = useLogStream(request.context, request.namespace, targets, {
    tailLines: 200,
    source: { key: `app:${request.id}#${request.revision}/${session.binding.name}/${label}`, open },
  });
  const status = end
    ? describeStreamEnd(end)
    : stream.status === "error"
      ? `Could not follow: ${stream.error?.detail ?? "the host refused"}`
      : stream.status === "live"
        ? "Following"
        : stream.status === "reconnecting"
          ? "Reconnecting: the lines shown may not be current"
          : stream.status === "completed"
            ? "The container's log ended"
            : "Connecting…";
  return (
    <>
      <p className="extension-message" role="status">
        {status}
        {dropped > 0 && ` · ${dropped.toLocaleString("en-US")} lines were dropped because they arrived faster than the host sends them`}
      </p>
      {/* Focusable: it scrolls both ways, and a keyboard must be able to reach the rest. */}
      <div className="extension-pod-log" role="log" aria-label={`Logs of ${label}`} tabIndex={0}>
        {stream.lines.length === 0 ? (
          <p className="extension-message">{end || stream.status === "error" ? "No lines." : "Nothing logged yet."}</p>
        ) : (
          // Compact rows rather than the Logs screen's four-column line: the pod and
          // container are the session's, named in its header, and the Inspector is narrow.
          stream.lines.map((line, index) => {
            const level = logLineLevel(line.text);
            return (
              <div key={index} className="extension-pod-line">
                {level && (
                  <span className="extension-pod-level" data-level={level.toLowerCase()}>
                    {plainText(level)}
                  </span>
                )}
                <span>{plainText(line.text)}</span>
              </div>
            );
          })
        )}
      </div>
    </>
  );
}

/** One confirmed run of the binding's command: its output as it arrives, then its exit code. */
function ExecSession({
  view,
  request,
  place,
  session,
}: {
  view: ExtensionView;
  request: Request;
  place: PodPlace;
  session: Extract<Session, { kind: "exec" }>;
}) {
  const [output, setOutput] = useState<Array<{ stream: "stdout" | "stderr"; text: string }>>([]);
  const [exit, setExit] = useState<number | null>(null);
  const [end, setEnd] = useState<ExtensionStreamEnd | null>(null);
  const [refused, setRefused] = useState<string | null>(null);
  useStream(
    view,
    () => ({
      ...request,
      source: {
        kind: "exec",
        capability: session.binding.name,
        name: objectName(place),
        pod: session.pod,
        container: session.container,
        // What the confirmation showed, field for field; the host refuses anything else.
        confirmed: { pod: session.pod, container: session.container, command: session.command },
      },
    }),
    (data) => {
      if (!isExtensionExecEvent(data)) return;
      if (data.event === "exit") setExit(data.code);
      else setOutput((all) => [...all, ...data.chunks]);
    },
    setEnd,
    setRefused,
  );
  const status = refused
    ? `The host refused the command: ${refused}`
    : exit !== null
      ? exit === 0
        ? "The command exited with code 0."
        : `The command exited with code ${exit}.`
      : end
        ? describeStreamEnd(end)
        : "Running…";
  return (
    <>
      <p className="extension-message" role="status">
        {status}
      </p>
      <p className="extension-message">
        <code className="extension-command">{session.command.map(commandArgument).join(" ")}</code>
      </p>
      {output.length > 0 && (
        <pre className="extension-pod-output" aria-label="Command output" tabIndex={0}>
          {output.map((chunk, index) => (
            <span key={index} data-stream={chunk.stream}>
              {/* Said in a word, not only by its tint. */}
              {chunk.stream === "stderr" && <span className="extension-pod-stream">stderr </span>}
              {plainTextKeepingLines(chunk.text)}
            </span>
          ))}
        </pre>
      )}
    </>
  );
}

/** Output text with its line breaks kept and every other control or format character escaped. */
function plainTextKeepingLines(text: string): string {
  return text.split("\n").map(plainText).join("\n");
}

/** A forward: where it listens, and where it reaches. */
function ForwardSession({
  view,
  request,
  place,
  session,
}: {
  view: ExtensionView;
  request: Request;
  place: PodPlace;
  session: Extract<Session, { kind: "forward" }>;
}) {
  const [ready, setReady] = useState<Extract<ExtensionForwardEvent, { event: "ready" }> | null>(null);
  const [failed, setFailed] = useState<{ count: number; message: string } | null>(null);
  const [end, setEnd] = useState<ExtensionStreamEnd | null>(null);
  const [refused, setRefused] = useState<string | null>(null);
  useStream(
    view,
    () => ({
      ...request,
      source: {
        kind: "portForward",
        capability: session.binding.name,
        name: objectName(place),
        ...(session.pod ? { pod: session.pod } : {}),
        ...(session.service ? { service: session.service } : {}),
      },
    }),
    (data) => {
      if (!isExtensionForwardEvent(data)) return;
      if (data.event === "ready") setReady(data);
      else setFailed((was) => ({ count: (was?.count ?? 0) + data.count, message: data.message }));
    },
    setEnd,
    setRefused,
  );
  if (refused) return <p className="extension-message" role="status">The host refused the forward: {refused}</p>;
  if (end) return <p className="extension-message" role="status">{describeStreamEnd(end)}</p>;
  if (!ready) return <p className="extension-message" role="status">Opening a local port…</p>;
  return (
    <>
      <p className="extension-message" role="status">
        Forwarding <code>127.0.0.1:{ready.localPort}</code> to <code>{plainText(ready.pod)}</code> port {ready.port}
        {ready.service ? ` (through Service ${plainText(ready.service)})` : ""}. The forward ends when you stop it or close this view.
      </p>
      {failed && (
        // The cluster refused connections through it; the forward itself still listens.
        <p className="extension-message extension-stale-notice" role="alert">
          {failed.count === 1 ? "1 connection through it failed" : `${failed.count.toLocaleString("en-US")} connections through it failed`}
          , the last because: {plainText(failed.message)}
        </p>
      )}
    </>
  );
}

/**
 * Open one stream for as long as the component is mounted: the session's
 * lifetime is the pane's, and unmounting cancels it. A refused open says why.
 */
function useStream(
  view: ExtensionView,
  request: () => Parameters<ExtensionView["open"]>[0],
  onData: (data: unknown) => void,
  onEnd: (end: ExtensionStreamEnd) => void,
  onRefused: (why: string) => void,
) {
  const handlers = useRef({ onData, onEnd, onRefused });
  handlers.current = { onData, onEnd, onRefused };
  const ask = useRef(request);
  useEffect(() => {
    let cancelled = false;
    let stream: ExtensionStream | null = null;
    view
      .open(ask.current(), {
        onData: (data) => !cancelled && handlers.current.onData(data),
        onEnd: (end) => !cancelled && handlers.current.onEnd(end),
      })
      .then(
        (opened) => {
          stream = opened;
          if (cancelled) void opened.cancel();
        },
        (error: unknown) => !cancelled && handlers.current.onRefused(error instanceof Error ? error.message : String(error)),
      );
    return () => {
      cancelled = true;
      void stream?.cancel();
    };
  }, [view]);
}
