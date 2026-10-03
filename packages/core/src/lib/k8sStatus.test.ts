import { describe, it, expect } from "vitest";
import {
  cronJobStatus,
  eventVerdict,
  jobStatus,
  nodeStatus,
  podStatus,
  resourceStatusLine,
  scaledStatus,
  type PodVitals,
} from "./k8sStatus";
import type { K8sObject } from "./manifest";

/** A Deployment-shaped object: `spec.replicas` desired, the rest on `status`. */
const deployment = (spec: Record<string, unknown>, status: Record<string, unknown>): K8sObject => ({
  kind: "Deployment",
  metadata: { name: "checkout-api", namespace: "checkout" },
  spec,
  status,
});

const pod = (status: Record<string, unknown>, spec: Record<string, unknown> = {}): K8sObject => ({
  kind: "Pod",
  metadata: { name: "cart-session-store-0", namespace: "checkout" },
  spec,
  status,
});

/** One container status, as kubelet reports it. */
const container = (name: string, state: Record<string, unknown>, ready: boolean, restartCount = 0) => ({
  name,
  ready,
  restartCount,
  state,
});

describe("resourceStatusLine — Deployment", () => {
  it("reads the mock's frame A: 9 of 12 ready is Degraded, danger-toned, and flagged", () => {
    const line = resourceStatusLine("Deployment", deployment({ replicas: 12 }, { readyReplicas: 9 }));
    expect(line).toEqual({
      status: "Degraded",
      health: "danger",
      readyText: "9/12 ready",
      flagged: true,
    });
  });

  it("calls a fully ready Deployment Running, success-toned, and unflagged", () => {
    expect(resourceStatusLine("Deployment", deployment({ replicas: 3 }, { readyReplicas: 3, availableReplicas: 3 }))).toEqual({
      status: "Running",
      health: "success",
      readyText: "3/3 ready",
      flagged: false,
    });
  });

  it("counts READY replicas, not available ones — the label says ready, so the number must be readyReplicas", () => {
    // The two fields are not the same: `availableReplicas` is the subset of
    // ready replicas that have also outlived `minReadySeconds`, so a healthy
    // rollout sits at ready > available for a while. A line labelled "ready"
    // that printed the available count would under-report during exactly the
    // window a reader is most likely to be watching it.
    const line = resourceStatusLine("Deployment", deployment({ replicas: 12 }, { readyReplicas: 12, availableReplicas: 9 }));
    expect(line?.readyText).toBe("12/12 ready");
    expect(line?.status).toBe("Running");
    expect(line?.flagged).toBe(false);
  });

  it("treats a Deployment scaled to zero as scaled down, not degraded — the list's dot agrees", () => {
    expect(resourceStatusLine("Deployment", deployment({ replicas: 0 }, {}))).toEqual({
      status: "Scaled down",
      health: "neutral",
      readyText: "0/0 ready",
      flagged: false,
    });
  });

  it("reads a missing status as zero ready rather than throwing", () => {
    expect(resourceStatusLine("Deployment", { kind: "Deployment", spec: { replicas: 2 } })).toEqual({
      status: "Degraded",
      health: "danger",
      readyText: "0/2 ready",
      flagged: true,
    });
  });
});

describe("resourceStatusLine — StatefulSet and ReplicaSet", () => {
  it("degrades a StatefulSet short of its desired replicas", () => {
    const sts: K8sObject = { kind: "StatefulSet", spec: { replicas: 3 }, status: { readyReplicas: 1 } };
    expect(resourceStatusLine("StatefulSet", sts)).toEqual({
      status: "Degraded",
      health: "danger",
      readyText: "1/3 ready",
      flagged: true,
    });
  });

  it("passes a fully ready ReplicaSet", () => {
    const rs: K8sObject = { kind: "ReplicaSet", spec: { replicas: 2 }, status: { readyReplicas: 2 } };
    expect(resourceStatusLine("ReplicaSet", rs)).toEqual({
      status: "Running",
      health: "success",
      readyText: "2/2 ready",
      flagged: false,
    });
  });

  it("calls a superseded ReplicaSet (zero desired) scaled down, not degraded", () => {
    const rs: K8sObject = { kind: "ReplicaSet", spec: { replicas: 0 }, status: {} };
    expect(resourceStatusLine("ReplicaSet", rs)?.flagged).toBe(false);
    expect(resourceStatusLine("ReplicaSet", rs)?.status).toBe("Scaled down");
  });
});

