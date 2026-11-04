import { describe, expect, it } from "vitest";
import type { EventSummary, NodeSummary, PodSummary } from "@srelens/core";
import type { EventRow } from "./kinds/events";
import type { PodRow } from "./kinds/columns";
import { deriveProblems, type ProblemRow } from "./problems";

const NOW = Date.parse("2026-11-04T12:00:00Z");

const pod = (name: string, status: string, over: Partial<PodSummary> = {}): PodRow => ({
  name,
  namespace: "checkout",
  phase: "Running",
  status,
  ready: "0/1",
  restarts: 3,
  node: "worker-1",
  image: "example/web:1",
  age: "3m",
  created: "2026-11-04T11:57:00Z",
  ...over,
} as PodRow);

const node = (name: string, over: Partial<NodeSummary> = {}): NodeSummary => ({
  name,
  status: "Ready",
  unschedulable: false,
  taints: 0,
  taintDetails: [],
  version: "v1.34.0",
  roles: "worker",
  age: "3d",
  created: "2026-11-01T12:00:00Z",
  allocatableCpuMillicores: 4_000,
  allocatableMemoryMiB: 16_384,
  allocatablePods: 110,
  instanceType: "m7i.xlarge",
  ...over,
});

const warning = (
  name: string,
  object: string,
  reason: string,
  minutesAgo: number,
  over: Partial<EventSummary> = {},
): EventRow => ({
  name,
  namespace: "checkout",
  type: "Warning",
  reason,
  object,
  message: `${reason} happened`,
  created: new Date(NOW - minutesAgo * 60_000).toISOString(),
  age: `${minutesAgo}m`,
  count: 1,
  ...over,
} as EventRow);

const brief = (rows: ProblemRow[]) => rows.map((row) =>
  `${row.source}:${row.severity}:${row.kind}:${row.namespace}/${row.name}:${row.reason}`,
);

describe("deriveProblems", () => {
  it("uses core's pod verdict, including Pending and Failed, and omits healthy or completed pods", () => {
    const rows = deriveProblems({
      pods: [
        pod("crashing", "CrashLoopBackOff"),
        pod("waiting", "Pending", { phase: "Pending" }),
        pod("failed", "Failed", { phase: "Failed" }),
        pod("healthy", "Running", { ready: "1/1" }),
        pod("finished", "Completed", { phase: "Succeeded", ready: "0/1" }),
      ],
      nodes: [],
      events: [],
      now: NOW,
    });

    expect(brief(rows)).toEqual([
      "pod:critical:Pod:checkout/crashing:CrashLoopBackOff",
      "pod:critical:Pod:checkout/failed:Failed",
      "pod:warning:Pod:checkout/waiting:Pending",
    ]);
  });

  it("surfaces NotReady, cordoned, and pressure-tainted nodes without inventing trouble on a healthy node", () => {
    const rows = deriveProblems({
      pods: [],
      nodes: [
        node("not-ready", { status: "NotReady" }),
        node("cordoned", { unschedulable: true }),
        node("pressured", {
          taints: 2,
          taintDetails: [
            { key: "node.kubernetes.io/memory-pressure", value: "", effect: "NoSchedule" },
            { key: "node.kubernetes.io/disk-pressure", value: "", effect: "NoSchedule" },
          ],
        }),
        node("healthy"),
      ],
      events: [],
      now: NOW,
    });

    expect(brief(rows)).toEqual([
      "node:critical:Node:/not-ready:NotReady",
      "node:critical:Node:/pressured:MemoryPressure,DiskPressure",
      "node:warning:Node:/cordoned:Ready,SchedulingDisabled",
    ]);
  });

  it("keeps only recent Warning events, groups repeats by involved object, and retains custom groups", () => {
    const rows = deriveProblems({
      pods: [],
      nodes: [],
      events: [
        warning("a", "Pod/web-7d4b", "BackOff", 2, { count: 3 }),
        warning("b", "Pod/web-7d4b", "FailedMount", 4, { count: 2 }),
        warning("c", "Deployment/api", "ReconcileFailed", 5, { objectApiVersion: "acme.io/v1" }),
        warning("d", "Pod/old", "BackOff", 61),
        warning("e", "Pod/normal", "Scheduled", 1, { type: "Normal" }),
        warning("f", "unparseable", "Odd", 1),
      ],
      now: NOW,
    });

    expect(rows).toHaveLength(2);
    expect(rows.find((row) => row.kind === "Pod")).toMatchObject({
      source: "event",
      severity: "critical",
      kind: "Pod",
      namespace: "checkout",
      name: "web-7d4b",
      reason: "BackOff + 1 other warning",
      count: 5,
    });
    expect(rows.find((row) => row.kind === "Deployment")).toMatchObject({
      source: "event",
      group: "acme.io",
      kind: "Deployment",
      name: "api",
      reason: "ReconcileFailed",
      count: 1,
    });
  });

  it("orders critical before warning before unknown, then keeps rows stable by kind, namespace, and name", () => {
    const rows = deriveProblems({
      pods: [
        pod("z-pending", "Pending", { phase: "Pending" }),
        pod("a-crash", "CrashLoopBackOff"),
        pod("unknown", "BrandNewState", { phase: "BrandNewState" }),
      ],
      nodes: [node("b-node", { status: "NotReady" })],
      events: [],
      now: NOW,
    });

    expect(rows.map((row) => `${row.severity}:${row.kind}/${row.name}`)).toEqual([
      "critical:Node/b-node",
      "critical:Pod/a-crash",
      "warning:Pod/z-pending",
      "unknown:Pod/unknown",
    ]);
    expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
  });
});
