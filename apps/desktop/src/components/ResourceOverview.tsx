import React, { useEffect, useMemo, useState } from "react";
import { ArrowLeftRight, ChevronDown, ChevronUp, ScrollText, SquareTerminal } from "lucide-react";
import type { X509Certificate } from "@peculiar/x509";
import { getObject, getSecret, type K8sObject } from "../lib/manifest";
import { listEndpointSlices } from "../lib/network";
import { podsForPvc, formatStorageSize } from "../lib/storage";
import { bindingsForServiceAccount, podsForServiceAccount, type SaBinding } from "../lib/rbac";
import { updateConfigData } from "../lib/actions";
import { useAccess, denyReason, reportActionError, type AccessCheck } from "../lib/access";
import { describeError } from "../lib/errors";
import {
  Spinner,
  StatusPill,
  Badge,
  Button,
  Table,
  IconButton,
  type StatusKind,
  type BadgeVariant,
  type Column,
} from "../ui";
import { DeployRevisions, ManagedPods, CronJobJobs } from "./WorkloadRelations";
import { MetricsPanel } from "./MetricsPanel";
import { ForwardDialog } from "./ForwardDialog";
import {
  isNavigableResourceKind,
  targetNamespace,
  type OpenResource,
  type ResourceTarget,
} from "../lib/resourceNavigation";

/* ------------------------------------------------------------------ */
/* small value helpers                                                 */
/* ------------------------------------------------------------------ */