describe("resourceStatusLine — DaemonSet", () => {
  it("counts nodes, not replicas: numberReady out of desiredNumberScheduled", () => {
    const ds: K8sObject = {
      kind: "DaemonSet",
      status: { desiredNumberScheduled: 5, currentNumberScheduled: 5, numberReady: 3, numberAvailable: 3 },
    };
    expect(resourceStatusLine("DaemonSet", ds)).toEqual({
      status: "Degraded",
      health: "danger",
      readyText: "3/5 ready",
      flagged: true,
    });
  });

  it("does not flag a DaemonSet that matches no nodes at all", () => {
    const ds: K8sObject = { kind: "DaemonSet", status: { desiredNumberScheduled: 0, numberReady: 0 } };
    expect(resourceStatusLine("DaemonSet", ds)).toEqual({
      status: "Not scheduled",
      health: "neutral",
      readyText: "0/0 ready",
      flagged: false,
    });
  });
});

describe("resourceStatusLine — Pod", () => {
  it("reads the mock's frame B: a Running pod, 1/1 ready, success-toned and unflagged", () => {
    const running = pod({
      phase: "Running",
      containerStatuses: [container("redis", { running: { startedAt: "2026-01-01T00:00:00Z" } }, true)],
    });
    expect(resourceStatusLine("Pod", running)).toEqual({
      status: "Running",
      health: "success",
      readyText: "1/1 ready",
      flagged: false,
    });
  });

  it("flags a Pending pod, warning-toned", () => {
    expect(resourceStatusLine("Pod", pod({ phase: "Pending" }))).toEqual({
      status: "Pending",
      health: "warning",
      readyText: null,
      flagged: true,
    });
  });

  it("does NOT flag a Succeeded pod — a green pill and a red dot on one header is the bug this replaces", () => {
    const succeeded = pod({
      phase: "Succeeded",
      containerStatuses: [container("runner", { terminated: { reason: "Completed", exitCode: 0 } }, false)],
    });
    const line = resourceStatusLine("Pod", succeeded);
    // kubectl's word for a finished pod is its container's: `Completed`.
    expect(line?.status).toBe("Completed");
    expect(line?.health).toBe("success");
    expect(line?.flagged).toBe(false);
  });

  it("flags a Failed pod, danger-toned", () => {
    const failed = pod({
      phase: "Failed",
      containerStatuses: [container("runner", { terminated: { reason: "Error", exitCode: 1 } }, false)],
    });
    expect(resourceStatusLine("Pod", failed)).toEqual({
      status: "Error",
      health: "danger",
      readyText: "0/1 ready",
      flagged: true,
    });
  });

  it("names a container killed for memory, as kubectl does, and tones it danger", () => {
    const oom = pod({
      phase: "Running",
      containerStatuses: [container("api", { terminated: { reason: "OOMKilled", exitCode: 137 } }, false, 3)],
    });
    expect(resourceStatusLine("Pod", oom)).toEqual({
      status: "OOMKilled",
      health: "danger",
      readyText: "0/1 ready",
      flagged: true,
    });
  });

  it("reads a pod still in its init containers as progress, warning-toned", () => {
    const initializing = pod(
      {
        phase: "Pending",
        initContainerStatuses: [container("fetch", { running: {} }, false)],
        containerStatuses: [container("app", { waiting: { reason: "PodInitializing" } }, false)],
      },
      { initContainers: [{ name: "fetch" }, { name: "migrate" }] },
    );
    expect(resourceStatusLine("Pod", initializing)).toEqual({
      status: "Init:0/2",
      health: "warning",
      readyText: "0/1 ready",
      flagged: true,
    });
  });

  it("reads a pod being deleted as Terminating, warning-toned", () => {
    const deleting: K8sObject = {
      ...pod({ phase: "Running", containerStatuses: [container("api", { running: {} }, true)] }),
      metadata: { name: "cart-session-store-0", namespace: "checkout", deletionTimestamp: "2026-10-03T10:05:00Z" },
    };
    expect(resourceStatusLine("Pod", deleting)).toEqual({
      status: "Terminating",
      health: "warning",
      readyText: "1/1 ready",
      flagged: true,
    });
  });

  it("shows the waiting reason, not the phase, for a crash-looping pod — and tones it danger", () => {
    const crashing = pod({
      phase: "Running",
      containerStatuses: [
        container("api", { waiting: { reason: "CrashLoopBackOff", message: "back-off 5m0s" } }, false),
      ],
    });
    expect(resourceStatusLine("Pod", crashing)).toEqual({
      status: "CrashLoopBackOff",
      health: "danger",
      readyText: "0/1 ready",
      flagged: true,
    });
  });

  it("shows a non-backoff waiting reason as a warning, not a failure", () => {
    const starting = pod({
      phase: "Pending",
      containerStatuses: [container("api", { waiting: { reason: "ContainerCreating" } }, false)],
    });
    expect(resourceStatusLine("Pod", starting)).toEqual({
      status: "ContainerCreating",
      health: "warning",
      readyText: "0/1 ready",
      flagged: true,
    });
  });

  it("never flags a finished pod, whatever its containers report", () => {
    // A Succeeded pod's containers are terminated; a stray waiting entry must
    // not re-earn a finished pod a dot. The word is still kubectl's, which
    // reads the containers whatever the phase.
    const done = pod({
      phase: "Succeeded",
      containerStatuses: [container("api", { waiting: { reason: "CrashLoopBackOff" } }, false)],
    });
    const line = resourceStatusLine("Pod", done);
    expect(line?.status).toBe("CrashLoopBackOff");
    expect(line?.health).toBe("success");
    expect(line?.flagged).toBe(false);
  });

  it("counts the ready containers across a multi-container pod", () => {
    const sidecar = pod({
      phase: "Running",
      containerStatuses: [
        container("api", { running: {} }, true),
        container("envoy", { running: {} }, false),
      ],
    });
    expect(resourceStatusLine("Pod", sidecar)?.readyText).toBe("1/2 ready");
  });

  it("offers no ratio for a pod the kubelet has not reported containers for yet", () => {
    expect(resourceStatusLine("Pod", pod({ phase: "Pending" }))?.readyText).toBeNull();
  });

  it("calls a pod with no phase at all Unknown, and flags it", () => {
    expect(resourceStatusLine("Pod", pod({}))).toEqual({
      status: "Unknown",
      health: "danger",
      readyText: null,
      flagged: true,
    });
  });
});

