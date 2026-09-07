import type { ReactNode } from "react";
import {
  ageFromTimestamp,
  asArray,
  asRecord,
  containerLastRestartTime,
  containerStateText,
  envText,
  latestRestartTime,
  mountText,
  orderPodConditions,
  podContainerStatuses,
  portText,
  probeChips,
  resourceStatusLine,
  resourceSummary,
  resourceText,
  restartTotal,
  str,
  summarizeAffinity,
  timestampWithAge,
  tolerationText,
  type Condition,
  type K8sObject,
} from "@srelens/core";
import {
  EmptyState,
  KV,
  PairList,
  StatusPill,
  SubHead,
  Table,
  type Column,
} from "@srelens/ui-kit";
import { ForwardAction } from "../forwards/ForwardAction";
import { Section } from "./Section";
import { ConditionsSection, StringList } from "./sections";
import type { DetailFact, FactsFor } from "./facts";

/**
 * Kubernetes' own labels for a pod volume's source kind, keyed on which field
 * of the volume (besides `name`) is actually set — matches classic's own
 * table (`VOLUME_TYPE_LABELS` in `ResourceOverview.tsx`).
 */
const VOLUME_TYPE_LABELS: Record<string, string> = {
  persistentVolumeClaim: "Persistent Volume Claim",
  emptyDir: "Empty Dir",
  secret: "Secret",
  configMap: "Config Map",
  projected: "Projected",
  hostPath: "Host Path",
  downwardAPI: "Downward API",
  nfs: "NFS",
  csi: "CSI",
};


/**
 * What a pod volume points at, as plain text — "PersistentVolumeClaim/data",
 * a host path, an NFS export, the CSI driver, or the config maps/secrets a
 * projected volume merges. Classic renders these through `ResourceLink`,
 * which navigates; nothing here can, since `PaneBody` has no navigation
 * contract yet (see the task report), so the same facts are shown inert —
 * still named, just not clickable.
 */
function volumeSourceText(volume: Record<string, unknown>): string {
  const pvc = asRecord(volume.persistentVolumeClaim);
  if (pvc.claimName) return `PersistentVolumeClaim/${str(pvc.claimName)}`;
  const configMap = asRecord(volume.configMap);
  if (configMap.name) return `ConfigMap/${str(configMap.name)}`;
  const secret = asRecord(volume.secret);
  if (secret.secretName) return `Secret/${str(secret.secretName)}`;
  if (volume.hostPath) return str(asRecord(volume.hostPath).path);
  if (volume.nfs) {
    const nfs = asRecord(volume.nfs);
    return `${str(nfs.server)}:${str(nfs.path)}`;
  }
  if (volume.csi) return str(asRecord(volume.csi).driver);
  if (volume.projected) {
    const sources = asArray(asRecord(volume.projected).sources).map(asRecord);
    const names = sources.flatMap((source) => {
      const projectedConfigMap = asRecord(source.configMap);
      if (projectedConfigMap.name) return [`ConfigMap/${str(projectedConfigMap.name)}`];
      const projectedSecret = asRecord(source.secret);
      if (projectedSecret.name) return [`Secret/${str(projectedSecret.name)}`];
      return [];
    });
    return names.length > 0 ? names.join(", ") : `${sources.length} projected sources`;
  }
  if (volume.emptyDir) return str(asRecord(volume.emptyDir).medium) || "Node temporary storage";
  return "—";
}

function volumeTypeLabel(volume: Record<string, unknown>): string {
  const type = Object.keys(volume).find((key) => key !== "name") ?? "unknown";
  return VOLUME_TYPE_LABELS[type] ?? type;
}

/** The images a pod runs, each named once however many containers share it. */
function imagesOf(containers: unknown): string[] {
  return [...new Set(asArray(containers).map((c) => str(asRecord(c).image)).filter(Boolean))];
}