/** Relative age from an ISO timestamp, e.g. "5d", "3h", "10m". */
export function ageFromTimestamp(iso?: string, now: number = Date.now()): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  const secs = Math.max(0, Math.floor((now - then) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

/** Human-readable duration between two ISO timestamps, e.g. "2m 30s". */
export function durationBetween(startIso?: string, endIso?: string): string {
  if (!startIso || !endIso) return "—";
  const secs = Math.max(0, Math.floor((new Date(endIso).getTime() - new Date(startIso).getTime()) / 1000));
  if (Number.isNaN(secs)) return "—";
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  if (mins < 60) return remSecs ? `${mins}m ${remSecs}s` : `${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  return remMins ? `${hours}h ${remMins}m` : `${hours}h`;
}

/** Absolute, human-readable timestamp, e.g. "Jun 10, 2026, 12:52:33 PM". */
export function absoluteTimestamp(iso?: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  });
}

function asRecord(v: unknown): Record<string, unknown> {
  return v && typeof v === "object" ? (v as Record<string, unknown>) : {};
}
function asArray(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}
function str(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  return String(v);
}
function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

type Pair = [label: string, value: React.ReactNode];

/** Render a definition-list grid, skipping rows whose value is empty. */
/** Property rows (label/value) for a detail section. Empty values are dropped. */
export function KV({ pairs }: { pairs: Pair[] }) {
  const rows = pairs.filter(([, v]) => v !== null && v !== undefined && v !== "" && v !== "—");
  if (rows.length === 0) return null;
  return (
    <dl className="fl-kv">
      {rows.map(([label, value]) => (
        <React.Fragment key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </React.Fragment>
      ))}
    </dl>
  );
}

/** A titled card section in a detail view (accent-barred header + bordered surface). */
export function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="fl-detail-section">
      <h4 className="fl-detail-section__title">{title}</h4>
      {children}
    </section>
  );
}

function ResourceLink({
  target,
  onOpenResource,
  children,
}: {
  target: ResourceTarget;
  onOpenResource?: OpenResource;
  children?: React.ReactNode;
}) {
  const content = children ?? target.name;
  if (!onOpenResource || !target.name || !isNavigableResourceKind(target.kind))
    return <span className="fl-mono">{content}</span>;
  return (
    <button
      type="button"
      className="fl-link fl-mono"
      aria-label={`Open ${target.kind} ${target.name}`}
      title={`Open ${target.kind}`}
      onClick={() => onOpenResource(target)}
    >
      {content}
    </button>
  );
}

function LinkedResources({
  targets,
  onOpenResource,
}: {
  targets: ResourceTarget[];
  onOpenResource?: OpenResource;
}) {
  return (
    <span>
      {targets.map((target, index) => (
        <React.Fragment key={`${target.kind}/${target.namespace ?? ""}/${target.name}/${index}`}>
          {index > 0 && ", "}
          <ResourceLink target={target} onOpenResource={onOpenResource}>
            {target.kind}/{target.name}
          </ResourceLink>
        </React.Fragment>
      ))}
    </span>
  );
}

/** Render a key/value map (labels, annotations, selectors) as chips. */
function Chips({ map }: { map?: Record<string, string> }) {
  const entries = Object.entries(map ?? {});
  if (entries.length === 0) return <span className="fl-detail-empty">None</span>;
  return (
    <div className="fl-chips">
      {entries.map(([k, v]) => (
        <span className="fl-chip" key={k} title={`${k}: ${v}`}>
          <span className="fl-chip__key">{k}</span>
          {v !== "" && <span className="fl-chip__val">{v}</span>}
        </span>
      ))}
    </div>
  );
}

/**
 * A labels/annotations/selector map shown as a collapsed count summary
 * ("42 Labels ⌄") that expands to the chips on click — so large maps (node
 * feature labels, last-applied annotations) don't dominate the detail panel.
 * The single source of truth for how these maps render across every detail view.
 */
function ChipMap({ map, singular }: { map?: Record<string, string>; singular: string }) {
  const count = Object.keys(map ?? {}).length;
  if (count === 0) return <span className="fl-detail-empty">None</span>;
  return (
    <Expandable summary={plural(count, singular)}>
      <Chips map={map} />
    </Expandable>
  );
}

/**
 * A count summary that expands to its full content on click — the srelens
 * idiom for long label/annotation/toleration lists that would dominate the
 * panel ("6 Labels ⌄").
 */
function Expandable({ summary, children }: { summary: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="fl-expandable">
      <button
        type="button"
        className="fl-expandable__toggle"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        {summary}
        <span className="fl-expandable__caret">
          {open ? <ChevronUp aria-hidden="true" /> : <ChevronDown aria-hidden="true" />}
        </span>
      </button>
      {open && <div className="fl-expandable__body">{children}</div>}
    </div>
  );
}

function CollapsibleText({
  text,
  label,
  lines = 4,
  muted = false,
}: {
  text: string;
  label: string;
  lines?: number;
  muted?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const long = text.length > 120 || text.split("\n").length > lines;
  if (!long) return <span className={`fl-mono${muted ? " fl-command" : ""}`}>{text}</span>;
  return (
    <div className="fl-collapsible-value">
      <span
        className={`fl-mono fl-collapsible-value__content${muted ? " fl-command" : ""}${expanded ? "" : " fl-collapsible-value__content--collapsed"}`}
        style={{ "--fl-collapse-lines": lines } as React.CSSProperties}
      >
        {text}
      </span>
      <button
        type="button"
        className="fl-collapsible-value__toggle"
        aria-expanded={expanded}
        aria-label={`${expanded ? "Collapse" : "Show full"} ${label}`}
        onClick={() => setExpanded((current) => !current)}
      >
        {expanded ? "Collapse" : "Show full"}
      </button>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* conditions                                                          */
/* ------------------------------------------------------------------ */

interface Condition {
  type: string;
  status: string;
  reason?: string;
  message?: string;
  lastTransitionTime?: string;
}

function conditionKind(c: Condition): StatusKind {
  const negative = /Pressure|Unavailable|Failed|Dangling|NetworkUnavailable/i.test(c.type);
  if (c.status === "Unknown") return "warning";
  const good = c.status === "True" ? !negative : negative;
  return good ? "success" : "danger";
}

function conditionBadgeVariant(c: Condition): BadgeVariant {
  if (c.status === "Unknown") return "warning";
  const negative = /Pressure|Unavailable|Failed|Failure|Dangling/i.test(c.type);
  if (c.status !== "True") return negative ? "success" : "neutral";
  if (/Progressing/i.test(c.type)) return "info";
  return negative ? "danger" : "success";
}

// The pod lifecycle, in the order kubelet reports it.
const POD_CONDITION_ORDER = ["PodScheduled", "Initialized", "ContainersReady", "Ready"];

/**
 * Sort pod conditions into lifecycle order (PodScheduled → Initialized →
 * ContainersReady → Ready); any other condition types keep their relative order
 * after the known lifecycle ones.
 */
export function orderPodConditions(conditions: Condition[]): Condition[] {
  const rank = (type: string) => {
    const index = POD_CONDITION_ORDER.indexOf(type);
    return index === -1 ? POD_CONDITION_ORDER.length : index;
  };
  return conditions
    .map((condition, index) => ({ condition, index }))
    .sort((a, b) => rank(a.condition.type) - rank(b.condition.type) || a.index - b.index)
    .map(({ condition }) => condition);
}

/**
 * One line per affinity type in use, e.g. "Node affinity: 2 required, 1
 * preferred". `nodeAffinity` counts `nodeSelectorTerms`; pod (anti-)affinity
 * count their rule arrays directly. Types with no rules are omitted.
 */
export function summarizeAffinity(affinity: Record<string, unknown>): string[] {
  const lines: string[] = [];
  const describe = (label: string, rule: Record<string, unknown>, requiredIsTerms: boolean) => {
    const required = requiredIsTerms
      ? asArray(asRecord(rule.requiredDuringSchedulingIgnoredDuringExecution).nodeSelectorTerms).length
      : asArray(rule.requiredDuringSchedulingIgnoredDuringExecution).length;
    const preferred = asArray(rule.preferredDuringSchedulingIgnoredDuringExecution).length;
    if (required === 0 && preferred === 0) return;
    const parts: string[] = [];
    if (required) parts.push(`${required} required`);
    if (preferred) parts.push(`${preferred} preferred`);
    lines.push(`${label}: ${parts.join(", ")}`);
  };
  describe("Node affinity", asRecord(affinity.nodeAffinity), true);
  describe("Pod affinity", asRecord(affinity.podAffinity), false);
  describe("Pod anti-affinity", asRecord(affinity.podAntiAffinity), false);
  return lines;
}

/** Conditions as a row of coloured badges (Pod/Deployment-style). */
function ConditionBadges({ conditions }: { conditions: Condition[] }) {
  if (conditions.length === 0) return <span className="fl-detail-empty">None</span>;
  return (
    <div className="fl-chips">
      {conditions.map((c) => (
        <Badge key={c.type} variant={conditionBadgeVariant(c)}>
          {c.type}
        </Badge>
      ))}
    </div>
  );
}

/** Conditions as a table (workload/node-style). */
function ConditionsTable({ conditions, now }: { conditions: Condition[]; now: number }) {
  if (conditions.length === 0) return null;
  const columns: Column<Condition>[] = [
    {
      key: "type",
      header: "Type",
      render: (c) => <StatusPill status={c.type} kind={conditionKind(c)} />,
    },
    { key: "status", header: "Status", render: (c) => c.status },
    { key: "reason", header: "Reason", render: (c) => c.reason || "—" },
    {
      key: "age",
      header: "Last transition",
      render: (c) => ageFromTimestamp(c.lastTransitionTime, now),
    },
  ];
  return (
    <Section title="Conditions">
      <Table columns={columns} data={conditions} getRowKey={(c) => c.type} />
    </Section>
  );
}

function phaseKind(phase: string): StatusKind {
  if (phase === "Running" || phase === "Succeeded" || phase === "Active" || phase === "Bound")
    return "success";
  if (phase === "Pending") return "warning";
  if (phase === "Failed" || phase === "Unknown" || phase === "Lost") return "danger";
  return "neutral";
}

/* ------------------------------------------------------------------ */
/* Pod detail (rich srelens presentation)                              */
/* ------------------------------------------------------------------ */

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

const PERSISTENT_VOLUME_SOURCE_TYPES = new Set([
  "awsElasticBlockStore",
  "azureDisk",
  "azureFile",
  "cephfs",
  "cinder",
  "csi",
  "fc",
  "flexVolume",
  "flocker",
  "gcePersistentDisk",
  "glusterfs",
  "hostPath",
  "iscsi",
  "local",
  "nfs",
  "photonPersistentDisk",
  "portworxVolume",
  "quobyte",
  "rbd",
  "scaleIO",
  "storageos",
  "vsphereVolume",
]);

/** Describe a container's runtime state, e.g. "running, ready". */
function containerStateText(st: Record<string, unknown>): { text: string; kind: StatusKind } {
  const state = asRecord(st.state);
  const ready = st.ready === true ? ", ready" : "";
  if ("running" in state) return { text: `running${ready}`, kind: "success" };
  if ("waiting" in state) {
    const reason = str(asRecord(state.waiting).reason) || "waiting";
    return { text: `waiting - ${reason}`, kind: reason.includes("BackOff") ? "danger" : "warning" };
  }
  if ("terminated" in state) {
    const t = asRecord(state.terminated);
    const reason = str(t.reason) || "terminated";
    const code = t.exitCode != null ? ` (exit code: ${str(t.exitCode)})` : "";
    return {
      text: `terminated${ready} - ${reason}${code}`,
      kind: reason === "Completed" ? "neutral" : "danger",
    };
  }
  return { text: "—", kind: "neutral" };
}

/** The previous termination marks when Kubernetes last restarted a container. */
export function containerLastRestartTime(status: unknown): string {
  const st = asRecord(status);
  if (Number(st.restartCount ?? 0) < 1) return "";
  return str(asRecord(asRecord(st.lastState).terminated).finishedAt);
}

function timestampWithAge(iso: string, now: number): string {
  return iso ? `${ageFromTimestamp(iso, now)} ago (${absoluteTimestamp(iso)})` : "";
}

function latestRestartTime(statuses: Record<string, unknown>[]): string {
  return statuses
    .map(containerLastRestartTime)
    .filter(Boolean)
    .sort((a, b) => Date.parse(b) - Date.parse(a))[0] ?? "";
}

/** Format a port as "name: port/protocol". */
function portText(p: Record<string, unknown>): string {
  const name = str(p.name);
  const proto = str(p.protocol) || "TCP";
  return `${name ? `${name}: ` : ""}${str(p.containerPort)}/${proto}`;
}

/** Probe → chips: "tcp-socket :cluster delay=30s timeout=1s period=10s …". */
function probeChips(probe: Record<string, unknown>): string[] {
  const chips: string[] = [];
  if (probe.httpGet) {
    const h = asRecord(probe.httpGet);
    chips.push(`http-get ${str(h.scheme || "HTTP").toLowerCase()}://:${str(h.port)}${str(h.path)}`);
  } else if (probe.tcpSocket) {
    chips.push(`tcp-socket :${str(asRecord(probe.tcpSocket).port)}`);
  } else if (probe.exec) {
    chips.push(`exec [${asArray(asRecord(probe.exec).command).map(str).join(" ")}]`);
  }
  if (probe.initialDelaySeconds != null) chips.push(`delay=${str(probe.initialDelaySeconds)}s`);
  if (probe.timeoutSeconds != null) chips.push(`timeout=${str(probe.timeoutSeconds)}s`);
  if (probe.periodSeconds != null) chips.push(`period=${str(probe.periodSeconds)}s`);
  if (probe.successThreshold != null) chips.push(`#success=${str(probe.successThreshold)}`);
  if (probe.failureThreshold != null) chips.push(`#failure=${str(probe.failureThreshold)}`);
  return chips;
}

function resourceText(r: Record<string, unknown>): string {
  return `CPU: ${str(r.cpu) || "—"}, Memory: ${str(r.memory) || "—"}`;
}

/** "NAME=value" or "NAME=<secret/configMap/field>" for an env entry. */
function envText(e: unknown): string {
  const r = asRecord(e);
  const name = str(r.name);
  if (r.value != null) return `${name}=${str(r.value)}`;
  const vf = asRecord(r.valueFrom);
  const src = vf.secretKeyRef
    ? "secret"
    : vf.configMapKeyRef
      ? "configMap"
      : vf.fieldRef
        ? "field"
        : vf.resourceFieldRef
          ? "resource"
          : "ref";
  return `${name}=<${src}>`;
}

/** "mountPath (ro) ← volume" for a volumeMount entry. */
function mountText(m: unknown): string {
  const r = asRecord(m);
  const ro = r.readOnly === true ? " (ro)" : "";
  return `${str(r.mountPath)}${ro} ← ${str(r.name)}`;
}

/** A toleration as "key=value → effect" (or "key Exists → effect"). */
function tolerationText(t: unknown): string {
  const r = asRecord(t);
  const key = str(r.key) || "(any taint)";
  const operator = str(r.operator) || "Equal";
  const effect = str(r.effect) || "all effects";
  const secs = r.tolerationSeconds != null ? ` for ${str(r.tolerationSeconds)}s` : "";
  const left = operator === "Exists" ? `${key} exists` : `${key}=${str(r.value)}`;
  return `${left} → ${effect}${secs}`;
}

function PlainChips({ items }: { items: string[] }) {
  return (
    <div className="fl-chips">
      {items.map((t, i) => (
        <span className="fl-chip fl-chip--plain" key={`${t}-${i}`}>
          {t}
        </span>
      ))}
    </div>
  );
}

/** One container (or init container) block. */
/** Target a port-forward can attach to (a Pod or Service in a context). */
export interface ForwardTarget {
  context: string;
  namespace: string;
  kind: "Pod" | "Service";
  name: string;
}

/**
 * Inline "forward" affordance for a single port: a compact icon button that
 * opens the forward dialog pre-filled with that port. Renders nothing without
 * a forward target (e.g. in tests, or when no context is available).
 */
function PortForwardButton({ target, port }: { target?: ForwardTarget; port?: number }) {
  const [open, setOpen] = useState(false);
  if (!target || !port) return null;
  return (
    <span className="fl-port-forward">
      <IconButton icon={ArrowLeftRight} label={`Forward port ${port}`} onClick={() => setOpen(true)} />
      {open && (
        <ForwardDialog
          context={target.context}
          namespace={target.namespace}
          kind={target.kind}
          name={target.name}
          defaultRemotePort={port}
          onClose={() => setOpen(false)}
        />
      )}
    </span>
  );
}

/** A port row with an inline forward button (used in Pod/Service detail). */
function ForwardablePorts({
  ports,
  target,
  portOf,
}: {
  ports: Record<string, unknown>[];
  target?: ForwardTarget;
  portOf: (p: Record<string, unknown>) => number | undefined;
}) {
  return (
    <div className="fl-chips">
      {ports.map((p, i) => (
        <span key={i} className="fl-port-chip">
          <span className="fl-chip fl-chip--plain">{portText(p)}</span>
          <PortForwardButton target={target} port={portOf(p)} />
        </span>
      ))}
    </div>
  );
}

function ContainerCard({
  container,
  status,
  forward,
  now,
  onLogs,
  onExec,
}: {
  container: Record<string, unknown>;
  status?: Record<string, unknown>;
  forward?: ForwardTarget;
  now: number;
  /** Open logs scoped to this container. */
  onLogs?: () => void;
  /** Open an exec session in this container. */
  onExec?: () => void;
}) {
  const name = str(container.name);
  const st = status ? containerStateText(status) : null;
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
  const restartCount = status?.restartCount;
  const lastRestart = containerLastRestartTime(status);
  const runningSince = str(asRecord(asRecord(status?.state).running).startedAt);

  return (
    <div className="fl-container-card">
      <div className="fl-container-card__name">
        <span className={`fl-status__dot fl-status--${st?.kind ?? "neutral"}`} />
        {name}
        {(onLogs || onExec) && (
          <span className="ml-auto flex gap-0.5">
            {onLogs && <IconButton icon={ScrollText} label={`Logs for ${name}`} onClick={onLogs} />}
            {onExec && <IconButton icon={SquareTerminal} label={`Exec into ${name}`} onClick={onExec} />}
          </span>
        )}
      </div>
      <KV
        pairs={[
          ["Status", st ? <span className={`fl-status--${st.kind} fl-status-text`}>{st.text}</span> : ""],
          container.targetContainerName
            ? ["Debugging", <span className="fl-mono">{str(container.targetContainerName)}</span>]
            : ["", ""],
          ["Restarts", restartCount != null ? str(restartCount) : ""],
          ["Last restart", timestampWithAge(lastRestart, now)],
          ["Running since", timestampWithAge(runningSince, now)],
          ["Image", <CollapsibleText text={str(container.image)} label="image" lines={2} />],
          [
            "Ports",
            ports.length ? (
              <ForwardablePorts
                ports={ports}
                target={forward}
                portOf={(p) => Number(p.containerPort) || undefined}
              />
            ) : (
              ""
            ),
          ],
          [
            "Environment",
            env.length ? (
              <Expandable summary={plural(env.length, "environment variable")}>
                <PlainChips items={env.map(envText)} />
              </Expandable>
            ) : (
              ""
            ),
          ],
          [
            "Mounts",
            mounts.length ? (
              <Expandable summary={plural(mounts.length, "mount")}>
                <PlainChips items={mounts.map(mountText)} />
              </Expandable>
            ) : (
              ""
            ),
          ],
          ["Liveness", Object.keys(liveness).length ? <PlainChips items={probeChips(liveness)} /> : ""],
          ["Readiness", Object.keys(readiness).length ? <PlainChips items={probeChips(readiness)} /> : ""],
          ["Startup", Object.keys(startup).length ? <PlainChips items={probeChips(startup)} /> : ""],
          ["Command", command ? <CollapsibleText text={command} label="command" muted /> : ""],
          ["Requests", Object.keys(requests).length ? resourceText(requests) : ""],
          ["Limits", Object.keys(limits).length ? resourceText(limits) : ""],
        ]}
      />
    </div>
  );
}

/**
 * Pod lifecycle conditions as an ordered timeline. Three aligned columns —
 * type (with reason), status pill, and the transition time (relative, with the
 * absolute timestamp on hover) right-aligned so the progression scans top-down.
 */
function PodConditionsTimeline({ conditions, now }: { conditions: Condition[]; now: number }) {
  if (conditions.length === 0) return null;
  return (
    <Section title="Conditions">
      <ol className="grid grid-cols-[auto_auto_1fr] items-center gap-x-4 gap-y-2">
        {orderPodConditions(conditions).map((condition) => (
          <li key={condition.type} className="contents">
            <span className="fl-mono text-sm">
              {condition.type}
              {condition.reason && condition.reason !== condition.type && (
                <span className="text-muted-foreground"> · {condition.reason}</span>
              )}
            </span>
            <StatusPill status={condition.status} kind={conditionKind(condition)} />
            <span
              className="text-right text-xs text-muted-foreground tabular-nums"
              title={condition.lastTransitionTime ? absoluteTimestamp(condition.lastTransitionTime) : undefined}
            >
              {condition.lastTransitionTime ? `${ageFromTimestamp(condition.lastTransitionTime, now)} ago` : ""}
            </span>
          </li>
        ))}
      </ol>
    </Section>
  );
}

function PodDetailView({
  obj,
  now,
  context = "",
  onOpenResource,
  onOpenLogs,
  onOpenExec,
}: {
  obj: K8sObject;
  now: number;
  context?: string;
  onOpenResource?: OpenResource;
  /** Open logs for a specific container of this pod. */
  onOpenLogs?: (container: string) => void;
  /** Open an exec/terminal session in a specific container of this pod. */
  onOpenExec?: (container: string) => void;
}) {
  const meta = asRecord(obj.metadata);
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const labels = (meta.labels ?? {}) as Record<string, string>;
  const annotations = (meta.annotations ?? {}) as Record<string, string>;
  const owners = asArray(meta.ownerReferences).map(asRecord);
  const conditions = asArray(status.conditions) as unknown as Condition[];
  const podIPs = asArray(status.podIPs).map((p) => str(asRecord(p).ip)).filter(Boolean);
  const tolerations = asArray(spec.tolerations);
  const nodeSelector = (spec.nodeSelector ?? {}) as Record<string, string>;
  const affinityLines = summarizeAffinity(asRecord(spec.affinity));
  const hasScheduling =
    !!spec.nodeName ||
    Object.keys(nodeSelector).length > 0 ||
    affinityLines.length > 0 ||
    tolerations.length > 0;
  const created = str(meta.creationTimestamp);
  const namespace = str(meta.namespace) || null;
  const forward: ForwardTarget | undefined = context
    ? { context, namespace: str(meta.namespace), kind: "Pod", name: str(meta.name) }
    : undefined;

  const podVolumes = asArray(spec.volumes).map(asRecord);
  const ownerTargets = owners
    .map((owner) => ({
      kind: str(owner.kind),
      name: str(owner.name),
      namespace: targetNamespace(str(owner.kind), namespace),
    }))
    .filter((target) => target.kind && target.name);
  const imagePullSecrets = asArray(spec.imagePullSecrets)
    .map((secret) => str(asRecord(secret).name))
    .filter(Boolean);

  const containerStatuses = new Map(
    asArray(status.containerStatuses).map((s) => [str(asRecord(s).name), asRecord(s)]),
  );
  const initStatuses = new Map(
    asArray(status.initContainerStatuses).map((s) => [str(asRecord(s).name), asRecord(s)]),
  );
  const ephemeralStatuses = new Map(
    asArray(status.ephemeralContainerStatuses).map((s) => [str(asRecord(s).name), asRecord(s)]),
  );
  const ephemeralContainers = asArray(spec.ephemeralContainers).map(asRecord);
  const allContainerStatuses = [
    ...asArray(status.initContainerStatuses),
    ...asArray(status.containerStatuses),
    ...asArray(status.ephemeralContainerStatuses),
  ].map(asRecord);
  const podRestartCount = allContainerStatuses.reduce(
    (total, containerStatus) => total + Number(containerStatus.restartCount ?? 0),
    0,
  );
  const podLastRestart = latestRestartTime(allContainerStatuses);

  const phase = str(status.phase);
  const volumeSource = (volume: Record<string, unknown>): React.ReactNode => {
    const pvc = asRecord(volume.persistentVolumeClaim);
    const configMap = asRecord(volume.configMap);
    const secret = asRecord(volume.secret);
    if (pvc.claimName)
      return (
        <ResourceLink
          target={{ kind: "PersistentVolumeClaim", namespace, name: str(pvc.claimName) }}
          onOpenResource={onOpenResource}
        />
      );
    if (configMap.name)
      return (
        <ResourceLink
          target={{ kind: "ConfigMap", namespace, name: str(configMap.name) }}
          onOpenResource={onOpenResource}
        />
      );
    if (secret.secretName)
      return (
        <ResourceLink
          target={{ kind: "Secret", namespace, name: str(secret.secretName) }}
          onOpenResource={onOpenResource}
        />
      );
    if (volume.hostPath) return <span className="fl-mono">{str(asRecord(volume.hostPath).path)}</span>;
    if (volume.nfs) {
      const nfs = asRecord(volume.nfs);
      return <span className="fl-mono">{str(nfs.server)}:{str(nfs.path)}</span>;
    }
    if (volume.csi) return <span className="fl-mono">{str(asRecord(volume.csi).driver)}</span>;
    if (volume.projected) {
      const sources = asArray(asRecord(volume.projected).sources).map(asRecord);
      const targets = sources.flatMap((source): ResourceTarget[] => {
        const projectedConfigMap = asRecord(source.configMap);
        const projectedSecret = asRecord(source.secret);
        if (projectedConfigMap.name)
          return [{ kind: "ConfigMap", namespace, name: str(projectedConfigMap.name) }];
        if (projectedSecret.name)
          return [{ kind: "Secret", namespace, name: str(projectedSecret.name) }];
        return [];
      });
      return targets.length ? (
        <LinkedResources targets={targets} onOpenResource={onOpenResource} />
      ) : (
        `${sources.length} projected sources`
      );
    }
    if (volume.emptyDir) return str(asRecord(volume.emptyDir).medium) || "Node temporary storage";
    return "—";
  };
  const volumeColumns: Column<Record<string, unknown>>[] = [
    { key: "name", header: "Name", render: (volume) => <span className="fl-mono">{str(volume.name)}</span> },
    {
      key: "type",
      header: "Type",
      render: (volume) => {
        const type = Object.keys(volume).find((key) => key !== "name") ?? "unknown";
        return VOLUME_TYPE_LABELS[type] ?? type;
      },
    },
    { key: "source", header: "Source", render: volumeSource },
  ];

  return (
    <div className="fl-detail">
      <Section title="Properties">
        <KV
          pairs={[
            ["Created", created ? `${ageFromTimestamp(created, now)} ago (${absoluteTimestamp(created)})` : ""],
            ["Name", <span className="fl-mono">{str(meta.name)}</span>],
            [
              "Namespace",
              meta.namespace ? (
                <ResourceLink
                  target={{ kind: "Namespace", namespace: null, name: str(meta.namespace) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            ["Labels", <ChipMap map={labels} singular="Label" />],
            ["Annotations", <ChipMap map={annotations} singular="Annotation" />],
            [
              "Controlled By",
              ownerTargets.length ? (
                <LinkedResources targets={ownerTargets} onOpenResource={onOpenResource} />
              ) : (
                ""
              ),
            ],
            ["Status", <StatusPill key="s" status={phase || "—"} kind={phaseKind(phase)} />],
            ["Container restarts", str(podRestartCount)],
            ["Last restart", timestampWithAge(podLastRestart, now)],
            [
              "Node",
              spec.nodeName ? (
                <ResourceLink
                  target={{ kind: "Node", namespace: null, name: str(spec.nodeName) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            ["Pod IP", <span className="fl-mono">{str(status.podIP)}</span>],
            ["Pod IPs", podIPs.length ? <PlainChips items={podIPs} /> : ""],
            [
              "Service Account",
              spec.serviceAccountName ? (
                <ResourceLink
                  target={{ kind: "ServiceAccount", namespace, name: str(spec.serviceAccountName) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            [
              "Priority Class",
              spec.priorityClassName ? (
                <ResourceLink
                  target={{ kind: "PriorityClass", namespace: null, name: str(spec.priorityClassName) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            [
              "Runtime Class",
              spec.runtimeClassName ? (
                <ResourceLink
                  target={{ kind: "RuntimeClass", namespace: null, name: str(spec.runtimeClassName) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            [
              "Image pull secrets",
              imagePullSecrets.length ? (
                <LinkedResources
                  targets={imagePullSecrets.map((name) => ({ kind: "Secret", namespace, name }))}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            ["QoS Class", str(status.qosClass)],
          ]}
        />
      </Section>

      <PodConditionsTimeline conditions={conditions} now={now} />

      {hasScheduling && (
        <Section title="Scheduling">
          <KV
            pairs={[
              [
                "Node",
                spec.nodeName ? (
                  <ResourceLink
                    target={{ kind: "Node", namespace: null, name: str(spec.nodeName) }}
                    onOpenResource={onOpenResource}
                  />
                ) : (
                  "Not scheduled"
                ),
              ],
              [
                "Node selector",
                Object.keys(nodeSelector).length ? <Chips map={nodeSelector} /> : "",
              ],
              ["Affinity", affinityLines.length ? <PlainChips items={affinityLines} /> : ""],
              [
                "Tolerations",
                tolerations.length ? (
                  <Expandable summary={plural(tolerations.length, "toleration")}>
                    <PlainChips items={tolerations.map(tolerationText)} />
                  </Expandable>
                ) : (
                  ""
                ),
              ],
            ]}
          />
        </Section>
      )}

      {podVolumes.length > 0 && (
        <Section title="Pod Volumes">
          <Table
            columns={volumeColumns}
            data={podVolumes}
            getRowKey={(volume) => str(volume.name)}
          />
        </Section>
      )}

      {asArray(spec.initContainers).length > 0 && (
        <Section title="Init Containers">
          {asArray(spec.initContainers).map((c) => {
            const cr = asRecord(c);
            return (
              <ContainerCard
                key={str(cr.name)}
                container={cr}
                status={initStatuses.get(str(cr.name))}
                now={now}
              />
            );
          })}
        </Section>
      )}

      <Section title="Containers">
        {asArray(spec.containers).length === 0 ? (
          <span className="fl-detail-empty">No containers</span>
        ) : (
          asArray(spec.containers).map((c) => {
            const cr = asRecord(c);
            const cn = str(cr.name);
            return (
              <ContainerCard
                key={cn}
                container={cr}
                status={containerStatuses.get(cn)}
                forward={forward}
                now={now}
                onLogs={onOpenLogs ? () => onOpenLogs(cn) : undefined}
                onExec={onOpenExec ? () => onOpenExec(cn) : undefined}
              />
            );
          })
        )}
      </Section>

      {ephemeralContainers.length > 0 && (
        <Section title="Ephemeral Containers">
          {ephemeralContainers.map((cr) => {
            const cn = str(cr.name);
            return (
              <ContainerCard
                key={cn}
                container={cr}
                status={ephemeralStatuses.get(cn)}
                now={now}
                onLogs={onOpenLogs ? () => onOpenLogs(cn) : undefined}
              />
            );
          })}
        </Section>
      )}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* other kinds                                                         */
/* ------------------------------------------------------------------ */

/** A single-section "Properties" view for Deployments/StatefulSets/ReplicaSets. */
function WorkloadDetailView({
  kind,
  obj,
  now,
  context,
  onOpenResource,
}: {
  kind: string;
  obj: K8sObject;
  now: number;
  context: string;
  onOpenResource?: OpenResource;
}) {
  const meta = asRecord(obj.metadata);
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const labels = (meta.labels ?? {}) as Record<string, string>;
  const annotations = (meta.annotations ?? {}) as Record<string, string>;
  const selector = asRecord(asRecord(spec.selector).matchLabels) as Record<string, string>;
  const name = str(meta.name);
  const namespace = str(meta.namespace);
  const conditions = asArray(status.conditions) as unknown as Condition[];
  const owners = asArray(meta.ownerReferences).map(asRecord);
  const ownerTargets = owners
    .map((owner) => ({
      kind: str(owner.kind),
      name: str(owner.name),
      namespace: targetNamespace(str(owner.kind), namespace),
    }))
    .filter((target) => target.kind && target.name);
  const created = str(meta.creationTimestamp);

  const num = (v: unknown) => (v != null ? Number(v) : 0);
  const desired = spec.replicas != null ? num(spec.replicas) : 0;
  const total = num(status.replicas);
  const updated = num(status.updatedReplicas);
  const available = num(status.availableReplicas);
  const unavailable = num(status.unavailableReplicas);
  const replicaText = `${desired} desired, ${updated} updated, ${total} total, ${available} available, ${unavailable} unavailable`;

  // srelens shows "Running" once the workload is fully available.
  const running = desired > 0 && available >= desired;
  const phase = running ? "Running" : "Pending";

  return (
    <div className="fl-detail">
      <Section title="Properties">
        <KV
          pairs={[
            ["Created", created ? `${ageFromTimestamp(created, now)} ago (${absoluteTimestamp(created)})` : ""],
            ["Name", <span className="fl-mono">{str(meta.name)}</span>],
            [
              "Namespace",
              meta.namespace ? (
                <ResourceLink
                  target={{ kind: "Namespace", namespace: null, name: str(meta.namespace) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            ["Labels", <ChipMap map={labels} singular="Label" />],
            ["Annotations", <ChipMap map={annotations} singular="Annotation" />],
            ["Replicas", replicaText],
            ["Selector", <Chips key="s" map={selector} />],
            [
              "Managed By",
              ownerTargets.length ? (
                <LinkedResources targets={ownerTargets} onOpenResource={onOpenResource} />
              ) : (
                ""
              ),
            ],
            [
              "Strategy Type",
              kind === "Deployment"
                ? str(asRecord(spec.strategy).type)
                : updateStrategyText(asRecord(spec.updateStrategy)),
            ],
            ...(kind === "StatefulSet"
              ? ([
                  ["Service", <span key="svc" className="fl-mono">{str(spec.serviceName)}</span>],
                  [
                    "Volume claim templates",
                    asArray(spec.volumeClaimTemplates)
                      .map((t) => str(asRecord(asRecord(t).metadata).name))
                      .filter(Boolean)
                      .join(", ") || "—",
                  ],
                ] as Pair[])
              : []),
            ["Status", <StatusPill key="st" status={phase} kind={phaseKind(phase)} />],
            ["Conditions", <ConditionBadges key="c" conditions={conditions} />],
          ]}
        />
      </Section>

      {kind === "Deployment" && (
        <DeployRevisions
          context={context}
          namespace={namespace}
          ownerName={name}
          onOpenResource={onOpenResource}
        />
      )}
      {Object.keys(selector).length > 0 && (
        <ManagedPods
          context={context}
          namespace={namespace}
          selector={selector}
          onOpenResource={onOpenResource}
        />
      )}
    </div>
  );
}

/** "RollingUpdate (partition 2)" / "RollingUpdate (max unavailable 1)" / "OnDelete". */
function updateStrategyText(strategy: Record<string, unknown>): string {
  const type = str(strategy.type) || "RollingUpdate";
  const ru = asRecord(strategy.rollingUpdate);
  const parts: string[] = [];
  if (ru.partition != null) parts.push(`partition ${str(ru.partition)}`);
  if (ru.maxUnavailable != null) parts.push(`max unavailable ${str(ru.maxUnavailable)}`);
  if (ru.maxSurge != null) parts.push(`max surge ${str(ru.maxSurge)}`);
  return parts.length ? `${type} (${parts.join(", ")})` : type;
}

function DaemonSetBody({ obj }: { obj: K8sObject }) {
  const status = asRecord(obj.status);
  const spec = asRecord(obj.spec);
  const selector = asRecord(asRecord(spec.selector).matchLabels) as Record<string, string>;
  return (
    <Section title="Scheduling">
      <KV
        pairs={[
          ["Desired", str(status.desiredNumberScheduled)],
          ["Current", str(status.currentNumberScheduled)],
          ["Ready", str(status.numberReady)],
          ["Up-to-date", str(status.updatedNumberScheduled)],
          ["Available", str(status.numberAvailable)],
          ["Update strategy", updateStrategyText(asRecord(spec.updateStrategy))],
          ["Selector", <Chips key="s" map={selector} />],
        ]}
      />
    </Section>
  );
}

interface PortRow {
  key: string;
  name: string;
  port: string;
  target: string;
  protocol: string;
  /** The service port number, for the inline forward button. */
  servicePort?: number;
}

function ServiceBody({
  obj,
  context = "",
  onOpenResource,
}: {
  obj: K8sObject;
  context?: string;
  onOpenResource?: OpenResource;
}) {
  const spec = asRecord(obj.spec);
  const meta = asRecord(obj.metadata);
  const namespace = str(meta.namespace) || null;
  const name = str(meta.name);
  const selector = asRecord(spec.selector) as Record<string, string>;

  // The service's EndpointSlices carry a `kubernetes.io/service-name` label the
  // backend surfaces as `service`; list them in the namespace and keep ours.
  // This closes the service → endpointslice → pods navigation chain.
  const [sliceTargets, setSliceTargets] = useState<ResourceTarget[]>([]);
  useEffect(() => {
    setSliceTargets([]);
    if (!context || !namespace || !name) return;
    let active = true;
    void listEndpointSlices(context, namespace).then((r) => {
      if (!active) return;
      const mine = (r.endpointslices ?? [])
        .filter((s) => s.service === name)
        .map((s): ResourceTarget => ({ kind: "EndpointSlice", namespace, name: s.name }));
      setSliceTargets(mine);
    });
    return () => {
      active = false;
    };
  }, [context, namespace, name]);
  const ports: PortRow[] = asArray(spec.ports).map((p, i) => {
    const pr = asRecord(p);
    return {
      key: str(pr.name) || `port-${i}`,
      name: str(pr.name) || "—",
      port: str(pr.port) + (pr.nodePort ? `:${str(pr.nodePort)}` : ""),
      target: str(pr.targetPort),
      protocol: str(pr.protocol) || "TCP",
      servicePort: Number(pr.port) || undefined,
    };
  });
  // Headless/ExternalName services can't be port-forwarded (no backing pod to
  // attach to), so only offer it when there's a selector.
  const forward: ForwardTarget | undefined =
    context && Object.keys(selector).length > 0
      ? { context, namespace: str(meta.namespace), kind: "Service", name: str(meta.name) }
      : undefined;
  const portCols: Column<PortRow>[] = [
    { key: "name", header: "Name", render: (p) => p.name },
    { key: "port", header: "Port", render: (p) => <span className="fl-mono">{p.port}</span> },
    { key: "target", header: "Target", render: (p) => <span className="fl-mono">{p.target}</span> },
    { key: "protocol", header: "Protocol", render: (p) => p.protocol },
    ...(forward
      ? [
          {
            key: "forward",
            header: "",
            render: (p: PortRow) => <PortForwardButton target={forward} port={p.servicePort} />,
          } as Column<PortRow>,
        ]
      : []),
  ];
  return (
    <>
      <Section title="Connection">
        <KV
          pairs={[
            ["Type", str(spec.type) || "ClusterIP"],
            ["Cluster IP", <span className="fl-mono">{str(spec.clusterIP)}</span>],
            ["Session affinity", str(spec.sessionAffinity)],
            ["Selector", <Chips key="s" map={selector} />],
          ]}
        />
      </Section>
      {ports.length > 0 && (
        <Section title="Ports">
          <Table columns={portCols} data={ports} getRowKey={(p) => p.key} />
        </Section>
      )}
      {sliceTargets.length > 0 && (
        <Section title="Endpoint Slices">
          <LinkedResources targets={sliceTargets} onOpenResource={onOpenResource} />
        </Section>
      )}
    </>
  );
}

function NodeBody({ obj }: { obj: K8sObject }) {
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const info = asRecord(status.nodeInfo);
  const cap = asRecord(status.capacity);
  const alloc = asRecord(status.allocatable);
  const cordoned = spec.unschedulable === true;
  return (
    <>
      <Section title="Info">
        <KV
          pairs={[
            [
              "Scheduling",
              <StatusPill
                key="s"
                status={cordoned ? "Disabled (cordoned)" : "Enabled"}
                kind={cordoned ? "warning" : "success"}
              />,
            ],
            ["Kubelet", str(info.kubeletVersion)],
            ["OS image", str(info.osImage)],
            ["Kernel", str(info.kernelVersion)],
            ["Container runtime", str(info.containerRuntimeVersion)],
            ["Architecture", str(info.architecture)],
          ]}
        />
      </Section>
      <Section title="Capacity">
        <KV
          pairs={[
            ["CPU", `${str(alloc.cpu)} / ${str(cap.cpu)}`],
            ["Memory", `${str(alloc.memory)} / ${str(cap.memory)}`],
            ["Pods", `${str(alloc.pods)} / ${str(cap.pods)}`],
          ]}
        />
      </Section>
    </>
  );
}

function JobBody({ obj, now }: { obj: K8sObject; now: number }) {
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const startTime = str(status.startTime);
  const completionTime = str(status.completionTime);
  const duration = completionTime
    ? durationBetween(startTime, completionTime)
    : startTime
      ? `${ageFromTimestamp(startTime, now)} (running)`
      : "—";
  return (
    <Section title="Job">
      <KV
        pairs={[
          ["Completions", str(spec.completions)],
          ["Parallelism", str(spec.parallelism)],
          ["Succeeded", str(status.succeeded) || "0"],
          ["Failed", str(status.failed) || "0"],
          ["Active", str(status.active) || "0"],
          ["Started", startTime ? timestampWithAge(startTime, now) : "—"],
          ["Completed", completionTime ? timestampWithAge(completionTime, now) : "—"],
          ["Duration", duration],
        ]}
      />
    </Section>
  );
}

function CronJobBody({
  obj,
  now,
  context = "",
  onOpenResource,
}: {
  obj: K8sObject;
  now: number;
  context?: string;
  onOpenResource?: OpenResource;
}) {
  const meta = asRecord(obj.metadata);
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const namespace = str(meta.namespace) || null;
  const name = str(meta.name);
  const lastSchedule = str(status.lastScheduleTime);
  const successKept = str(spec.successfulJobsHistoryLimit) || "3";
  const failedKept = str(spec.failedJobsHistoryLimit) || "1";
  const activeJobs = asArray(status.active)
    .map(asRecord)
    .map((job) => ({
      kind: str(job.kind) || "Job",
      namespace: str(job.namespace) || namespace,
      name: str(job.name),
    }))
    .filter((job) => job.name);
  return (
    <>
      <Section title="Schedule">
        <KV
          pairs={[
            ["Schedule", <span className="fl-mono">{str(spec.schedule)}</span>],
            ["Suspend", spec.suspend === true ? "Yes" : "No"],
            ["Concurrency policy", str(spec.concurrencyPolicy)],
            ["Last schedule", lastSchedule ? timestampWithAge(lastSchedule, now) : "—"],
            ["History (kept)", `${successKept} succeeded, ${failedKept} failed`],
            [
              "Active jobs",
              activeJobs.length ? (
                <LinkedResources targets={activeJobs} onOpenResource={onOpenResource} />
              ) : (
                "0"
              ),
            ],
          ]}
        />
      </Section>
      {context && namespace && (
        <CronJobJobs
          context={context}
          namespace={namespace}
          ownerName={name}
          onOpenResource={onOpenResource}
        />
      )}
    </>
  );
}

function decodeBase64(v: string): string {
  try {
    const binary = atob(v);
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return v;
  }
}

function decodedByteLength(v: string): number {
  try {
    return atob(v).length;
  } catch {
    return new TextEncoder().encode(v).length;
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/** One ConfigMap/Secret entry: key + value. Secret values are base64-decoded
 *  and masked behind a reveal toggle. */
function ConfigDataEntry({ name, value, secret }: { name: string; value: string; secret: boolean }) {
  const [revealed, setRevealed] = useState(false);
  const display = secret ? (revealed ? decodeBase64(value) : "••••••••") : value;
  return (
    <div className="fl-secret-entry">
      <div className="fl-secret-entry__header">
        <span className="fl-mono">{name}</span>
        {secret && (
          <button
            type="button"
            onClick={() => setRevealed((r) => !r)}
            className="fl-secret-entry__toggle"
          >
            {revealed ? "Hide" : "Reveal"}
          </button>
        )}
      </div>
      <pre className="fl-secret-entry__value">{display}</pre>
    </div>
  );
}

function SecretData({ data }: { data: Record<string, string> }) {
  const keys = Object.keys(data);
  return (
    <Section title={`Data (${plural(keys.length, "key")})`}>
      {keys.length === 0 ? (
        <span className="fl-detail-empty">No data</span>
      ) : (
        <div className="flex flex-col gap-3">
          {keys.map((key) => (
            <ConfigDataEntry key={key} name={key} value={str(data[key])} secret />
          ))}
        </div>
      )}
    </Section>
  );
}

interface CertificateRow {
  key: string;
  role: string;
  subject: string;
  issuer: string;
  serial: string;
  validFrom: string;
  validUntil: string;
  status: string;
  keyAlgorithm: string;
  sans: string[];
  size: string;
}

function publicKeyAlgorithm(certificate: X509Certificate): string {
  const algorithm = certificate.publicKey.algorithm as Algorithm & {
    modulusLength?: number;
    namedCurve?: string;
  };
  if (algorithm.namedCurve) return `${algorithm.name} ${algorithm.namedCurve}`;
  if (algorithm.modulusLength) return `${algorithm.name} ${algorithm.modulusLength}-bit`;
  return algorithm.name;
}

async function certificateRows(pem: string): Promise<CertificateRow[]> {
  await import("reflect-metadata");
  const { SubjectAlternativeNameExtension, X509Certificate } = await import("@peculiar/x509");
  const matches = pem.match(/-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g) ?? [];
  return matches.map((pemCertificate, index) => {
    const fallback: CertificateRow = {
      key: String(index),
      role: index === 0 ? "Leaf" : `Chain ${index}`,
      subject: "Unable to parse certificate",
      issuer: "",
      serial: "",
      validFrom: "",
      validUntil: "",
      status: "Invalid",
      keyAlgorithm: "",
      sans: [],
      size: formatBytes(new TextEncoder().encode(pemCertificate).length),
    };
    try {
      const certificate = new X509Certificate(pemCertificate);
      const now = Date.now();
      const expires = certificate.notAfter.getTime();
      const starts = certificate.notBefore.getTime();
      const status = now < starts
        ? "Not yet valid"
        : now > expires
          ? "Expired"
          : expires - now < 30 * 86_400_000
            ? "Expires soon"
            : "Valid";
      const san = certificate.getExtension(SubjectAlternativeNameExtension);
      return {
        ...fallback,
        subject: certificate.subject,
        issuer: certificate.issuer,
        serial: certificate.serialNumber,
        validFrom: certificate.notBefore.toISOString(),
        validUntil: certificate.notAfter.toISOString(),
        status,
        keyAlgorithm: publicKeyAlgorithm(certificate),
        sans: san?.names.items.map((name) => name.value) ?? [],
      };
    } catch {
      return fallback;
    }
  });
}

function privateKeyType(pem: string): string {
  if (/BEGIN RSA PRIVATE KEY/.test(pem)) return "RSA (PKCS#1)";
  if (/BEGIN EC PRIVATE KEY/.test(pem)) return "EC (SEC1)";
  if (/BEGIN ENCRYPTED PRIVATE KEY/.test(pem)) return "Encrypted PKCS#8";
  if (/BEGIN PRIVATE KEY/.test(pem)) return "PKCS#8";
  return pem ? "Unrecognized format" : "Missing";
}

function TlsSecretBody({ data }: { data: Record<string, string> }) {
  const certificate = decodeBase64(str(data["tls.crt"]));
  const privateKey = decodeBase64(str(data["tls.key"]));
  const certificateCount = certificate.match(/-----BEGIN CERTIFICATE-----/g)?.length ?? 0;
  const [certificates, setCertificates] = useState<CertificateRow[] | null>(null);
  useEffect(() => {
    let active = true;
    if (certificateCount === 0) {
      setCertificates([]);
    } else {
      void certificateRows(certificate)
        .then((rows) => {
          if (active) setCertificates(rows);
        })
        .catch(() => {
          if (active) setCertificates([]);
        });
    }
    return () => {
      active = false;
    };
  }, [certificate, certificateCount]);
  const leaf = certificates?.[0];
  const columns: Column<CertificateRow>[] = [
    { key: "role", header: "Certificate", render: (row) => row.role },
    { key: "subject", header: "Subject", render: (row) => <span className="fl-mono">{row.subject}</span> },
    {
      key: "status",
      header: "Status",
      render: (row) => (
        <StatusPill
          status={row.status}
          kind={row.status === "Valid" ? "success" : row.status === "Expires soon" ? "warning" : "danger"}
        />
      ),
    },
    { key: "size", header: "Size", render: (row) => row.size },
  ];
  return (
    <>
      <Section title="TLS material">
        <KV
          pairs={[
            ["Type", "kubernetes.io/tls"],
            ["Certificates", certificateCount ? plural(certificateCount, "certificate") : "Missing tls.crt"],
            ["Private key", privateKeyType(privateKey)],
            [
              "Certificate status",
              leaf ? (
                <StatusPill
                  status={leaf.status}
                  kind={leaf.status === "Valid" ? "success" : leaf.status === "Expires soon" ? "warning" : "danger"}
                />
              ) : (
                ""
              ),
            ],
            ["Subject", leaf?.subject],
            ["Issuer", leaf?.issuer],
            ["Serial number", leaf?.serial ? <span className="fl-mono">{leaf.serial}</span> : ""],
            ["Public key", leaf?.keyAlgorithm],
            ["Valid from", leaf?.validFrom ? absoluteTimestamp(leaf.validFrom) : ""],
            ["Valid until", leaf?.validUntil ? absoluteTimestamp(leaf.validUntil) : ""],
            ["DNS / IP names", leaf?.sans.length ? <PlainChips items={leaf.sans} /> : ""],
            ["Certificate data", data["tls.crt"] ? formatBytes(decodedByteLength(data["tls.crt"])) : ""],
            ["Private key data", data["tls.key"] ? formatBytes(decodedByteLength(data["tls.key"])) : ""],
          ]}
        />
        {certificates === null && certificateCount > 0 && <Spinner label="Reading certificates" />}
        {certificates && certificates.length > 0 && (
          <Table columns={columns} data={certificates} getRowKey={(row) => row.key} />
        )}
      </Section>
      <SecretData data={data} />
    </>
  );
}

interface DockerRegistryRow {
  registry: string;
  username: string;
  credential: string;
}

function dockerRegistries(data: Record<string, string>, type: string): DockerRegistryRow[] {
  const key = type === "kubernetes.io/dockercfg" ? ".dockercfg" : ".dockerconfigjson";
  try {
    const parsed = JSON.parse(decodeBase64(str(data[key]))) as Record<string, unknown>;
    const auths = type === "kubernetes.io/dockercfg" ? parsed : asRecord(parsed.auths);
    return Object.entries(auths).map(([registry, raw]) => {
      const auth = asRecord(raw);
      const decodedAuth = auth.auth ? decodeBase64(str(auth.auth)) : "";
      const username = str(auth.username) || decodedAuth.split(":", 1)[0];
      return {
        registry,
        username: username || "—",
        credential: auth.identitytoken ? "Identity token" : auth.auth || auth.password ? "Stored" : "Missing",
      };
    });
  } catch {
    return [];
  }
}

function DockerSecretBody({ data, type }: { data: Record<string, string>; type: string }) {
  const configKey = type === "kubernetes.io/dockercfg" ? ".dockercfg" : ".dockerconfigjson";
  const registries = dockerRegistries(data, type);
  const columns: Column<DockerRegistryRow>[] = [
    { key: "registry", header: "Registry", render: (row) => <span className="fl-mono">{row.registry}</span> },
    { key: "username", header: "Username", render: (row) => row.username },
    { key: "credential", header: "Credential", render: (row) => row.credential },
  ];
  return (
    <>
      <Section title="Docker registries">
        <KV
          pairs={[
            ["Type", type],
            ["Registries", plural(registries.length, "registry", "registries")],
            ["Config size", data[configKey] ? formatBytes(decodedByteLength(data[configKey])) : ""],
          ]}
        />
        {registries.length ? (
          <Table columns={columns} data={registries} getRowKey={(row) => row.registry} />
        ) : (
          <span className="fl-detail-empty">No valid registry credentials found</span>
        )}
      </Section>
      <SecretData data={data} />
    </>
  );
}

/**
 * Fetch a Secret's real (base64) values via the gated `getSecret`. `getObject`
 * redacts Secret data, so this is how the detail views get the actual values.
 * Falls back to the redacted keys (blank values) while loading or when there's
 * no context (a static preview). Values sit in memory but stay masked in the
 * DOM until the user reveals a key.
 */
function useSecretData(
  context: string,
  namespace: string,
  name: string,
  redacted: Record<string, string>,
): Record<string, string> {
  const [data, setData] = useState<Record<string, string> | null>(null);
  useEffect(() => {
    setData(null);
    if (!context) return;
    let active = true;
    void getSecret(context, namespace, name).then((r) => {
      if (active && r.data) setData(r.data);
    });
    return () => {
      active = false;
    };
  }, [context, namespace, name]);
  return data ?? redacted;
}

function GeneralSecretBody({
  obj,
  data,
  context = "",
  onEdited,
}: {
  obj: K8sObject;
  data: Record<string, string>;
  context?: string;
  onEdited?: () => void;
}) {
  const meta = asRecord(obj.metadata);
  const keys = Object.keys(data);
  const immutable = obj.immutable === true;
  return (
    <>
      <Section title="Secret summary">
        <KV
          pairs={[
            ["Type", str(obj.type) || "Opaque"],
            ["Keys", plural(keys.length, "key")],
            ["Immutable", immutable ? "Yes" : "No"],
          ]}
        />
      </Section>
      <Section title={`Data (${plural(keys.length, "key")})`}>
        {/* Immutable Secrets can't be edited; render read-only by withholding context. */}
        <ConfigDataEditor
          context={immutable ? "" : context}
          kind="Secret"
          namespace={str(meta.namespace)}
          name={str(meta.name)}
          data={data}
          secret
          onSaved={onEdited}
        />
      </Section>
    </>
  );
}

function SecretBody({
  obj,
  context = "",
  onEdited,
}: {
  obj: K8sObject;
  context?: string;
  onEdited?: () => void;
}) {
  const meta = asRecord(obj.metadata);
  const type = str(obj.type) || "Opaque";
  const redacted = asRecord(obj.data) as Record<string, string>;
  const data = useSecretData(context, str(meta.namespace), str(meta.name), redacted);
  if (type === "kubernetes.io/tls") return <TlsSecretBody data={data} />;
  if (type === "kubernetes.io/dockerconfigjson" || type === "kubernetes.io/dockercfg")
    return <DockerSecretBody data={data} type={type} />;
  return <GeneralSecretBody obj={obj} data={data} context={context} onEdited={onEdited} />;
}

/**
 * Editable key/value data for ConfigMaps and Secrets. Secret values are masked
 * until the user reveals a key (which decodes it for editing); Save only sends
 * the keys that actually changed, as plaintext (the backend patches
 * ConfigMap `data` / Secret `stringData`, so the apiserver handles encoding).
 * Editing is only enabled when a `context` is available (i.e. the live detail).
 */
export function ConfigDataEditor({
  context,
  kind,
  namespace,
  name,
  data,
  secret,
  onSaved,
}: {
  context: string;
  kind: string;
  namespace: string;
  name: string;
  /** Plaintext-or-base64 values by key. For Secrets these are the real values
   * fetched via the gated `getSecret` (see SecretBody); base64 here. */
  data: Record<string, string>;
  secret: boolean;
  onSaved?: () => void;
}) {
  // Baseline plaintext per key (Secret values decoded for editing).
  const [baseline, setBaseline] = useState<Record<string, string>>({});
  const [edited, setEdited] = useState<Record<string, string>>({});
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [copied, setCopied] = useState<string | null>(null);

  const keys = Object.keys(data);

  // Preflight the mutating RBAC (verb `update` on configmaps/secrets in this
  // namespace) so Save fails closed until the check resolves allowed.
  const updateCheck: AccessCheck = {
    verb: "update",
    resource: kind === "Secret" ? "secrets" : "configmaps",
    namespace,
  };
  const access = useAccess(context, [updateCheck]);

  // Baseline follows the data (Secret values arrive asynchronously via the
  // gated fetch); updating it here must NOT clobber the user's reveal/edit
  // state, so those reset only when the target object itself changes.
  useEffect(() => {
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(data)) out[k] = secret ? decodeBase64(str(v)) : str(v);
    setBaseline(out);
  }, [data, secret]);

  useEffect(() => {
    setEdited({});
    setRevealed({});
  }, [context, kind, namespace, name]);

  const valueOf = (k: string) => edited[k] ?? baseline[k] ?? "";
  const changedKeys = keys.filter((k) => edited[k] !== undefined && edited[k] !== (baseline[k] ?? ""));
  const editable = context !== "";

  function reveal(k: string) {
    setRevealed((r) => ({ ...r, [k]: !r[k] }));
  }

  async function copy(k: string) {
    try {
      await navigator.clipboard?.writeText(valueOf(k));
      setCopied(k);
      setTimeout(() => setCopied((c) => (c === k ? null : c)), 1500);
    } catch {
      /* clipboard unavailable — ignore */
    }
  }

  async function save() {
    setBusy(true);
    setErr("");
    const patch = Object.fromEntries(changedKeys.map((k) => [k, edited[k]]));
    const r = await updateConfigData(context, kind, namespace, name, patch);
    setBusy(false);
    if (r.error) {
      setErr(describeError(r.error).detail);
      reportActionError(context, `Failed to save ${name}`, r.error);
      return;
    }
    setEdited({});
    onSaved?.();
  }

  if (keys.length === 0) return <span className="fl-detail-empty">No data</span>;

  return (
    <div className="flex flex-col gap-3">
      {keys.map((k) => {
        const shown = !secret || revealed[k];
        return (
          <div key={k} className="fl-secret-entry">
            <div className="fl-secret-entry__header">
              <span className="fl-mono">{k}</span>
              <span className="fl-config-editor__key-actions">
                {shown && (
                  <button
                    type="button"
                    className="fl-secret-entry__toggle"
                    aria-label={`Copy value for ${k}`}
                    onClick={() => void copy(k)}
                  >
                    {copied === k ? "Copied" : "Copy"}
                  </button>
                )}
                {secret && (
                  <button
                    type="button"
                    className="fl-secret-entry__toggle"
                    onClick={() => void reveal(k)}
                  >
                    {revealed[k] ? "Hide" : "Reveal"}
                  </button>
                )}
              </span>
            </div>
            {shown ? (
              <textarea
                className="fl-config-editor__value"
                aria-label={`Value for ${k}`}
                value={valueOf(k)}
                readOnly={!editable}
                onChange={(e) => setEdited((ed) => ({ ...ed, [k]: e.target.value }))}
                rows={valueOf(k).split("\n").length > 1 ? 4 : 1}
              />
            ) : (
              <pre className="fl-secret-entry__value">••••••••</pre>
            )}
          </div>
        );
      })}
      {editable && changedKeys.length > 0 && (
        <div className="fl-config-editor__actions">
          <Button
            size="sm"
            onClick={() => void save()}
            disabled={busy || !access.allowed(updateCheck)}
            title={denyReason(access, updateCheck)}
          >
            {busy ? <Spinner label="Saving" /> : "Save"}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setEdited({})} disabled={busy}>
            Reset
          </Button>
        </div>
      )}
      {err && <p style={{ color: "var(--fl-color-danger)" }}>Error: {err}</p>}
    </div>
  );
}

function ConfigBody({
  obj,
  context = "",
  onEdited,
}: {
  obj: K8sObject;
  context?: string;
  onEdited?: () => void;
}) {
  const meta = asRecord(obj.metadata);
  const data = asRecord(obj.data) as Record<string, string>;
  const keys = Object.keys(data);
  return (
    <Section title={`Data (${plural(keys.length, "key")})`}>
      <ConfigDataEditor
        context={context}
        kind="ConfigMap"
        namespace={str(meta.namespace)}
        name={str(meta.name)}
        data={data}
        secret={false}
        onSaved={onEdited}
      />
    </Section>
  );
}

function PvcBody({
  obj,
  context = "",
  onOpenResource,
}: {
  obj: K8sObject;
  context?: string;
  onOpenResource?: OpenResource;
}) {
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const meta = asRecord(obj.metadata);
  const namespace = str(meta.namespace) || null;
  const name = str(meta.name);
  const phase = str(status.phase);

  // Pods that mount this claim — the backend scans pod volumes for the claim
  // name; completes the PVC → consuming pods navigation.
  const [podTargets, setPodTargets] = useState<ResourceTarget[]>([]);
  useEffect(() => {
    setPodTargets([]);
    if (!context || !namespace || !name) return;
    let active = true;
    void podsForPvc(context, namespace, name).then((r) => {
      if (!active) return;
      setPodTargets((r.pods ?? []).map((p) => ({ kind: "Pod", namespace, name: p.name })));
    });
    return () => {
      active = false;
    };
  }, [context, namespace, name]);

  return (
    <>
      <Section title="Volume">
        <KV
          pairs={[
            ["Status", <StatusPill key="s" status={phase || "—"} kind={phaseKind(phase)} />],
            ["Capacity", formatStorageSize(str(asRecord(status.capacity).storage))],
            ["Access modes", asArray(spec.accessModes).map(str).join(", ")],
            [
              "Storage class",
              spec.storageClassName ? (
                <ResourceLink
                  target={{ kind: "StorageClass", namespace: null, name: str(spec.storageClassName) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            [
              "Volume",
              spec.volumeName ? (
                <ResourceLink
                  target={{ kind: "PersistentVolume", namespace: null, name: str(spec.volumeName) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
          ]}
        />
      </Section>
      {podTargets.length > 0 && (
        <Section title="Consumed by">
          <LinkedResources targets={podTargets} onOpenResource={onOpenResource} />
        </Section>
      )}
    </>
  );
}

function PersistentVolumeBody({
  obj,
  onOpenResource,
}: {
  obj: K8sObject;
  onOpenResource?: OpenResource;
}) {
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const claim = asRecord(spec.claimRef);
  const phase = str(status.phase);
  const sourceType = Object.keys(spec).find((key) => PERSISTENT_VOLUME_SOURCE_TYPES.has(key)) ?? "";
  return (
    <Section title="Persistent Volume">
      <KV
        pairs={[
          ["Status", <StatusPill key="s" status={phase || "—"} kind={phaseKind(phase)} />],
          ["Capacity", formatStorageSize(str(asRecord(spec.capacity).storage))],
          ["Access modes", asArray(spec.accessModes).map(str).join(", ")],
          ["Reclaim policy", str(spec.persistentVolumeReclaimPolicy)],
          ["Volume mode", str(spec.volumeMode)],
          ["Source", sourceType ? VOLUME_TYPE_LABELS[sourceType] ?? sourceType : ""],
          [
            "Storage class",
            spec.storageClassName ? (
              <ResourceLink
                target={{ kind: "StorageClass", namespace: null, name: str(spec.storageClassName) }}
                onOpenResource={onOpenResource}
              />
            ) : (
              ""
            ),
          ],
          [
            "Claim",
            claim.name ? (
              <ResourceLink
                target={{
                  kind: "PersistentVolumeClaim",
                  namespace: str(claim.namespace) || null,
                  name: str(claim.name),
                }}
                onOpenResource={onOpenResource}
              />
            ) : (
              ""
            ),
          ],
        ]}
      />
    </Section>
  );
}

interface IngressPathRow {
  key: string;
  host: string;
  path: string;
  backend: string;
}

function IngressBody({ obj, onOpenResource }: { obj: K8sObject; onOpenResource?: OpenResource }) {
  const spec = asRecord(obj.spec);
  const namespace = str(asRecord(obj.metadata).namespace) || null;
  const rows: IngressPathRow[] = [];
  asArray(spec.rules).forEach((r, ri) => {
    const rr = asRecord(r);
    const host = str(rr.host) || "*";
    asArray(asRecord(rr.http).paths).forEach((p, pi) => {
      const pp = asRecord(p);
      const svc = asRecord(asRecord(pp.backend).service);
      const port = asRecord(svc.port);
      rows.push({
        key: `${ri}-${pi}`,
        host,
        path: str(pp.path) || "/",
        backend: `${str(svc.name)}:${str(port.number) || str(port.name)}`,
      });
    });
  });
  const cols: Column<IngressPathRow>[] = [
    { key: "host", header: "Host", render: (r) => <span className="fl-mono">{r.host}</span> },
    { key: "path", header: "Path", render: (r) => <span className="fl-mono">{r.path}</span> },
    {
      key: "backend",
      header: "Backend",
      render: (r) => {
        const serviceName = r.backend.split(":", 1)[0];
        return (
          <ResourceLink
            target={{ kind: "Service", namespace, name: serviceName }}
            onOpenResource={onOpenResource}
          >
            {r.backend}
          </ResourceLink>
        );
      },
    },
  ];
  const tls = asArray(spec.tls).flatMap((t) => asArray(asRecord(t).hosts).map(str));
  const tlsSecrets = asArray(spec.tls).map((t) => str(asRecord(t).secretName)).filter(Boolean);
  return (
    <>
      <Section title="Ingress">
        <KV
          pairs={[
            ["Class", str(spec.ingressClassName)],
            ["TLS hosts", tls.length ? tls.join(", ") : ""],
            [
              "TLS secrets",
              tlsSecrets.length ? (
                <LinkedResources
                  targets={tlsSecrets.map((name) => ({ kind: "Secret", namespace, name }))}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
          ]}
        />
      </Section>
      {rows.length > 0 && (
        <Section title="Rules">
          <Table columns={cols} data={rows} getRowKey={(r) => r.key} />
        </Section>
      )}
    </>
  );
}

function HpaBody({ obj, onOpenResource }: { obj: K8sObject; onOpenResource?: OpenResource }) {
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  const target = asRecord(spec.scaleTargetRef);
  const namespace = str(asRecord(obj.metadata).namespace) || null;
  return (
    <Section title="Autoscaler">
      <KV
        pairs={[
          [
            "Scale target",
            target.name ? (
              <ResourceLink
                target={{ kind: str(target.kind), namespace, name: str(target.name) }}
                onOpenResource={onOpenResource}
              >
                {str(target.kind)}/{str(target.name)}
              </ResourceLink>
            ) : (
              ""
            ),
          ],
          [
            "Replicas",
            `${str(status.currentReplicas) || "?"} current / ${str(status.desiredReplicas) || "?"} desired`,
          ],
          ["Min replicas", str(spec.minReplicas)],
          ["Max replicas", str(spec.maxReplicas)],
        ]}
      />
    </Section>
  );
}

interface QuotaRow {
  key: string;
  resource: string;
  used: string;
  hard: string;
}

/** Parse a Kubernetes quantity (e.g. "500m", "2Gi", "4") to a base-unit number. */
export function parseQuantity(q: string): number | null {
  const m = /^([0-9.]+)\s*([a-zA-Z]*)$/.exec((q ?? "").trim());
  if (!m) return null;
  const n = parseFloat(m[1]);
  if (Number.isNaN(n)) return null;
  const unit = m[2];
  const binary: Record<string, number> = { Ki: 2 ** 10, Mi: 2 ** 20, Gi: 2 ** 30, Ti: 2 ** 40, Pi: 2 ** 50, Ei: 2 ** 60 };
  const decimal: Record<string, number> = { k: 1e3, M: 1e6, G: 1e9, T: 1e12, P: 1e15, E: 1e18 };
  if (unit === "") return n;
  if (unit === "m") return n / 1000;
  if (binary[unit]) return n * binary[unit];
  if (decimal[unit]) return n * decimal[unit];
  return n; // unknown unit — same on both sides, so the ratio still holds
}

function usagePercent(used: string, hard: string): number | null {
  const u = parseQuantity(used);
  const h = parseQuantity(hard);
  if (u == null || h == null || h === 0) return null;
  return Math.round((u / h) * 100);
}

function ResourceQuotaBody({ obj }: { obj: K8sObject }) {
  const status = asRecord(obj.status);
  const hard = asRecord(status.hard);
  const used = asRecord(status.used);
  const rows: QuotaRow[] = Object.keys(hard).map((k) => ({
    key: k,
    resource: k,
    used: str(used[k]),
    hard: str(hard[k]),
  }));
  const cols: Column<QuotaRow>[] = [
    { key: "resource", header: "Resource", render: (r) => <span className="fl-mono">{r.resource}</span> },
    { key: "used", header: "Used", render: (r) => r.used },
    { key: "hard", header: "Hard", render: (r) => r.hard },
    {
      key: "usage",
      header: "Usage",
      render: (r) => {
        const pct = usagePercent(r.used, r.hard);
        if (pct == null) return <span className="fl-detail-empty">—</span>;
        const kind: StatusKind = pct >= 90 ? "danger" : pct >= 75 ? "warning" : "success";
        return (
          <div
            className={`fl-usage-bar fl-usage-bar--${kind}`}
            role="progressbar"
            aria-label={`${r.resource} usage`}
            aria-valuenow={pct}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <div className="fl-usage-bar__fill" style={{ width: `${Math.min(100, pct)}%` }} />
            <span className="fl-usage-bar__label">{pct}%</span>
          </div>
        );
      },
    },
  ];
  return (
    <Section title="Quota">
      {rows.length ? (
        <Table columns={cols} data={rows} getRowKey={(r) => r.key} />
      ) : (
        <span className="fl-detail-empty">No quota</span>
      )}
    </Section>
  );
}

function PdbBody({ obj }: { obj: K8sObject }) {
  const spec = asRecord(obj.spec);
  const status = asRecord(obj.status);
  return (
    <Section title="Disruption Budget">
      <KV
        pairs={[
          ["Min available", str(spec.minAvailable)],
          ["Max unavailable", str(spec.maxUnavailable)],
          ["Healthy", `${str(status.currentHealthy)} / ${str(status.desiredHealthy)}`],
          ["Disruptions allowed", str(status.disruptionsAllowed)],
        ]}
      />
    </Section>
  );
}

function NetworkPolicyBody({ obj }: { obj: K8sObject }) {
  const spec = asRecord(obj.spec);
  const sel = asRecord(asRecord(spec.podSelector).matchLabels) as Record<string, string>;
  return (
    <Section title="Network Policy">
      <KV
        pairs={[
          ["Pod selector", Object.keys(sel).length ? <Chips key="s" map={sel} /> : "all pods"],
          ["Policy types", asArray(spec.policyTypes).map(str).join(", ")],
          ["Ingress rules", str(asArray(spec.ingress).length)],
          ["Egress rules", str(asArray(spec.egress).length)],
        ]}
      />
    </Section>
  );
}

function ServiceAccountBody({
  obj,
  context = "",
  onOpenResource,
}: {
  obj: K8sObject;
  context?: string;
  onOpenResource?: OpenResource;
}) {
  const secrets = asArray(obj.secrets).map((s) => str(asRecord(s).name)).filter(Boolean);
  const pull = asArray(obj.imagePullSecrets).map((s) => str(asRecord(s).name)).filter(Boolean);
  const meta = asRecord(obj.metadata);
  const namespace = str(meta.namespace) || null;
  const name = str(meta.name);

  // "What can this SA do?" — the (Cluster)RoleBindings that grant it permissions
  // and the pods that run as it. Both are reverse lookups the backend resolves.
  const [bindings, setBindings] = useState<SaBinding[]>([]);
  const [podTargets, setPodTargets] = useState<ResourceTarget[]>([]);
  useEffect(() => {
    setBindings([]);
    setPodTargets([]);
    if (!context || !namespace || !name) return;
    let active = true;
    void bindingsForServiceAccount(context, namespace, name).then((r) => {
      if (active) setBindings(r.bindings ?? []);
    });
    void podsForServiceAccount(context, namespace, name).then((r) => {
      if (active) setPodTargets((r.pods ?? []).map((p) => ({ kind: "Pod", namespace, name: p.name })));
    });
    return () => {
      active = false;
    };
  }, [context, namespace, name]);

  return (
    <>
      <Section title="Service Account">
        <KV
          pairs={[
            [
              "Secrets",
              secrets.length ? (
                <LinkedResources
                  targets={secrets.map((name) => ({ kind: "Secret", namespace, name }))}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            [
              "Image pull secrets",
              pull.length ? (
                <LinkedResources
                  targets={pull.map((name) => ({ kind: "Secret", namespace, name }))}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            ["Automount token", obj.automountServiceAccountToken === false ? "No" : "Yes"],
          ]}
        />
      </Section>
      {(bindings.length > 0 || podTargets.length > 0) && (
        <Section title="Used by">
          <KV
            pairs={[
              [
                "Bindings",
                bindings.length ? (
                  <div className="fl-sa-bindings">
                    {bindings.map((b, i) => (
                      <div key={`${b.kind}/${b.name}/${i}`}>
                        <ResourceLink
                          target={{ kind: b.kind, namespace: b.namespace, name: b.name }}
                          onOpenResource={onOpenResource}
                        >
                          {b.kind}/{b.name}
                        </ResourceLink>{" "}
                        grants <span className="fl-mono">{b.role}</span>
                      </div>
                    ))}
                  </div>
                ) : (
                  ""
                ),
              ],
              [
                "Pods",
                podTargets.length ? (
                  <LinkedResources targets={podTargets} onOpenResource={onOpenResource} />
                ) : (
                  ""
                ),
              ],
            ]}
          />
        </Section>
      )}
    </>
  );
}

function portText2(p: Record<string, unknown>): string {
  const name = str(p.name);
  return `${name ? `${name}: ` : ""}${str(p.port)}/${str(p.protocol) || "TCP"}`;
}

function EndpointsBody({ obj, onOpenResource }: { obj: K8sObject; onOpenResource?: OpenResource }) {
  const addrs: string[] = [];
  const ports: string[] = [];
  const namespace = str(asRecord(obj.metadata).namespace) || null;
  const targets: ResourceTarget[] = [];
  asArray(obj.subsets).forEach((s) => {
    const ss = asRecord(s);
    [...asArray(ss.addresses), ...asArray(ss.notReadyAddresses)].forEach((a) => {
      const address = asRecord(a);
      addrs.push(str(address.ip));
      const target = asRecord(address.targetRef);
      if (target.name) {
        const kind = str(target.kind);
        targets.push({
          kind,
          namespace: targetNamespace(kind, str(target.namespace) || namespace),
          name: str(target.name),
        });
      }
    });
    asArray(ss.ports).forEach((p) => ports.push(portText2(asRecord(p))));
  });
  return (
    <Section title="Endpoints">
      <KV
        pairs={[
          ["Addresses", addrs.length ? <PlainChips key="a" items={addrs} /> : "None"],
          ["Ports", ports.length ? <PlainChips key="p" items={ports} /> : ""],
          [
            "Targets",
            targets.length ? (
              <LinkedResources targets={targets} onOpenResource={onOpenResource} />
            ) : (
              ""
            ),
          ],
        ]}
      />
    </Section>
  );
}

function EndpointSliceBody({ obj, onOpenResource }: { obj: K8sObject; onOpenResource?: OpenResource }) {
  const addrs = asArray(obj.endpoints).flatMap((e) => asArray(asRecord(e).addresses).map(str));
  const ports = asArray(obj.ports).map((p) => portText2(asRecord(p)));
  const namespace = str(asRecord(obj.metadata).namespace) || null;
  const targets = asArray(obj.endpoints).flatMap((endpoint): ResourceTarget[] => {
    const target = asRecord(asRecord(endpoint).targetRef);
    if (!target.name) return [];
    const kind = str(target.kind);
    return [{
      kind,
      namespace: targetNamespace(kind, str(target.namespace) || namespace),
      name: str(target.name),
    }];
  });
  return (
    <Section title="Endpoint Slice">
      <KV
        pairs={[
          ["Address type", str(obj.addressType)],
          ["Addresses", addrs.length ? <PlainChips key="a" items={addrs} /> : "None"],
          ["Ports", ports.length ? <PlainChips key="p" items={ports} /> : ""],
          [
            "Targets",
            targets.length ? (
              <LinkedResources targets={targets} onOpenResource={onOpenResource} />
            ) : (
              ""
            ),
          ],
        ]}
      />
    </Section>
  );
}

interface RuleRow {
  key: string;
  apiGroups: string;
  resources: string;
  verbs: string;
}

function RoleBody({ obj }: { obj: K8sObject }) {
  const rows: RuleRow[] = asArray(obj.rules).map((r, i) => {
    const rr = asRecord(r);
    return {
      key: String(i),
      apiGroups: asArray(rr.apiGroups).map((g) => str(g) || "*").join(", ") || "*",
      resources: asArray(rr.resources).map(str).join(", "),
      verbs: asArray(rr.verbs).map(str).join(", "),
    };
  });
  const cols: Column<RuleRow>[] = [
    { key: "apiGroups", header: "API Groups", render: (r) => <span className="fl-mono">{r.apiGroups}</span> },
    { key: "resources", header: "Resources", render: (r) => <span className="fl-mono">{r.resources}</span> },
    { key: "verbs", header: "Verbs", render: (r) => <span className="fl-mono">{r.verbs}</span> },
  ];
  return (
    <Section title={`Rules (${rows.length})`}>
      {rows.length ? (
        <Table columns={cols} data={rows} getRowKey={(r) => r.key} />
      ) : (
        <span className="fl-detail-empty">No rules</span>
      )}
    </Section>
  );
}

interface SubjectRow {
  key: string;
  kind: string;
  name: string;
  namespace: string;
}

function RoleBindingBody({ obj, onOpenResource }: { obj: K8sObject; onOpenResource?: OpenResource }) {
  const roleRef = asRecord(obj.roleRef);
  const bindingNamespace = str(asRecord(obj.metadata).namespace) || null;
  const subjects: SubjectRow[] = asArray(obj.subjects).map((s, i) => {
    const ss = asRecord(s);
    return { key: String(i), kind: str(ss.kind), name: str(ss.name), namespace: str(ss.namespace) };
  });
  const cols: Column<SubjectRow>[] = [
    { key: "kind", header: "Kind", render: (r) => r.kind },
    {
      key: "name",
      header: "Name",
      render: (r) =>
        r.kind === "ServiceAccount" ? (
          <ResourceLink
            target={{ kind: r.kind, namespace: r.namespace || bindingNamespace, name: r.name }}
            onOpenResource={onOpenResource}
          />
        ) : (
          <span className="fl-mono">{r.name}</span>
        ),
    },
    { key: "namespace", header: "Namespace", render: (r) => <span className="fl-mono">{r.namespace || "—"}</span> },
  ];
  return (
    <>
      <Section title="Role Ref">
        <KV
          pairs={[
            ["Kind", str(roleRef.kind)],
            [
              "Name",
              <ResourceLink
                key="n"
                target={{
                  kind: str(roleRef.kind),
                  namespace: targetNamespace(str(roleRef.kind), bindingNamespace),
                  name: str(roleRef.name),
                }}
                onOpenResource={onOpenResource}
              />,
            ],
          ]}
        />
      </Section>
      {subjects.length > 0 && (
        <Section title={`Subjects (${subjects.length})`}>
          <Table columns={cols} data={subjects} getRowKey={(r) => r.key} />
        </Section>
      )}
    </>
  );
}

function PriorityClassBody({ obj }: { obj: K8sObject }) {
  return (
    <Section title="Priority Class">
      <KV
        pairs={[
          ["Value", str(obj.value)],
          ["Global default", obj.globalDefault === true ? "Yes" : "No"],
          ["Preemption policy", str(obj.preemptionPolicy)],
        ]}
      />
    </Section>
  );
}

function StorageClassBody({ obj }: { obj: K8sObject }) {
  return (
    <Section title="Storage Class">
      <KV
        pairs={[
          ["Provisioner", <span key="p" className="fl-mono">{str(obj.provisioner)}</span>],
          ["Reclaim policy", str(obj.reclaimPolicy)],
          ["Volume binding mode", str(obj.volumeBindingMode)],
          ["Allow expansion", obj.allowVolumeExpansion === true ? "Yes" : "No"],
        ]}
      />
    </Section>
  );
}

function RuntimeClassBody({ obj }: { obj: K8sObject }) {
  return (
    <Section title="Runtime Class">
      <KV pairs={[["Handler", <span key="h" className="fl-mono">{str(obj.handler)}</span>]]} />
    </Section>
  );
}

function IngressClassBody({ obj }: { obj: K8sObject }) {
  const spec = asRecord(obj.spec);
  return (
    <Section title="Ingress Class">
      <KV pairs={[["Controller", <span key="c" className="fl-mono">{str(spec.controller)}</span>]]} />
    </Section>
  );
}

function LimitRangeBody({ obj }: { obj: K8sObject }) {
  const limits = asArray(asRecord(obj.spec).limits);
  return (
    <Section title={`Limits (${limits.length})`}>
      {limits.length ? (
        <KV
          pairs={limits.map((l, i) => {
            const ll = asRecord(l);
            const constraints = Object.keys({
              ...asRecord(ll.default),
              ...asRecord(ll.max),
              ...asRecord(ll.min),
            });
            return [str(ll.type) || `Limit ${i + 1}`, constraints.join(", ") || "—"] as Pair;
          })}
        />
      ) : (
        <span className="fl-detail-empty">None</span>
      )}
    </Section>
  );
}

function LeaseBody({ obj }: { obj: K8sObject }) {
  const spec = asRecord(obj.spec);
  const renew = str(spec.renewTime);
  return (
    <Section title="Lease">
      <KV
        pairs={[
          ["Holder", <span key="h" className="fl-mono">{str(spec.holderIdentity)}</span>],
          ["Duration", spec.leaseDurationSeconds != null ? `${str(spec.leaseDurationSeconds)}s` : ""],
          ["Renewed", renew ? `${ageFromTimestamp(renew)} ago` : ""],
        ]}
      />
    </Section>
  );
}

function WebhookBody({ obj }: { obj: K8sObject }) {
  const webhooks = asArray(obj.webhooks).map((w) => str(asRecord(w).name)).filter(Boolean);
  return (
    <Section title={`Webhooks (${webhooks.length})`}>
      {webhooks.length ? (
        <PlainChips items={webhooks} />
      ) : (
        <span className="fl-detail-empty">None</span>
      )}
    </Section>
  );
}

function KindBody({
  kind,
  obj,
  context = "",
  now = Date.now(),
  onEdited,
  onOpenResource,
}: {
  kind: string;
  obj: K8sObject;
  context?: string;
  now?: number;
  onEdited?: () => void;
  onOpenResource?: OpenResource;
}) {
  switch (kind) {
    case "DaemonSet":
      return <DaemonSetBody obj={obj} />;
    case "Service":
      return <ServiceBody obj={obj} context={context} onOpenResource={onOpenResource} />;
    case "Node":
      return <NodeBody obj={obj} />;
    case "Job":
      return <JobBody obj={obj} now={now} />;
    case "CronJob":
      return <CronJobBody obj={obj} now={now} context={context} onOpenResource={onOpenResource} />;
    case "ConfigMap":
      return <ConfigBody obj={obj} context={context} onEdited={onEdited} />;
    case "Secret":
      return <SecretBody obj={obj} context={context} onEdited={onEdited} />;
    case "PersistentVolumeClaim":
      return <PvcBody obj={obj} context={context} onOpenResource={onOpenResource} />;
    case "PersistentVolume":
      return <PersistentVolumeBody obj={obj} onOpenResource={onOpenResource} />;
    case "Ingress":
      return <IngressBody obj={obj} onOpenResource={onOpenResource} />;
    case "HorizontalPodAutoscaler":
      return <HpaBody obj={obj} onOpenResource={onOpenResource} />;
    case "ResourceQuota":
      return <ResourceQuotaBody obj={obj} />;
    case "PodDisruptionBudget":
      return <PdbBody obj={obj} />;
    case "NetworkPolicy":
      return <NetworkPolicyBody obj={obj} />;
    case "ServiceAccount":
      return <ServiceAccountBody obj={obj} context={context} onOpenResource={onOpenResource} />;
    case "Endpoints":
      return <EndpointsBody obj={obj} onOpenResource={onOpenResource} />;
    case "EndpointSlice":
      return <EndpointSliceBody obj={obj} onOpenResource={onOpenResource} />;
    case "Role":
    case "ClusterRole":
      return <RoleBody obj={obj} />;
    case "RoleBinding":
    case "ClusterRoleBinding":
      return <RoleBindingBody obj={obj} onOpenResource={onOpenResource} />;
    case "PriorityClass":
      return <PriorityClassBody obj={obj} />;
    case "StorageClass":
      return <StorageClassBody obj={obj} />;
    case "RuntimeClass":
      return <RuntimeClassBody obj={obj} />;
    case "IngressClass":
      return <IngressClassBody obj={obj} />;
    case "LimitRange":
      return <LimitRangeBody obj={obj} />;
    case "Lease":
      return <LeaseBody obj={obj} />;
    case "MutatingWebhookConfiguration":
    case "ValidatingWebhookConfiguration":
      return <WebhookBody obj={obj} />;
    default:
      return null;
  }
}

function relatedPodSelector(kind: string, obj: K8sObject): Record<string, string> {
  const spec = asRecord(obj.spec);
  switch (kind) {
    case "Service":
      return asRecord(spec.selector) as Record<string, string>;
    case "DaemonSet":
    case "Job":
      return asRecord(asRecord(spec.selector).matchLabels) as Record<string, string>;
    case "PodDisruptionBudget":
      return asRecord(asRecord(spec.selector).matchLabels) as Record<string, string>;
    case "NetworkPolicy":
      return asRecord(asRecord(spec.podSelector).matchLabels) as Record<string, string>;
    default:
      return {};
  }
}

/** Generic detail layout for non-Pod kinds: metadata + kind body + conditions. */
function GenericDetail({
  kind,
  obj,
  now,
  context = "",
  onEdited,
  onOpenResource,
}: {
  kind: string;
  obj: K8sObject;
  now: number;
  context?: string;
  onEdited?: () => void;
  onOpenResource?: OpenResource;
}) {
  const meta = asRecord(obj.metadata);
  const namespace = str(meta.namespace) || null;
  const owners = asArray(meta.ownerReferences).map((o) => {
    const or = asRecord(o);
    const kind = str(or.kind);
    return { kind, name: str(or.name), namespace: targetNamespace(kind, namespace) };
  }).filter((target) => target.kind && target.name);
  const conditions = asArray(asRecord(obj.status).conditions) as unknown as Condition[];
  const created = str(meta.creationTimestamp);
  const podSelector = relatedPodSelector(kind, obj);

  return (
    <div className="fl-detail">
      <Section title="Metadata">
        <KV
          pairs={[
            ["Name", <span className="fl-mono">{str(meta.name)}</span>],
            [
              "Namespace",
              meta.namespace ? (
                <ResourceLink
                  target={{ kind: "Namespace", namespace: null, name: str(meta.namespace) }}
                  onOpenResource={onOpenResource}
                />
              ) : (
                ""
              ),
            ],
            ["Created", created ? `${ageFromTimestamp(created, now)} ago (${absoluteTimestamp(created)})` : ""],
            [
              "Controlled by",
              owners.length ? (
                <LinkedResources targets={owners} onOpenResource={onOpenResource} />
              ) : (
                ""
              ),
            ],
          ]}
        />
        <div className="fl-detail-subhead">Labels</div>
        <ChipMap map={meta.labels as Record<string, string>} singular="Label" />
        <div className="fl-detail-subhead">Annotations</div>
        <ChipMap map={meta.annotations as Record<string, string>} singular="Annotation" />
      </Section>

      <KindBody kind={kind} obj={obj} context={context} now={now} onEdited={onEdited} onOpenResource={onOpenResource} />

      {context && namespace && Object.keys(podSelector).length > 0 && (
        <ManagedPods
          context={context}
          namespace={namespace}
          selector={podSelector}
          onOpenResource={onOpenResource}
        />
      )}

      <ConditionsTable conditions={conditions} now={now} />
    </div>
  );
}

/** Render the structured detail of a fetched object. Exported for testing. */
export function ObjectDetail({
  kind,
  obj,
  now,
  context = "",
  onEdited,
  onOpenResource,
  onOpenLogs,
  onOpenExec,
}: {
  kind: string;
  obj: K8sObject;
  now: number;
  context?: string;
  onEdited?: () => void;
  onOpenResource?: OpenResource;
  /** Open logs scoped to a container (Pod detail only). */
  onOpenLogs?: (container: string) => void;
  /** Open an exec session in a container (Pod detail only). */
  onOpenExec?: (container: string) => void;
}) {
  const meta = asRecord(obj.metadata);
  // Metrics chart sits above the rest, matching Lens. Pods and Nodes read their
  // own usage; workload controllers aggregate the usage of their pods (matched
  // by the controller's label selector). Needs a context to poll; in tests
  // without one it's simply omitted.
  const WORKLOAD_METRIC_KINDS = ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"];
  const workloadSelector = asRecord(asRecord(obj.spec).selector).matchLabels as
    | Record<string, string>
    | undefined;
  const metrics =
    context && (kind === "Pod" || kind === "Node") ? (
      <MetricsPanel
        kind={kind}
        context={context}
        namespace={str(meta.namespace) || null}
        name={str(meta.name)}
      />
    ) : context && WORKLOAD_METRIC_KINDS.includes(kind) && workloadSelector ? (
      <MetricsPanel
        kind={kind as "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Job"}
        context={context}
        namespace={str(meta.namespace) || null}
        name={str(meta.name)}
        selector={workloadSelector}
      />
    ) : null;

  if (kind === "Pod")
    return (
      <>
        {metrics}
        <PodDetailView
          obj={obj}
          now={now}
          context={context}
          onOpenResource={onOpenResource}
          onOpenLogs={onOpenLogs}
          onOpenExec={onOpenExec}
        />
      </>
    );
  if (kind === "Deployment" || kind === "StatefulSet" || kind === "ReplicaSet")
    return (
      <>
        {metrics}
        <WorkloadDetailView
          kind={kind}
          obj={obj}
          now={now}
          context={context}
          onOpenResource={onOpenResource}
        />
      </>
    );
  return (
    <>
      {metrics}
      <GenericDetail
        kind={kind}
        obj={obj}
        now={now}
        context={context}
        onEdited={onEdited}
        onOpenResource={onOpenResource}
      />
    </>
  );
}

/* ------------------------------------------------------------------ */
/* the overview (data loader)                                          */
/* ------------------------------------------------------------------ */

/**
 * Structured detail panel for a resource. Fetches the object via `k8s.getObject`
 * and renders metadata, status, and kind-specific sections. `getObjectFn` is
 * injectable for testing.
 */
export function ResourceOverview({
  context,
  kind,
  namespace,
  name,
  getObjectFn = getObject,
  reloadKey = 0,
  onOpenResource,
  onOpenLogs,
  onOpenExec,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
  getObjectFn?: typeof getObject;
  /** Bumped by the parent after a write action to re-fetch the shown object. */
  reloadKey?: number;
  onOpenResource?: OpenResource;
  /** Open logs scoped to a container (Pod detail only). */
  onOpenLogs?: (container: string) => void;
  /** Open an exec session in a container (Pod detail only). */
  onOpenExec?: (container: string) => void;
}) {
  const [obj, setObj] = useState<K8sObject | null>(null);
  const [error, setError] = useState("");
  // Bumped locally after an in-place edit saves, to re-fetch the fresh object.
  const [editReload, setEditReload] = useState(0);

  useEffect(() => {
    let active = true;
    setObj(null);
    setError("");
    void getObjectFn(context, kind, namespace, name).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else setObj(out.object ?? {});
    });
    return () => {
      active = false;
    };
  }, [context, kind, namespace, name, getObjectFn, reloadKey, editReload]);

  if (error) return <p style={{ color: "var(--fl-color-danger)" }}>Error: {error}</p>;
  if (obj === null) return <Spinner label="Loading details" />;
  return (
    <ObjectDetail
      kind={kind}
      obj={obj}
      now={Date.now()}
      context={context}
      onEdited={() => setEditReload((n) => n + 1)}
      onOpenResource={onOpenResource}
      onOpenLogs={onOpenLogs}
      onOpenExec={onOpenExec}
    />
  );
}