describe("resourceStatusLine — Job and CronJob", () => {
  const job = (spec: Record<string, unknown>, status: Record<string, unknown>): K8sObject => ({
    kind: "Job",
    spec,
    status,
  });

  it("calls a completed Job Complete, success-toned and unflagged", () => {
    expect(resourceStatusLine("Job", job({ completions: 3 }, { succeeded: 3 }))).toEqual({
      status: "Complete",
      health: "success",
      readyText: "3/3 complete",
      flagged: false,
    });
  });

  it("flags a Job with a failed pod, danger-toned — the list's own rule", () => {
    expect(resourceStatusLine("Job", job({ completions: 1 }, { failed: 2, succeeded: 0 }))).toEqual({
      status: "Failed",
      health: "danger",
      readyText: "0/1 complete",
      flagged: true,
    });
  });

  it("does not flag a Job that is merely still running, though it tones it warning", () => {
    // Matches `jobFlagged` in the list exactly: only a failure earns the dot,
    // even though an in-flight Job's pill is amber.
    expect(resourceStatusLine("Job", job({}, { active: 1 }))).toEqual({
      status: "Active",
      health: "warning",
      readyText: "0/1 complete",
      flagged: false,
    });
  });

  it("defaults an unset completions count to one", () => {
    expect(resourceStatusLine("Job", job({}, { succeeded: 1 }))?.readyText).toBe("1/1 complete");
  });

  it("reads a CronJob's suspension, and gives it no ratio", () => {
    const suspended: K8sObject = { kind: "CronJob", spec: { suspend: true }, status: {} };
    expect(resourceStatusLine("CronJob", suspended)).toEqual({
      status: "Suspended",
      health: "neutral",
      readyText: null,
      flagged: false,
    });
  });

  it("calls an unsuspended CronJob Active, and never flags one", () => {
    const active: K8sObject = { kind: "CronJob", spec: {}, status: { active: [{ name: "run-1" }] } };
    expect(resourceStatusLine("CronJob", active)).toEqual({
      status: "Active",
      health: "success",
      readyText: null,
      flagged: false,
    });
  });
});

describe("resourceStatusLine — Node", () => {
  const node = (conditions: unknown[], spec: Record<string, unknown> = {}): K8sObject => ({
    kind: "Node",
    metadata: { name: "eu-w4-n2-standard-b5" },
    spec,
    status: { conditions },
  });

  it("reads readiness off the Ready condition", () => {
    expect(resourceStatusLine("Node", node([{ type: "Ready", status: "True" }]))).toEqual({
      status: "Ready",
      health: "success",
      readyText: null,
      flagged: false,
    });
  });

  it("flags a NotReady node, danger-toned", () => {
    expect(resourceStatusLine("Node", node([{ type: "MemoryPressure", status: "False" }, { type: "Ready", status: "False" }]))).toEqual({
      status: "NotReady",
      health: "danger",
      readyText: null,
      flagged: true,
    });
  });

  it("names a cordoned node the way kubectl does, and flags it warning — the list already badges it", () => {
    const cordoned = node([{ type: "Ready", status: "True" }], { unschedulable: true });
    expect(resourceStatusLine("Node", cordoned)).toEqual({
      status: "Ready,SchedulingDisabled",
      health: "warning",
      readyText: null,
      flagged: true,
    });
  });

  it("keeps danger over warning for a node that is both NotReady and cordoned", () => {
    const both = node([{ type: "Ready", status: "False" }], { unschedulable: true });
    expect(resourceStatusLine("Node", both)?.status).toBe("NotReady,SchedulingDisabled");
    expect(resourceStatusLine("Node", both)?.health).toBe("danger");
  });

  it("calls a node with no Ready condition Unknown", () => {
    expect(resourceStatusLine("Node", node([]))?.status).toBe("Unknown");
    expect(resourceStatusLine("Node", node([]))?.health).toBe("danger");
  });
});

