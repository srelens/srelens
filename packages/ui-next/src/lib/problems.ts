import { eventVerdict, nodeStatus, podStatus, type HealthKind, type NodeSummary } from "@srelens/core";
import { eventRouteGroup } from "./eventRoute";
import type { PodRow } from "./kinds/columns";
import { involvedObject, type EventRow } from "./kinds/events";

/** One hour: the same window Home uses for a warning that is still current. */
export const RECENT_WARNING_MS = 60 * 60 * 1000;

export type ProblemSource = "pod" | "node" | "event";
export type ProblemSeverity = "critical" | "warning" | "unknown";

/** One actionable row on the cluster Problems screen. */
export interface ProblemRow {
  /** Stable within a cluster snapshot, including the source so an event and
   *  the resource it describes remain two explicitly different facts. */
  key: string;
  source: ProblemSource;
  severity: ProblemSeverity;
  health: HealthKind;
  kind: string;
  /** Set only for a custom involved object. */
  group?: string;
  /** Empty for a cluster-scoped object such as a Node. */
  namespace: string;
  name: string;
  /** Core's status word, or the warning event's reason. */
  reason: string;
  /** Supporting text that does not replace the reason. */
  detail?: string;
  created?: string | null;
  age?: string;
  /** Sum of repeated warning-event counts; absent for resource health rows. */
  count?: number;
}

const SEVERITY: Record<ProblemSeverity, number> = {
  critical: 0,
  warning: 1,
  unknown: 2,
};

function severity(health: HealthKind): ProblemSeverity {
  if (health === "danger") return "critical";
  if (health === "warning") return "warning";
  return "unknown";
}

function key(row: Pick<ProblemRow, "source" | "group" | "kind" | "namespace" | "name">): string {
  return [row.source, row.group ?? "", row.kind, row.namespace, row.name].join("\u0000");
}

const PRESSURE_TAINTS: Readonly<Record<string, string>> = {
  "node.kubernetes.io/memory-pressure": "MemoryPressure",
  "node.kubernetes.io/disk-pressure": "DiskPressure",
  "node.kubernetes.io/pid-pressure": "PIDPressure",
  "node.kubernetes.io/network-unavailable": "NetworkUnavailable",
};

function pressureWords(node: NodeSummary): string[] {
  return [...new Set(
    (node.taintDetails ?? [])
      .map((taint) => PRESSURE_TAINTS[taint.key])
      .filter((word): word is string => word !== undefined),
  )];
}

interface ProblemInput {
  pods: readonly PodRow[];
  nodes: readonly NodeSummary[];
  events: readonly EventRow[];
  now?: number;
}

interface EventGroup {
  latest: EventRow;
  latestAt: number;
  reasons: Set<string>;
  count: number;
  kind: string;
  group?: string;
  namespace: string;
  name: string;
}

/**
 * Derive the screen's rows from the three existing live-list surfaces.
 *
 * Health is never guessed from colour or copied predicates: pods and nodes use
 * the same core verdicts as their lists and detail headers. Warning events are
 * grouped by involved object, so a noisy object remains one row while its API
 * count is preserved.
 */
export function deriveProblems({ pods, nodes, events, now = Date.now() }: ProblemInput): ProblemRow[] {
  const rows: ProblemRow[] = [];

  for (const pod of pods) {
    const verdict = podStatus(pod);
    if (!verdict.flagged) continue;
    const row: ProblemRow = {
      key: "",
      source: "pod",
      severity: severity(verdict.health),
      health: verdict.health,
      kind: "Pod",
      namespace: pod.namespace,
      name: pod.name,
      reason: String(verdict.status),
      detail: `${pod.ready} ready · ${pod.restarts} restarts`,
      created: pod.created,
      age: pod.age,
    };
    row.key = key(row);
    rows.push(row);
  }

  for (const node of nodes) {
    const verdict = nodeStatus(node.status, node.unschedulable);
    const pressure = pressureWords(node);
    if (!verdict.flagged && pressure.length === 0) continue;
    const health: HealthKind = pressure.length > 0 ? "danger" : verdict.health;
    const words = [...new Set([...(verdict.flagged ? [String(verdict.status)] : []), ...pressure])];
    const row: ProblemRow = {
      key: "",
      source: "node",
      severity: severity(health),
      health,
      kind: "Node",
      namespace: "",
      name: node.name,
      reason: words.join(","),
      detail: node.roles ? `${node.roles} · ${node.version}` : node.version,
      created: node.created,
      age: node.age,
    };
    row.key = key(row);
    rows.push(row);
  }

  const grouped = new Map<string, EventGroup>();
  for (const event of events) {
    if (!eventVerdict(event.type).bad) continue;
    const at = Date.parse(event.created ?? "");
    if (!Number.isFinite(at) || now - at > RECENT_WARNING_MS) continue;
    const object = involvedObject(event);
    if (!object.kind || !object.name) continue;
    const group = eventRouteGroup(object.kind, event.objectApiVersion);
    const groupKey = [group ?? "", object.kind, event.namespace, object.name].join("\u0000");
    const held = grouped.get(groupKey);
    if (!held) {
      grouped.set(groupKey, {
        latest: event,
        latestAt: at,
        reasons: new Set([event.reason]),
        count: Math.max(1, event.count),
        kind: object.kind,
        ...(group ? { group } : {}),
        namespace: event.namespace,
        name: object.name,
      });
      continue;
    }
    held.reasons.add(event.reason);
    held.count += Math.max(1, event.count);
    if (at > held.latestAt) {
      held.latest = event;
      held.latestAt = at;
    }
  }

  for (const event of grouped.values()) {
    const otherReasons = event.reasons.size - 1;
    const reason = otherReasons > 0
      ? `${event.latest.reason} + ${otherReasons} other ${otherReasons === 1 ? "warning" : "warnings"}`
      : event.latest.reason;
    const health = eventVerdict(event.latest.type).health;
    const row: ProblemRow = {
      key: "",
      source: "event",
      severity: severity(health),
      health,
      kind: event.kind,
      ...(event.group ? { group: event.group } : {}),
      namespace: event.namespace,
      name: event.name,
      reason,
      detail: event.latest.message,
      created: event.latest.created,
      age: event.latest.age,
      count: event.count,
    };
    row.key = key(row);
    rows.push(row);
  }

  return rows.sort((a, b) =>
    SEVERITY[a.severity] - SEVERITY[b.severity]
      || a.kind.localeCompare(b.kind)
      || a.namespace.localeCompare(b.namespace)
      || a.name.localeCompare(b.name)
      || a.source.localeCompare(b.source),
  );
}
