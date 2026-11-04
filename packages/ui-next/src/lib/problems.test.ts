import { describe, expect, it } from "vitest";
import type { EventSummary, NodeSummary, PodSummary } from "@srelens/core";
import type { EventRow } from "./kinds/events";
import type { PodRow } from "./kinds/columns";
import { deriveProblems, HIGH_RESTART_COUNT, type ProblemRow } from "./problems";

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
  it("keeps explicit pod failures but excludes progress, termination, success, and unsupported generic Pending", () => {
    const rows = deriveProblems({
      pods: [
        pod("crashing", "CrashLoopBackOff"),
        pod("failed", "Failed", { phase: "Failed" }),
        pod("creating", "ContainerCreating", { phase: "Pending" }),
        pod("waiting", "Pending", { phase: "Pending" }),
        pod("terminating", "Terminating"),
        pod("healthy", "Running", { ready: "1/1" }),
        pod("completed", "Completed", { phase: "Succeeded", ready: "0/1", restarts: HIGH_RESTART_COUNT }),
        pod("succeeded", "Succeeded", { phase: "Succeeded", ready: "0/1", restarts: HIGH_RESTART_COUNT }),
      ],
      nodes: [],
      events: [],
      now: NOW,
    });

    expect(brief(rows)).toEqual([
      "pod:critical:Pod:checkout/crashing:CrashLoopBackOff",
      "pod:critical:Pod:checkout/failed:Failed",
    ]);
  });

  it("surfaces known waiting-container failures while ordinary startup waits stay excluded", () => {
    const rows = deriveProblems({
      pods: [
        pod("bad-config", "CreateContainerConfigError", {
          phase: "Pending",
          waitingReason: "CreateContainerConfigError",
        }),
        pod("create-error", "CreateContainerError", {
          phase: "Pending",
          waitingReason: "CreateContainerError",
        }),
        pod("pull-error", "ErrImagePull", { phase: "Pending", waitingReason: "ErrImagePull" }),
        pod("bad-image", "InvalidImageName", { phase: "Pending", waitingReason: "InvalidImageName" }),
        pod("pull-backoff", "ImagePullBackOff", { phase: "Pending", waitingReason: "ImagePullBackOff" }),
        pod("creating", "ContainerCreating", { phase: "Pending", waitingReason: "ContainerCreating" }),
        pod("initializing", "PodInitializing", { phase: "Pending", waitingReason: "PodInitializing" }),
      ],
      nodes: [],
      events: [],
      now: NOW,
    });

    expect(brief(rows)).toEqual([
      "pod:critical:Pod:checkout/pull-backoff:ImagePullBackOff",
      "pod:warning:Pod:checkout/bad-config:CreateContainerConfigError",
      "pod:warning:Pod:checkout/bad-image:InvalidImageName",
      "pod:warning:Pod:checkout/create-error:CreateContainerError",
      "pod:warning:Pod:checkout/pull-error:ErrImagePull",
    ]);
  });

  it("calls a Pending pod unschedulable only when a recent FailedScheduling warning names that pod", () => {
    const rows = deriveProblems({
      pods: [
        pod("blocked", "Pending", { phase: "Pending" }),
        pod("old-warning", "Pending", { phase: "Pending" }),
        pod("different-warning", "Pending", { phase: "Pending" }),
      ],
      nodes: [],
      events: [
        warning("a", "Pod/blocked", "FailedScheduling", 2),
        warning("b", "Pod/old-warning", "FailedScheduling", 61),
        warning("c", "Pod/different-warning", "FailedMount", 2),
      ],
      now: NOW,
    });

    expect(brief(rows.filter((row) => row.source === "pod"))).toEqual([
      "pod:warning:Pod:checkout/blocked:Pending,FailedScheduling",
    ]);
  });

  it("surfaces Running pods that are unready or truly high-restart, at a documented threshold", () => {
    const rows = deriveProblems({
      pods: [
        pod("unready", "Running", { ready: "1/2", restarts: 0 }),
        pod("high-restart", "Running", { ready: "1/1", restarts: HIGH_RESTART_COUNT }),
        pod("both", "Running", { ready: "0/1", restarts: HIGH_RESTART_COUNT + 4 }),
        pod("below-threshold", "Running", { ready: "1/1", restarts: HIGH_RESTART_COUNT - 1 }),
      ],
      nodes: [],
      events: [],
      now: NOW,
    });

    expect(brief(rows)).toEqual([
      "pod:warning:Pod:checkout/both:NotReady,HighRestartCount",
      "pod:warning:Pod:checkout/high-restart:HighRestartCount",
      "pod:warning:Pod:checkout/unready:NotReady",
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
        pod("z-unready", "Running", { ready: "0/1" }),
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
      "warning:Pod/z-unready",
      "unknown:Pod/unknown",
    ]);
    expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
  });
});