describe("resourceStatusLine — kinds with no status line", () => {
  it("returns null for a kind that has no health of its own", () => {
    expect(resourceStatusLine("ConfigMap", { kind: "ConfigMap", metadata: { name: "app-config" } })).toBeNull();
    expect(resourceStatusLine("Service", { kind: "Service" })).toBeNull();
    expect(resourceStatusLine("Secret", { kind: "Secret" })).toBeNull();
  });

  it("returns null for a custom resource, rather than guessing at its status", () => {
    const cr: K8sObject = {
      apiVersion: "argoproj.io/v1alpha1",
      kind: "Rollout",
      status: { readyReplicas: 1, phase: "Degraded" },
    };
    expect(resourceStatusLine("Rollout", cr)).toBeNull();
  });

  it("returns null for an empty kind, and never throws on an empty object", () => {
    expect(resourceStatusLine("", {})).toBeNull();
    expect(() => resourceStatusLine("Pod", {})).not.toThrow();
    expect(() => resourceStatusLine("Deployment", {})).not.toThrow();
  });
});

/**
 * One pod row's vitals — the fields `PodSummary` carries that `podStatus`
 * reads — with no waiting reason unless a test says so.
 */
const vitals = (over: PodVitals): PodVitals => ({ waitingReason: "", ...over });

describe("podStatus — the one reading a list row and a fetched object share", () => {
  it("gives a crash-looping pod the same verdict the header derives from the object", () => {
    // The whole point of the shared function: `PodSummary` carries the phase,
    // the waiting reason and kubectl's word, `K8sObject` carries the
    // container statuses those were summarised from, and both arrive here.
    const backingOff = vitals({ phase: "Running", waitingReason: "CrashLoopBackOff" });
    expect(podStatus(backingOff)).toEqual({
      status: "CrashLoopBackOff",
      health: "danger",
      flagged: true,
    });
    const object: K8sObject = {
      kind: "Pod",
      status: {
        phase: "Running",
        containerStatuses: [
          { name: "api", ready: false, restartCount: 1123, state: { waiting: { reason: "CrashLoopBackOff" } } },
        ],
      },
    };
    const line = resourceStatusLine("Pod", object)!;
    const { readyText, ...verdict } = line;
    expect(verdict).toEqual(podStatus(backingOff));
    expect(readyText).toBe("0/1 ready");
  });

  it("warns rather than fails for a pod still pulling or creating", () => {
    expect(podStatus(vitals({ phase: "Pending", waitingReason: "ContainerCreating" }))).toEqual({
      status: "ContainerCreating",
      health: "warning",
      flagged: true,
    });
    expect(podStatus(vitals({ phase: "Pending", waitingReason: "ImagePullBackOff" }))).toEqual({
      status: "ImagePullBackOff",
      health: "danger",
      flagged: true,
    });
  });

  it("falls back to the phase when no container is waiting and every one is ready", () => {
    expect(podStatus(vitals({ phase: "Running" }))).toEqual({ status: "Running", health: "success", flagged: false });
  });

  it("keeps a finished pod finished, whatever a stale waiting entry says", () => {
    expect(podStatus(vitals({ phase: "Succeeded", waitingReason: "CrashLoopBackOff" }))).toEqual({
      status: "Succeeded",
      health: "success",
      flagged: false,
    });
    expect(podStatus(vitals({ phase: "Failed", waitingReason: "CrashLoopBackOff" }))).toEqual({
      status: "Failed",
      health: "danger",
      flagged: true,
    });
  });

  it("calls an empty phase Unknown rather than rendering a blank pill", () => {
    expect(podStatus(vitals({ phase: "" }))).toEqual({ status: "Unknown", health: "danger", flagged: true });
  });

  it("flags a phase word it does not recognise without inventing a colour for it", () => {
    // `podFlagged`'s rule verbatim: anything the phase table does not call
    // healthy earns the dot. The tone stays neutral because nothing has told
    // us it is red. (Not `Evicted`: that is a word kubectl uses, and it is red.)
    expect(podStatus(vitals({ phase: "Mystery" }))).toEqual({ status: "Mystery", health: "neutral", flagged: true });
  });
});

