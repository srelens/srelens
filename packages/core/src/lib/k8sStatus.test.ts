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
    expect(line?.status).toBe("Succeeded");
    expect(line?.health).toBe("success");
    expect(line?.flagged).toBe(false);
  });

  it("flags a Failed pod, danger-toned", () => {
    const failed = pod({
      phase: "Failed",
      containerStatuses: [container("runner", { terminated: { reason: "Error", exitCode: 1 } }, false)],
    });
    expect(resourceStatusLine("Pod", failed)).toEqual({
      status: "Failed",
      health: "danger",
      readyText: "0/1 ready",
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

  it("ignores a waiting container once the pod has reached a terminal phase", () => {
    // A Succeeded pod's containers are terminated; a stray waiting entry must
    // not drag a finished pod back to "not ready" and re-earn it a dot.
    const done = pod({
      phase: "Succeeded",
      containerStatuses: [container("api", { waiting: { reason: "CrashLoopBackOff" } }, false)],
    });
    expect(resourceStatusLine("Pod", done)?.status).toBe("Succeeded");
    expect(resourceStatusLine("Pod", done)?.flagged).toBe(false);
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
 * One pod row's vitals — the four fields `PodSummary` carries, defaulted to a
 * plain healthy pod so each test states only the facts it is about.
 *
 * The defaults are the healthy ones on purpose: a test that means "not ready"
 * has to SAY "0/1", so no assertion below passes because a fixture happened to
 * agree with it.
 */
const vitals = (over: Partial<PodVitals> & { phase: string }): PodVitals => ({
  waitingReason: "",
  ready: "1/1",
  restarts: 0,
  ...over,
});

describe("podStatus — the one reading a list row and a fetched object share", () => {
  it("gives a crash-looping pod the same verdict the header derives from the object", () => {
    // The whole point of the shared function: `PodSummary` carries the phase,
    // the waiting reason and the ready count, `K8sObject` carries the
    // container statuses those were summarised from, and both arrive here.
    const backingOff = vitals({
      phase: "Running",
      waitingReason: "CrashLoopBackOff",
      ready: "0/1",
      restarts: 1123,
    });
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
    expect(podStatus(vitals({ phase: "Pending", waitingReason: "ContainerCreating", ready: "0/1" }))).toEqual({
      status: "ContainerCreating",
      health: "warning",
      flagged: true,
    });
    expect(podStatus(vitals({ phase: "Pending", waitingReason: "ImagePullBackOff", ready: "0/1" }))).toEqual({
      status: "ImagePullBackOff",
      health: "danger",
      flagged: true,
    });
  });

  it("falls back to the phase when no container is waiting and every one is ready", () => {
    expect(podStatus(vitals({ phase: "Running" }))).toEqual({ status: "Running", health: "success", flagged: false });
  });

  it("keeps a finished pod finished, whatever a stale waiting entry says", () => {
    expect(podStatus(vitals({ phase: "Succeeded", waitingReason: "CrashLoopBackOff", ready: "0/1" }))).toEqual({
      status: "Succeeded",
      health: "success",
      flagged: false,
    });
    expect(podStatus(vitals({ phase: "Failed", waitingReason: "CrashLoopBackOff", ready: "0/1" }))).toEqual({
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
    // us it is red.
    expect(podStatus(vitals({ phase: "Evicted" }))).toEqual({ status: "Evicted", health: "neutral", flagged: true });
  });
});

/**
 * The flicker, and the property that ends it.
 *
 * Written against a real pod: `legacy-adapter-857bf965b8-4wclg` in `payments`
 * on `kind-srelens-demo`, restart count 1123, back-off 5m0s. A `kubectl get -w`
 * on it publishes exactly three shapes, and they are worth reading rather than
 * reasoning about, because the middle one is not the shape the docs suggest:
 *
 *     phase=Running ready=false restarts=1125 state=waiting(CrashLoopBackOff)
 *     phase=Running ready=false restarts=1126 state=terminated(Error)
 *     phase=Running ready=false restarts=1126 state=waiting(CrashLoopBackOff)
 *
 * The middle event is the flicker window. The container is NOT waiting there,
 * so `waitingReason` is `""` and the phase is still `Running` — and the old
 * rule, with nothing else to read, called the pod well. (Polling for it does
 * not work: at 0.25s intervals, 1400 samples never caught it. The window is
 * one status publish wide, which is why it took a watch.)
 *
 * Note the two columns that do NOT move across all three events: `ready` is
 * `false` in every one, and `restartCount` never goes down. That pair is the
 * signal, and the fixtures below are built out of it. The container state is
 * deliberately NOT: a pod caught mid-restart may be `terminated` (as here) or
 * `running` (as a slower container would be), and a rule keyed on which one
 * would have swapped one flicker for another.
 */
describe("podStatus — a persistently unready pod does not flicker out of the list", () => {
  /** The same pod, at the two moments a poll can catch it. */
  const BACKING_OFF = vitals({
    phase: "Running",
    waitingReason: "CrashLoopBackOff",
    ready: "0/1",
    restarts: 1123,
  });
  const BETWEEN_RESTARTS = vitals({ phase: "Running", waitingReason: "", ready: "0/1", restarts: 1123 });

  it("condemns the pod at BOTH moments, on the same tone and the same dot", () => {
    const backingOff = podStatus(BACKING_OFF);
    const between = podStatus(BETWEEN_RESTARTS);
    // Membership of every unhealthy list, and the tone that list counts by —
    // `Overview`'s `worst()` reads `health`, so a tone that moved would
    // flicker the tile's colour even with the row still present.
    expect({ flagged: between.flagged, health: between.health }).toEqual({ flagged: true, health: "danger" });
    expect({ flagged: backingOff.flagged, health: backingOff.health }).toEqual({ flagged: true, health: "danger" });
  });

  it("says NotReady in the moment there is no waiting reason to name", () => {
    // The word is the one fact that legitimately differs: between restarts the
    // kubelet is reporting no reason, so there is none to show. `NotReady` is
    // the ready ratio read aloud, and it goes through `phaseKind` — which
    // already tones that exact word danger for a Node — rather than pairing a
    // word with a colour here.
    expect(podStatus(BETWEEN_RESTARTS).status).toBe("NotReady");
    expect(podStatus(BACKING_OFF).status).toBe("CrashLoopBackOff");
  });

  it("leaves a pod that is legitimately starting alone", () => {
    // A container up for two seconds and not yet past its readiness probe is
    // normal, and is NOT the same thing as one that is failing. The restart
    // count is what separates them, and it is the only in-snapshot evidence
    // there is: a row carries no clock, so "not ready yet" and "not ready for
    // an hour" are the same row. A pod that has never restarted has not
    // failed at anything.
    expect(podStatus(vitals({ phase: "Running", ready: "0/1", restarts: 0 }))).toEqual({
      status: "Running",
      health: "success",
      flagged: false,
    });
    expect(podStatus(vitals({ phase: "Running", ready: "1/2", restarts: 0 }))).toEqual({
      status: "Running",
      health: "success",
      flagged: false,
    });
  });

  it("leaves a pod that has restarted but recovered alone", () => {
    // Restarts alone condemn nothing: the pod is ready NOW, which is the
    // question. Only the pair — restarted, and still not ready — is a crash
    // loop caught mid-breath.
    expect(podStatus(vitals({ phase: "Running", ready: "1/1", restarts: 1123 }))).toEqual({
      status: "Running",
      health: "success",
      flagged: false,
    });
    expect(podStatus(vitals({ phase: "Running", ready: "2/2", restarts: 4 }))).toEqual({
      status: "Running",
      health: "success",
      flagged: false,
    });
  });

  it("does NOT resurrect a Succeeded pod, however unready and however restarted", () => {
    // The old bug this file was refactored around: a `Succeeded` pod with a
    // green pill and a red dot. Its containers are TERMINATED, so its ready
    // ratio is `0/1` forever — reading that ratio without stopping at the
    // terminal phases would flag every finished pod in the cluster, which on
    // the demo cluster is 53 of them.
    expect(podStatus(vitals({ phase: "Succeeded", ready: "0/1", restarts: 0 }))).toEqual({
      status: "Succeeded",
      health: "success",
      flagged: false,
    });
    expect(podStatus(vitals({ phase: "Succeeded", ready: "0/3", restarts: 7 }))).toEqual({
      status: "Succeeded",
      health: "success",
      flagged: false,
    });
  });

  it("does not re-word a Failed pod, which already has a word of its own", () => {
    expect(podStatus(vitals({ phase: "Failed", ready: "0/1", restarts: 9 }))).toEqual({
      status: "Failed",
      health: "danger",
      flagged: true,
    });
  });

  it("keeps a Pending pod's own word rather than overriding it with the ratio", () => {
    // Pending is already amber and already flagged; the ratio adds nothing
    // and `NotReady` would lose the reader the fact that it is unscheduled.
    expect(podStatus(vitals({ phase: "Pending", ready: "0/1", restarts: 2 }))).toEqual({
      status: "Pending",
      health: "warning",
      flagged: true,
    });
  });

  it("keeps an unrecognised phase word, which the ratio has no standing to overrule", () => {
    expect(podStatus(vitals({ phase: "Evicted", ready: "0/1", restarts: 3 }))).toEqual({
      status: "Evicted",
      health: "neutral",
      flagged: true,
    });
  });

  it("reads a ratio it cannot parse as no reading, never as unready", () => {
    // "0/0" is what the backend sends for a pod the kubelet has reported no
    // containers for, and an empty or malformed cell is what a fixture sends.
    // None of the three is evidence a pod is unready, and inventing a dot
    // from an absence is how a healthy pod gets condemned.
    for (const ready of ["0/0", "", "—", "1", "x/y"]) {
      expect({ ready, verdict: podStatus(vitals({ phase: "Running", ready, restarts: 5 })) }).toEqual({
        ready,
        verdict: { status: "Running", health: "success", flagged: false },
      });
    }
  });

  it("derives the same verdict off a fetched object as off the row", () => {
    // The two readings that must never disagree — the pods list row and the
    // detail header, on the same pod at the same moment.
    const object: K8sObject = {
      kind: "Pod",
      status: {
        phase: "Running",
        containerStatuses: [
          {
            name: "adapter",
            ready: false,
            restartCount: 1123,
            // The live shape: terminated with exit code 1, no waiting entry.
            state: { terminated: { exitCode: 1, reason: "Error", finishedAt: "2026-08-24T13:28:18Z" } },
          },
        ],
      },
    };
    const line = resourceStatusLine("Pod", object)!;
    expect(line).toEqual({ status: "NotReady", health: "danger", flagged: true, readyText: "0/1 ready" });
    const { readyText, ...verdict } = line;
    expect(verdict).toEqual(podStatus(BETWEEN_RESTARTS));
  });

  it("sums restarts across containers when reading an object, as the row does", () => {
    // `summarise_pod` sums `restart_count` over every container status; a
    // header that read only the first would disagree with its own row on a
    // sidecar pod.
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
      status: "NotReady",
      health: "danger",
      flagged: true,
      readyText: "1/2 ready",
    });
  });

  it("leaves a pod the kubelet has not reported containers for alone", () => {
    // No container statuses at all: `0/0`, no ratio to show, and nothing that
    // says the pod is unready — only that nobody has looked yet.
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
      // The unready branch, which neither the phase nor a waiting reason
      // reaches: a crash-looper caught between restarts, up and not ready.
      ["Pod", pod({ phase: "Running", containerStatuses: [container("a", { running: {} }, false, 1123)] })],
      // A word the phase table does not know — the only producer of UNREADABLE.
      ["Pod", pod({ phase: "Evicted" })],
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