/** One image reads as a fact; several read as a list. */
function ImageValue({ images }: { images: string[] }) {
  return images.length === 1 ? <span className="font-mono">{images[0]}</span> : <StringList items={images} />;
}

/**
 * The pod's facts, in the order the design's own Pod frame reads them:
 * Status, Node, Pod IP, QoS class, Service account, Containers ready,
 * Restarts, Controlled by, Created, Image.
 *
 * DATA, so each of the two detail screens can lay the list out its own way —
 * the peek down one column, the full tab across three. Neither draws the
 * other's markup; both draw this list. It used to be a `Section` rendered
 * here, which is why the tab had to restyle it from above (`FactGrid`). (#331)
 *
 * No heading, either way. The design heads the first block of a detail with
 * nothing — the pane's header has already given the name, the kind and the
 * namespace — and there is no `Name` fact either, which repeated that header
 * verbatim.
 *
 * The frame's ten facts are not everything srelens knows, and the extras it
 * carries over from classic (Pod IPs, Priority class, Runtime class, Image
 * pull secrets, Last restart) stay: the frame is a design, not a schema. They
 * sit beside their own kin rather than in a heap at the end, so the reading
 * order the frame set survives them.
 *
 * `Image` is new here. It used to live only on the Containers pane, which
 * meant the one question asked of a pod more often than any other — what is
 * it running — took a tab change to answer.
 *
 * The status word comes from core's `resourceStatusLine`, the same reading the
 * pane's header and the list row use. Nothing here derives a second opinion:
 * a pod whose container sits in `CrashLoopBackOff` still reports phase
 * "Running", so a fact list reading `status.phase` by itself would print
 * "Running" under a header saying "CrashLoopBackOff".
 *
 * Namespace, Node, Service account, Priority class, Runtime class, Controlled
 * by and Image pull secrets are `ResourceLink`s in classic that navigate;
 * they render here as plain text (see the task report for the full list).
 */
export const podFacts: FactsFor = ({ object }) => {
  const meta = object.metadata ?? {};
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  const owners = meta.ownerReferences ?? [];
  const podIPs = asArray(status.podIPs)
    .map((p) => str(asRecord(p).ip))
    .filter(Boolean);
  const imagePullSecrets = asArray(spec.imagePullSecrets)
    .map((secret) => str(asRecord(secret).name))
    .filter(Boolean);
  const containerStatuses = asArray(status.containerStatuses).map(asRecord);
  // Core's read, not a second one: the full tab's Restarts tile shows the very
  // same number, and two reduces over one field is how two surfaces start
  // disagreeing about how often a pod has restarted.
  const allContainerStatuses = podContainerStatuses(status);
  const podRestartCount = restartTotal(allContainerStatuses);
  const podLastRestart = latestRestartTime(allContainerStatuses);
  const containersReady = containerStatuses.filter((c) => c.ready === true).length;
  const created = str(meta.creationTimestamp);
  const nodeName = str(spec.nodeName);
  const podIP = str(status.podIP);
  const serviceAccountName = str(spec.serviceAccountName);
  const priorityClassName = str(spec.priorityClassName);
  const runtimeClassName = str(spec.runtimeClassName);
  const qosClass = str(status.qosClass);
  const images = imagesOf(spec.containers);
  const statusLine = resourceStatusLine("Pod", object);

  const facts: DetailFact[] = [];
  if (statusLine) {
    facts.push({
      label: "Status",
      value: <StatusPill status={statusLine.status} kind={statusLine.health} tinted />,
    });
  }
  if (nodeName) facts.push({ label: "Node", value: nodeName, mono: true });
  if (podIP) facts.push({ label: "Pod IP", value: podIP, mono: true });
  if (podIPs.length > 0) facts.push({ label: "Pod IPs", value: <StringList items={podIPs} /> });
  if (qosClass) facts.push({ label: "QoS class", value: qosClass });
  if (serviceAccountName) facts.push({ label: "Service account", value: serviceAccountName, mono: true });
  if (priorityClassName) facts.push({ label: "Priority class", value: priorityClassName, mono: true });
  if (runtimeClassName) facts.push({ label: "Runtime class", value: runtimeClassName, mono: true });
  if (imagePullSecrets.length > 0) {
    facts.push({
      label: "Image pull secrets",
      value: <StringList items={imagePullSecrets.map((name) => `Secret/${name}`)} />,
    });
  }
  // No container status at all means the kubelet has not reported yet:
  // "0 of 0" and "0 restarts" would read as facts where there is only an
  // absence.
  if (containerStatuses.length > 0) {
    facts.push({ label: "Containers ready", value: `${containersReady} of ${containerStatuses.length}` });
  }
  if (allContainerStatuses.length > 0) facts.push({ label: "Restarts", value: str(podRestartCount) });
  if (podLastRestart) {
    facts.push({ label: "Last restart", value: timestampWithAge(podLastRestart, Date.now()) });
  }
  if (owners.length > 0) {
    facts.push({
      label: "Controlled by",
      value: <StringList items={owners.map((o) => `${o.kind}/${o.name}`)} />,
    });
  }
  if (meta.namespace) facts.push({ label: "Namespace", value: str(meta.namespace), mono: true });
  if (created) facts.push({ label: "Created", value: `${ageFromTimestamp(created, Date.now())} ago` });
  if (images.length > 0) facts.push({ label: "Image", value: <ImageValue images={images} /> });
  return facts;
};