/**
 * The word `kubectl get pods` prints — `PodSummary.status` on a row — read
 * the way the spec's table reads it, first rule that matches.
 */
describe("podStatus — kubectl's words, toned", () => {
  const RED = { health: "danger", flagged: true } as const;
  const AMBER = { health: "warning", flagged: true } as const;
  const GREEN = { health: "success", flagged: false } as const;
  const read = (phase: string, status: string, waitingReason = "") => podStatus({ phase, waitingReason, status });

  it("shows kubectl's word over the waiting reason and the phase", () => {
    expect(read("Running", "OOMKilled")).toEqual({ status: "OOMKilled", ...RED });
  });

  it("keeps a finished pod's colours, whatever word kubectl gives it", () => {
    expect(read("Succeeded", "Completed")).toEqual({ status: "Completed", ...GREEN });
    for (const word of ["Evicted", "Error", "OOMKilled"]) {
      expect(read("Failed", word)).toEqual({ status: word, ...RED });
    }
  });

  it("tones a waiting container's word by the back-off rule", () => {
    expect(read("Running", "CrashLoopBackOff", "CrashLoopBackOff")).toEqual({ status: "CrashLoopBackOff", ...RED });
    for (const word of ["CreateContainerConfigError", "ErrImagePull"]) {
      expect(read("Pending", word, word)).toEqual({ status: word, ...AMBER });
    }
  });

  it("reads an init container's word by what follows the prefix", () => {
    expect(read("Pending", "Init:0/2")).toEqual({ status: "Init:0/2", ...AMBER });
    for (const word of ["Init:Error", "Init:ExitCode:2", "Init:OOMKilled", "Init:CrashLoopBackOff"]) {
      expect(read("Pending", word)).toEqual({ status: word, ...RED });
    }
    expect(read("Pending", "Init:ErrImagePull")).toEqual({ status: "Init:ErrImagePull", ...AMBER });
  });

  it("calls a terminated container's failure red on a pod still Running", () => {
    for (const word of [
      "Error",
      "OOMKilled",
      "ExitCode:1",
      "Signal:9",
      "ContainerCannotRun",
      "DeadlineExceeded",
      "ContainerStatusUnknown",
      "StartError",
    ]) {
      expect(read("Running", word)).toEqual({ status: word, ...RED });
    }
  });

  it("calls a pod on its way somewhere amber", () => {
    for (const word of ["Pending", "PodInitializing", "SchedulingGated", "Terminating", "ContainerCreating"]) {
      expect(read("Pending", word)).toEqual({ status: word, ...AMBER });
    }
    // A container that exited 0 under `restartPolicy: Always` is about to
    // be restarted, not finished.
    expect(read("Running", "Completed")).toEqual({ status: "Completed", ...AMBER });
  });

  it("keeps the phase table's red words red", () => {
    for (const word of ["Unknown", "NotReady", "Failed"]) {
      expect(read("Running", word)).toEqual({ status: word, ...RED });
    }
  });

  it("calls kubectl's Running green", () => {
    expect(read("Running", "Running")).toEqual({ status: "Running", ...GREEN });
  });

  it("flags a word it does not know without inventing a colour for it", () => {
    expect(read("Running", "Mystery")).toEqual({ status: "Mystery", health: "neutral", flagged: true });
  });
});

/**
 * A crash-looping pod, at each moment a watch can catch it.
 *
 * Written against a real pod: `legacy-adapter-857bf965b8-4wclg` in `payments`
 * on `kind-srelens-demo`, restart count 1123, back-off 5m0s. A
 * `kubectl get -w` on it publishes these shapes, and kubectl prints a word for
 * each one:
 *
 *     state=waiting(CrashLoopBackOff)           → CrashLoopBackOff
 *     state=terminated(Error), exit code 1       → Error
 *     state=running, not ready (between restarts) → Running
 *
 * The row carries kubectl's word as `status`, and `podStatus` tones it. The
 * first two are red. The third is kubectl's `Running`, green and unflagged, by
 * decision: the desktop matches kubectl exactly. That decision gave up an
 * earlier `NotReady` rule, derived from the ready ratio and the restart count,
 * which held the pod in the Overview's unhealthy list through that moment.
 */
describe("podStatus — a crash-looper reads kubectl's word at every moment", () => {
  const BACKING_OFF = vitals({ phase: "Running", waitingReason: "CrashLoopBackOff", status: "CrashLoopBackOff" });
  const EXITED = vitals({ phase: "Running", status: "Error" });
  const UP_BETWEEN_RESTARTS = vitals({ phase: "Running", status: "Running" });

  it("is red while its container is down, waiting or exited", () => {
    expect(podStatus(BACKING_OFF)).toEqual({ status: "CrashLoopBackOff", health: "danger", flagged: true });
    expect(podStatus(EXITED)).toEqual({ status: "Error", health: "danger", flagged: true });
  });

  it("reads Running, green, while its container is up between restarts, as kubectl does", () => {
    expect(podStatus(UP_BETWEEN_RESTARTS)).toEqual({ status: "Running", health: "success", flagged: false });
  });

  /** The live pod at each moment, as the detail header fetches it. */
  const live = (state: Record<string, unknown>): K8sObject => ({
    kind: "Pod",
    status: {
      phase: "Running",
      containerStatuses: [{ name: "adapter", ready: false, restartCount: 1123, state }],
    },
  });

  it("gives the header the same verdict as the row, at every moment", () => {
    // The two readings that must never disagree: the pods list row and the
    // detail header, on the same pod at the same moment.
    const verdictOf = (object: K8sObject) => {
      const { readyText, ...verdict } = resourceStatusLine("Pod", object)!;
      expect(readyText).toBe("0/1 ready");
      return verdict;
    };
    expect(verdictOf(live({ waiting: { reason: "CrashLoopBackOff" } }))).toEqual(podStatus(BACKING_OFF));
    expect(verdictOf(live({ terminated: { exitCode: 1, reason: "Error", finishedAt: "2026-08-24T13:28:18Z" } }))).toEqual(
      podStatus(EXITED),
    );
    expect(verdictOf(live({ running: { startedAt: "2026-08-24T13:28:18Z" } }))).toEqual(
      podStatus(UP_BETWEEN_RESTARTS),
    );
  });

  it("reads a sidecar pod with one unready container as kubectl does: Running", () => {
    const object: K8sObject = {
      kind: "Pod",
      status: {
        phase: "Running",
        containerStatuses: [
          { name: "app", ready: true, restartCount: 0, state: { running: {} } },
          { name: "envoy", ready: false, restartCount: 12, state: { running: {} } },
        ],
      },
    };
    expect(resourceStatusLine("Pod", object)).toEqual({
      status: "Running",
      health: "success",
      flagged: false,
      readyText: "1/2 ready",
    });
  });

  it("leaves a pod the kubelet has not reported containers for alone", () => {
    // No container statuses at all: no ratio to show, and nothing that says
    // the pod is unready — only that nobody has looked yet.
    const object: K8sObject = { kind: "Pod", status: { phase: "Running" } };
    expect(resourceStatusLine("Pod", object)).toEqual({
      status: "Running",
      health: "success",
      flagged: false,
      readyText: null,
    });
  });
});

/**
 * The six legal (tone, dot) pairs, keyed by what `k8sStatus` calls them. Any
 * pair outside this table is a hand-rolled one — the class of mistake that put
 * a green pill and a red dot on one `Succeeded` pod.
 */
const LEGAL_VERDICTS: Record<string, string> = {
  "success/false": "WELL",
  "neutral/false": "AT_REST",
  "warning/false": "IN_FLIGHT",
  "warning/true": "UNSETTLED",
  "danger/true": "BROKEN",
  "neutral/true": "UNREADABLE",
};