/**
 * Where and how the pod is placed — classic's "Scheduling" section, shown
 * only when there is something to say (a node, a selector, an affinity rule
 * or a toleration), same as classic's `hasScheduling` gate.
 */
function SchedulingSection({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const nodeSelector = (spec.nodeSelector ?? {}) as Record<string, string>;
  const affinityLines = summarizeAffinity(asRecord(spec.affinity));
  const tolerations = asArray(spec.tolerations);
  const hasScheduling =
    !!spec.nodeName || Object.keys(nodeSelector).length > 0 || affinityLines.length > 0 || tolerations.length > 0;

  if (!hasScheduling) return null;

  return (
    <Section title="Scheduling">
      <KV k="Node" v={spec.nodeName ? str(spec.nodeName) : "Not scheduled"} mono={!!spec.nodeName} />
      {Object.keys(nodeSelector).length > 0 && (
        <KV k="Node selector" v={<PairList pairs={Object.entries(nodeSelector)} breakValues />} />
      )}
      {affinityLines.length > 0 && <KV k="Affinity" v={<StringList items={affinityLines} />} />}
      {tolerations.length > 0 && (
        <KV k="Tolerations" v={<StringList items={tolerations.map(tolerationText)} />} />
      )}
    </Section>
  );
}

const VOLUME_COLUMNS: Column<Record<string, unknown>>[] = [
  { key: "name", header: "Name", render: (v) => <span className="font-mono">{str(v.name)}</span> },
  { key: "type", header: "Type", render: volumeTypeLabel },
  { key: "source", header: "Source", render: volumeSourceText },
];

/**
 * The pod's own volumes — classic's "Pod Volumes" table. The "Source" column
 * is one of the plain-text substitutions for a `ResourceLink`: it names the
 * PersistentVolumeClaim/ConfigMap/Secret a volume points at without being
 * able to open it (see the task report).
 */
function PodVolumesSection({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const volumes = asArray(spec.volumes).map(asRecord);
  if (volumes.length === 0) return null;
  return (
    <Section title="Pod Volumes">
      <Table columns={VOLUME_COLUMNS} data={volumes} getRowKey={(v) => str(v.name)} />
    </Section>
  );
}



/**
 * A pod's Details pane, as a flat run of blocks divided by hairline rules —
 * Scheduling and Pod Volumes, then Conditions.
 *
 * The lead fact list is NOT here: it is data ({@link podFacts}), and the
 * screen drawing the pane lays it out above this in its own layout.
 *
 * Every block is a sibling of every other, with nothing wrapped around any of
 * them: `.section + .section` is what draws the rule between two blocks, so a
 * div — or a bare `LoadingState` — between two of them quietly removes the
 * rule on both sides. A block with nothing to say renders nothing at all, and
 * the rules land in the right places on their own.
 *
 * Conditions close the body rather than following the facts immediately as the
 * design frame draws them. The frame has no Scheduling or Volumes block to
 * place, and `GenericBody` already ends every other kind's Details this way —
 * so a reader moving between kinds finds the same block in the same place.
 *
 * Labels and Annotations are not here at all: they close every kind, so the
 * host places them (see `GenericBody`'s note). The peek stacks them under
 * this; the full tab reads them side by side.
 *
 * The container list lives on the Containers pane instead
 * (`PodContainersBody`, below), which is what `panes.containers` exists for.
 */
export function PodDetailsBody({ object }: { object: K8sObject }) {
  const status = asRecord(object.status);
  const conditions = asArray(status.conditions) as unknown as Condition[];

  const sections: ReactNode[] = [
    <SchedulingSection key="scheduling" object={object} />,
    <PodVolumesSection key="volumes" object={object} />,
    <ConditionsSection key="conditions" conditions={orderPodConditions(conditions)} />,
  ];

  return <>{sections}</>;
}

/** The highest port a TCP socket can bind. */
const MAX_PORT = 65_535;

/** Where a container's ports are being drawn, so a forward opened from one
 *  knows what it is a port OF. A container is not a forward target; the pod
 *  around it is. */
interface PortsSubject {
  context: string;
  namespace: string;
  /** The POD's name — `kubectl port-forward pod/<name>`. */
  pod: string;
  /** The container the ports belong to, which is what tells two identical
   *  port numbers on one pod apart in an accessible name. */
  container: string;
}

/**
 * A container's ports, each one a way into a forward.
 *
 * THE ONE RENDERING OF A CONTAINER'S PORTS, drawn by both surfaces that show
 * them — the peek's Containers pane down a column, the full tab's table across
 * a cell. They used to be two inert renderings of core's `portText`: a
 * `StringList` in one and `ports.map(portText).join(", ")` in the other. The
 * words are still core's, unchanged; what changed is that each one is now the
 * control that forwards it, rather than a string with no way to act on it.
 *
 * A port is a chip and not a button beside a string on purpose: a row of
 * "Forward" buttons next to `http: 8080/TCP, metrics: 9090/TCP` would make the
 * reader match a verb to a number by position.
 *
 * A port with no number a socket could bind — a malformed spec — renders as
 * the same text it always did rather than as a control that could not work.
 */
function ContainerPorts({
  ports,
  subject,
}: {
  ports: Record<string, unknown>[];
  subject?: PortsSubject;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      {ports.map((port, i) => {
        const text = portText(port);
        const number = Number(str(port.containerPort));
        const forwardable =
          subject && Number.isInteger(number) && number > 0 && number <= MAX_PORT;
        return forwardable ? (
          <ForwardAction
            key={`${text}-${i}`}
            context={subject.context}
            namespace={subject.namespace}
            kind="Pod"
            name={subject.pod}
            remotePort={number}
            // Contains the chip's own words, so the spoken name and the drawn
            // one are one label — and carries the container, because two
            // containers in a pod may well publish the same port.
            label={`Forward ${text} on ${subject.container}`}
          >
            {text}
          </ForwardAction>
        ) : (
          <span key={`${text}-${i}`}>{text}</span>
        );
      })}
    </div>
  );
}

/**
 * One container's block — app, init or ephemeral. State and restart count come
 * from its `containerStatuses` entry (absent while the pod is still being
 * scheduled, e.g. an init container that hasn't started); ports, probes,
 * environment and mounts come from the spec and are omitted outright, not
 * shown empty, when the container has none.
 *
 * "Last restart" and "Running since" are distinct facts, not one shown two
 * ways: `containerLastRestartTime` reads `lastState` (the PREVIOUS run's
 * termination), `runningSince` reads `state.running.startedAt` (when the
 * CURRENT run began) — a reader diagnosing a crash loop needs both.
 * "Debugging" (`targetContainerName`) only appears on an ephemeral
 * container, naming which container its debug session is attached to.
 */
function ContainerCard({
  container,
  status,
  subject,
}: {
  container: Record<string, unknown>;
  status?: Record<string, unknown>;
  /** Absent only where the pane cannot say which pod in which cluster it is
   *  drawing, which is what a forward would need. */
  subject?: Omit<PortsSubject, "container">;
}) {
  const name = str(container.name);
  const state = status ? containerStateText(status) : undefined;
  const targetContainerName = str(container.targetContainerName);
  const restarts = status?.restartCount;
  const lastRestart = status ? containerLastRestartTime(status) : "";
  const runningSince = status ? str(asRecord(asRecord(status.state).running).startedAt) : "";
  const image = str(container.image);
  const ports = asArray(container.ports).map(asRecord);
  const env = asArray(container.env);
  const mounts = asArray(container.volumeMounts);
  const resources = asRecord(container.resources);
  const requests = asRecord(resources.requests);
  const limits = asRecord(resources.limits);
  const liveness = asRecord(container.livenessProbe);
  const readiness = asRecord(container.readinessProbe);
  const startup = asRecord(container.startupProbe);
  const command = [...asArray(container.command), ...asArray(container.args)].map(str).join(" ");

  return (
    <div className="flex flex-col gap-1.5">
      <SubHead>
        <span className="flex items-center gap-2">
          {name}
          {state && <StatusPill status={state.text} kind={state.kind} tinted />}
        </span>
      </SubHead>
      {targetContainerName && <KV k="Debugging" v={targetContainerName} mono />}
      {restarts != null && <KV k="Restarts" v={str(restarts)} />}
      {lastRestart && <KV k="Last restart" v={timestampWithAge(lastRestart, Date.now())} />}
      {runningSince && <KV k="Running since" v={timestampWithAge(runningSince, Date.now())} />}
      {image && <KV k="Image" v={image} mono />}
      {ports.length > 0 && (
        <KV
          k="Ports"
          v={<ContainerPorts ports={ports} subject={subject && { ...subject, container: name }} />}
        />
      )}
      {env.length > 0 && <KV k="Environment" v={<StringList items={env.map(envText)} />} />}
      {mounts.length > 0 && <KV k="Mounts" v={<StringList items={mounts.map(mountText)} />} />}
      {Object.keys(liveness).length > 0 && <KV k="Liveness" v={<StringList items={probeChips(liveness)} />} />}
      {Object.keys(readiness).length > 0 && <KV k="Readiness" v={<StringList items={probeChips(readiness)} />} />}
      {Object.keys(startup).length > 0 && <KV k="Startup" v={<StringList items={probeChips(startup)} />} />}
      {command && <KV k="Command" v={command} mono />}
      {Object.keys(requests).length > 0 && <KV k="Requests" v={resourceText(requests)} />}
      {Object.keys(limits).length > 0 && <KV k="Limits" v={resourceText(limits)} />}
    </div>
  );
}

function ContainerGroup({
  title,
  containers,
  statuses,
  subject,
}: {
  title: string;
  containers: Record<string, unknown>[];
  statuses: Map<string, Record<string, unknown>>;
  subject?: Omit<PortsSubject, "container">;
}) {
  if (containers.length === 0) return null;
  return (
    <Section title={title}>
      <div className="flex flex-col gap-4">
        {containers.map((c) => (
          <ContainerCard
            key={str(c.name)}
            container={c}
            status={statuses.get(str(c.name))}
            subject={subject}
          />
        ))}
      </div>
    </Section>
  );
}

function statusesByName(list: unknown): Map<string, Record<string, unknown>> {
  return new Map(asArray(list).map((s) => [str(asRecord(s).name), asRecord(s)]));
}

/**
 * What a container's ports can be forwarded AS, or `undefined` where they
 * cannot be forwarded at all.
 *
 * One derivation for both surfaces, so the peek's pane and the tab's table
 * cannot come to disagree about which pod a port belongs to. A pod with no
 * namespace or no name — or a screen with no cluster in focus — has nothing to
 * point a forward at, and the ports render as the text they always were.
 */
function forwardSubject(
  object: K8sObject,
  context: string,
): Omit<PortsSubject, "container"> | undefined {
  const namespace = str(object.metadata?.namespace);
  const pod = str(object.metadata?.name);
  return context && namespace && pod ? { context, namespace, pod } : undefined;
}

/**
 * A pod's Containers pane: every container named, its runtime state and
 * restart count, its ports, probes, environment and mounts. Ported from
 * classic's `ContainerCard`/`PodDetailView` — the largest single body in
 * `ResourceOverview.tsx` — onto kit components. The interactive port-forward
 * affordance classic offers inline IS wired now, through
 * {@link ContainerPorts}: each port is the control that forwards it, into
 * §A.4's dialog.
 *
 * Flat blocks, like the Details pane beside it: the two panes are read in the
 * same 352px column, and a card here beside a rule there is two answers to
 * one question.
 */
export function PodContainersBody({ object, context }: { object: K8sObject; context: string }) {
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  const containers = asArray(spec.containers).map(asRecord);
  const initContainers = asArray(spec.initContainers).map(asRecord);
  const ephemeralContainers = asArray(spec.ephemeralContainers).map(asRecord);

  if (containers.length === 0 && initContainers.length === 0 && ephemeralContainers.length === 0) {
    return <EmptyState title="No containers" />;
  }

  const containerStatuses = statusesByName(status.containerStatuses);
  const subject = forwardSubject(object, context);

  return (
    <>
      <ContainerGroup
        title="Init containers"
        containers={initContainers}
        statuses={statusesByName(status.initContainerStatuses)}
        subject={subject}
      />
      {/* Always open, never conditional on this pod having init or ephemeral
          containers beside it. It is the pane's subject — a reader who clicks
          a tab named Containers and is shown one word and a caret has been
          answered with nothing — and the fold memory is keyed per KIND, so a
          default that varied with the subject would make the stored document
          mean different things depending on which pod was on screen when the
          reader clicked: open on a pod with an init group, shut it, and the
          entry is dropped as "back to default" — leaving the next pod without
          one open. The init and ephemeral groups keep the shut rule; they are
          extras, and the pane says something either way. */}
      <Section title="Containers" defaultOpen>
        {containers.length === 0 ? (
          <EmptyState title="No containers" />
        ) : (
          <div className="flex flex-col gap-4">
            {containers.map((c) => (
              <ContainerCard
                key={str(c.name)}
                container={c}
                status={containerStatuses.get(str(c.name))}
                subject={subject}
              />
            ))}
          </div>
        )}
      </Section>
      <ContainerGroup
        title="Ephemeral containers"
        containers={ephemeralContainers}
        statuses={statusesByName(status.ephemeralContainerStatuses)}
        subject={subject}
      />
    </>
  );
}

/** One row of the Overview's containers table: the spec entry beside the
 *  status the kubelet reports for it, matched by name. */
interface ContainerRow {
  container: Record<string, unknown>;
  status?: Record<string, unknown>;
}

/**
 * The one probe a summary row has room for.
 *
 * Readiness first because it is the probe that decides whether the container
 * is in service, which is what a reader scanning a row is asking about;
 * liveness and startup stand in when there is no readiness probe, so a
 * container that has one of them still says so. `probeChips` is core's single
 * rendering of a probe — the peek's Containers pane prints the very same
 * clauses down a column, and a second phrasing here would be a second answer
 * to "what does it check".
 */
function probeSummary(container: Record<string, unknown>): string {
  for (const key of ["readinessProbe", "livenessProbe", "startupProbe"]) {
    const probe = asRecord(container[key]);
    if (Object.keys(probe).length > 0) return probeChips(probe).join(" ");
  }
  return "—";
}

/**
 * The summary table's columns, built against the pod they are drawn for.
 *
 * A function rather than a module-level constant because ONE of these cells is
 * interactive — a port has to know which pod it would forward — and the reason
 * the rest of this app keeps its columns module-level (so sort and filter read
 * the same strings the reader sees) is not in play for a cell holding
 * controls: that column is neither sortable nor filterable, and says so.
 */
function containerColumns(subject?: Omit<PortsSubject, "container">): Column<ContainerRow>[] {
  return [
  { key: "name", header: "Name", render: (r) => <span className="font-mono">{str(r.container.name)}</span> },
  { key: "image", header: "Image", render: (r) => <span className="font-mono">{str(r.container.image) || "—"}</span> },
  {
    key: "ports",
    header: "Ports",
    // Controls, not text: there is nothing here for a comparator to order or a
    // search to match, and a header that sorted by rendered nodes would sort
    // by nothing at all.
    sortable: false,
    filterable: false,
    render: (r) => {
      const ports = asArray(r.container.ports).map(asRecord);
      if (ports.length === 0) return "—";
      return (
        <ContainerPorts
          ports={ports}
          subject={subject && { ...subject, container: str(r.container.name) }}
        />
      );
    },
  },
  // `cpu · memory` in one cell, the design's own form — `resourceSummary` is
  // core's compact rendering of the very two fields the peek's rows name in
  // full through `resourceText`.
  {
    key: "requests",
    header: "Requests",
    render: (r) => resourceSummary(asRecord(asRecord(r.container.resources).requests)),
  },
  {
    key: "limits",
    header: "Limits",
    render: (r) => resourceSummary(asRecord(asRecord(r.container.resources).limits)),
  },
  { key: "probe", header: "Probe", render: (r) => probeSummary(r.container) },
  {
    key: "state",
    header: "State",
    // The word AND its tone are `containerStateText`'s. Nothing here pairs a
    // state with a colour: that table exists once, in core, and every
    // container state in srelens is read through it.
    render: (r) => {
      if (!r.status) return "—";
      const state = containerStateText(r.status);
      return <StatusPill status={state.text} kind={state.kind} tinted />;
    },
  },
  ];
}

/**
 * A pod's containers as one table, the way the full tab reads them inline on
 * Overview — name, image, ports, requests, limits, probe, state.
 *
 * Deliberately NOT a second {@link PodContainersBody}. That pane is one block
 * per container and prints everything a container has: its environment, its
 * mounts, its command, all three probes, when the current run started and when
 * the last one ended. This is a summary a reader scans across, which is what
 * the design asks a page-wide surface for, and it is why the peek keeps
 * Containers as a tab of its own while the tab folds it into Overview.
 *
 * Every cell is a shared derivation — `portText`, `resourceSummary`,
 * `probeChips`, `containerStateText` — so the two renderings cannot come to
 * disagree about what a container is doing, only about how much of it they
 * show.
 *
 * Init and ephemeral containers are not here. The design's table is the pod's
 * app containers, and the peek's pane is where a finished migration step or an
 * attached debug container is read in full.
 */
export function PodContainersTable({ object, context }: { object: K8sObject; context: string }) {
  const spec = asRecord(object.spec);
  const containers = asArray(spec.containers).map(asRecord);
  if (containers.length === 0) return null;
  const statuses = statusesByName(asRecord(object.status).containerStatuses);
  const rows: ContainerRow[] = containers.map((container) => ({
    container,
    status: statuses.get(str(container.name)),
  }));
  return (
    <Section title="Containers">
      <Table
        columns={containerColumns(forwardSubject(object, context))}
        data={rows}
        getRowKey={(r) => str(r.container.name)}
      />
    </Section>
  );
}