describe("the tone and the dot are paired structurally", () => {
  it("pairs every kind and state as one of the six verdicts, and reaches all six", () => {
    const objects: [string, K8sObject][] = [
      ["Pod", pod({ phase: "Running", containerStatuses: [container("a", { running: {} }, true)] })],
      ["Pod", pod({ phase: "Succeeded" })],
      ["Pod", pod({ phase: "Pending" })],
      ["Pod", pod({ phase: "Failed" })],
      // The waiting branch, which the phase alone never reaches: a pod stuck
      // in a back-off, and one merely on its way up.
      ["Pod", pod({ phase: "Running", containerStatuses: [container("a", { waiting: { reason: "CrashLoopBackOff" } }, false)] })],
      ["Pod", pod({ phase: "Pending", containerStatuses: [container("a", { waiting: { reason: "ContainerCreating" } }, false)] })],
      // A crash-looper caught between restarts, up and not ready: kubectl's
      // `Running`, which is green.
      ["Pod", pod({ phase: "Running", containerStatuses: [container("a", { running: {} }, false, 1123)] })],
      // A word nothing knows, neither kubectl's table nor the phase table —
      // the only producer of UNREADABLE.
      ["Pod", pod({ phase: "Mystery" })],
      // kubectl's words that no phase carries: a container killed for
      // memory, and a pod still in its init containers.
      ["Pod", pod({ phase: "Running", containerStatuses: [container("a", { terminated: { reason: "OOMKilled", exitCode: 137 } }, false)] })],
      ["Pod", pod({ phase: "Pending", initContainerStatuses: [container("i", { running: {} }, false)] }, { initContainers: [{ name: "i" }] })],
      ["Deployment", deployment({ replicas: 3 }, { readyReplicas: 3 })],
      ["Deployment", deployment({ replicas: 3 }, { readyReplicas: 1 })],
      ["Deployment", deployment({ replicas: 0 }, {})],
      ["StatefulSet", { kind: "StatefulSet", spec: { replicas: 1 }, status: { readyReplicas: 1 } }],
      ["ReplicaSet", { kind: "ReplicaSet", spec: { replicas: 0 }, status: {} }],
      ["DaemonSet", { kind: "DaemonSet", status: { desiredNumberScheduled: 2, numberReady: 2 } }],
      ["DaemonSet", { kind: "DaemonSet", status: { desiredNumberScheduled: 2, numberReady: 0 } }],
      ["Job", { kind: "Job", spec: {}, status: { succeeded: 1 } }],
      ["Job", { kind: "Job", spec: {}, status: { active: 1 } }],
      ["Job", { kind: "Job", spec: {}, status: { failed: 1 } }],
      ["CronJob", { kind: "CronJob", spec: { suspend: true }, status: {} }],
      ["CronJob", { kind: "CronJob", spec: {}, status: {} }],
      ["Node", { kind: "Node", spec: {}, status: { conditions: [{ type: "Ready", status: "True" }] } }],
      ["Node", { kind: "Node", spec: { unschedulable: true }, status: { conditions: [{ type: "Ready", status: "True" }] } }],
      // Cordoned AND NotReady: the branch that has to keep the worse verdict.
      ["Node", { kind: "Node", spec: { unschedulable: true }, status: { conditions: [{ type: "Ready", status: "False" }] } }],
      ["Node", { kind: "Node", spec: {}, status: { conditions: [] } }],
    ];

    const reached = new Set<string>();
    for (const [kind, object] of objects) {
      const line = resourceStatusLine(kind, object)!;
      expect(line).not.toBeNull();
      const subject = `${kind} · ${line.status}`;
      const verdict = LEGAL_VERDICTS[`${line.health}/${line.flagged}`];
      // `expect.any(String)` rather than `toBeDefined`: an illegal pair fails
      // with the kind and the word that produced it, not just `undefined`.
      expect({ subject, verdict }).toEqual({ subject, verdict: expect.any(String) });
      reached.add(verdict);
      // The two directions of the original rule, kept explicit: a healthy
      // tone never earns a dot, and a failing one always does.
      if (line.health === "success") expect({ subject, flagged: line.flagged }).toEqual({ subject, flagged: false });
      if (line.health === "danger") expect({ subject, flagged: line.flagged }).toEqual({ subject, flagged: true });
    }

    // `eventVerdict` shares this same table — the sweep this file uses to
    // catch a producer pairing a healthy tone with a flagged word applies to
    // it too, not just to `resourceStatusLine`'s producers. `bad` stands in
    // for `flagged` here: an event has no status word to hang a dot off, so
    // it names only whether the word itself is worth colouring.
    for (const type of ["Warning", "Normal", "", "Something"]) {
      const { health, bad } = eventVerdict(type);
      const subject = `Event · ${type || "(empty)"}`;
      const verdict = LEGAL_VERDICTS[`${health}/${bad}`];
      expect({ subject, verdict }).toEqual({ subject, verdict: expect.any(String) });
      reached.add(verdict);
      // No `health === "success"` arm here, unlike the loop above: unlike
      // `HealthKind`, `eventVerdict`'s return type has no "success" member at
      // all, so that comparison is a compile error (TS2367) rather than a
      // runtime-only guarantee — the type itself is the proof.
      if (health === "danger") expect({ subject, bad }).toEqual({ subject, bad: true });
    }

    // The sweep is only worth its prose if it actually walks every branch:
    // without this, a verdict could stop being produced by anything and no
    // test would notice.
    expect([...reached].sort()).toEqual([...new Set(Object.values(LEGAL_VERDICTS))].sort());
  });
});

describe("eventVerdict", () => {
  it("colours a Warning and leaves a Normal plain", () => {
    expect(eventVerdict("Warning")).toEqual({ health: "danger", bad: true });
    expect(eventVerdict("Normal")).toEqual({ health: "neutral", bad: false });
  });

  it("reads an unknown type as unremarkable rather than alarming", () => {
    expect(eventVerdict("")).toEqual({ health: "neutral", bad: false });
    expect(eventVerdict("Something")).toEqual({ health: "neutral", bad: false });
  });

  it("makes the illegal pair a compile error, not merely an untested one", () => {
    // A green tone paired with a flagged word — the exact `Succeeded`-pod bug
    // `Verdict` exists to prevent — must not typecheck against `eventVerdict`'s
    // return type. `@ts-expect-error` fails `tsc --noEmit` if this line ever
    // STOPS erroring (an "unused directive" error), so a future widening of
    // the return type back to `{ health: HealthKind; bad: boolean }` fails the
    // typecheck gate, not just this assertion.
    type EV = ReturnType<typeof eventVerdict>;
    // @ts-expect-error — "success" is not one of eventVerdict's two health
    // words, and pairing it with `bad: true` is unrepresentable by design.
    const illegal: EV = { health: "success", bad: true };
    expect(illegal).toEqual({ health: "success", bad: true });
  });
});

/**
 * The row-facing half of the same verdicts. A list row carries counts and no
 * object; a detail header carries an object and no row. Both have to say the
 * same thing about one workload, so both go through one function and these
 * tests assert exactly that — the verdict, and its identity with the line the
 * object path produces. (#331)
 */
describe("the row reading and the object reading are one verdict", () => {
  /** Everything but the ready phrase, which only a header shows. */
  const asVerdict = ({ status, health, flagged }: { status: string; health: string; flagged: boolean }) => ({
    status,
    health,
    flagged,
  });

  it("scaledStatus reads a Deployment the way its own header does", () => {
    for (const [ready, desired] of [[9, 12], [3, 3], [0, 0], [0, 5]] as const) {
      expect(scaledStatus("Deployment", ready, desired)).toEqual(
        asVerdict(resourceStatusLine("Deployment", deployment({ replicas: desired }, { readyReplicas: ready }))!),
      );
    }
  });

  it("gives a DaemonSet matching no node its own zero word, not the replica one", () => {
    expect(scaledStatus("DaemonSet", 0, 0)).toEqual({
      status: "Not scheduled",
      health: "neutral",
      flagged: false,
    });
    expect(scaledStatus("Deployment", 0, 0).status).toBe("Scaled down");
    expect(scaledStatus("StatefulSet", 0, 0).status).toBe("Scaled down");
  });

  it("jobStatus reads a Job the way its own header does", () => {
    const job = (status: Record<string, unknown>): K8sObject => ({
      kind: "Job",
      metadata: { name: "nightly", namespace: "batch" },
      spec: { completions: 1 },
      status,
    });
    for (const [failed, active] of [[1, 0], [0, 2], [0, 0], [2, 3]] as const) {
      expect(jobStatus(failed, active)).toEqual(
        asVerdict(resourceStatusLine("Job", job({ failed, active, succeeded: 0 }))!),
      );
    }
    // A failure outranks an in-flight pod: a Job with both is Failed, not Active.
    expect(jobStatus(2, 3).status).toBe("Failed");
  });

  it("cronJobStatus reads a CronJob the way its own header does", () => {
    for (const suspend of [true, false]) {
      expect(cronJobStatus(suspend)).toEqual(
        asVerdict(
          resourceStatusLine("CronJob", {
            kind: "CronJob",
            metadata: { name: "nightly", namespace: "batch" },
            spec: { suspend },
            status: {},
          })!,
        ),
      );
    }
  });

  it("nodeStatus reads a Node the way its own header does, cordoned or not", () => {
    const node = (ready: string, unschedulable: boolean): K8sObject => ({
      kind: "Node",
      metadata: { name: "node-a" },
      spec: unschedulable ? { unschedulable: true } : {},
      status: { conditions: [{ type: "Ready", status: ready }] },
    });
    for (const [word, ready] of [["Ready", "True"], ["NotReady", "False"]] as const) {
      for (const unschedulable of [false, true]) {
        expect(nodeStatus(word, unschedulable)).toEqual(asVerdict(resourceStatusLine("Node", node(ready, unschedulable))!));
      }
    }
    // Both readings a list row can hold are flagged, which is what the row
    // chip's own question turns on: a NotReady node asked from the row and
    // from the pane must send the same question.
    expect(nodeStatus("NotReady", false).flagged).toBe(true);
    expect(nodeStatus("Ready", true).flagged).toBe(true);
    expect(nodeStatus("Ready", false).flagged).toBe(false);
  });
});
