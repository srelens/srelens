/**
 * The STATUS `kubectl get pods` prints for a fetched Pod: kubectl's `printPod`
 * (`pkg/printers/internalversion/printers.go`), rule for rule. These are the
 * same rules the backend's `kubectl_status` (`crates/kube/src/workloads.rs`)
 * puts on every `PodSummary` as `status`.
 *
 * A second copy of those rules, on purpose and under contract. A list row
 * carries the backend's word; a detail header holds the raw Pod and nothing
 * else, and the two must say the same thing about one pod. Both copies run
 * every case in `crates/kube/tests/fixtures/pod_status_cases.json`, so neither
 * can drift without a red test. Add a case there before changing either.
 *
 * In kubectl's order, each later rule overriding an earlier one:
 *
 * 1. The pod's own `status.reason` (`Evicted`), else the phase. A pod held by
 *    a scheduling gate is `SchedulingGated`.
 * 2. The first init container that has not finished: `Init:<reason>` when it
 *    failed or is stuck, `Init:<finished>/<total>` while it works. A started
 *    native sidecar (`restartPolicy: Always`) counts as finished here.
 * 3. Unless an init container spoke and the pod is not yet `Initialized`, the
 *    first regular container that is waiting or terminated. A `Completed`
 *    word gives way to `Running` beside a running container of a Ready pod,
 *    else to the first non-zero exit's reason, else to `NotReady` while a
 *    container still runs.
 * 4. A pod being deleted is `Terminating`, or `Unknown` when its node was
 *    lost, unless it had already finished.
 */
import { asArray, asRecord, str } from "./k8sRaw";
import type { K8sObject } from "./manifest";

/** A terminated container in kubectl's words: its reason, else the signal that killed it, else its exit code. */
function terminatedWord(t: Record<string, unknown>): string {
  const reason = str(t.reason);
  if (reason) return reason;
  const signal = Number(t.signal ?? 0);
  return signal ? `Signal:${signal}` : `ExitCode:${Number(t.exitCode ?? 0)}`;
}

export function kubectlPodStatus(pod: K8sObject): string {
  const status = asRecord(pod.status);
  const phase = str(status.phase) || "Unknown";
  const podReason = str(status.reason);
  const conditions = asArray(status.conditions).map(asRecord);
  const conditionTrue = (type: string) =>
    conditions.some((c) => str(c.type) === type && str(c.status) === "True");

  let reason = podReason || phase;
  if (conditions.some((c) => str(c.type) === "PodScheduled" && str(c.reason) === "SchedulingGated")) {
    reason = "SchedulingGated";
  }

  const initSpecs = asArray(asRecord(pod.spec).initContainers).map(asRecord);
  const isSidecar = (name: string) =>
    initSpecs.some((c) => str(c.name) === name && str(c.restartPolicy) === "Always");
  let initializing = false;
  for (const [i, c] of asArray(status.initContainerStatuses).map(asRecord).entries()) {
    const state = asRecord(c.state);
    const terminated = state.terminated == null ? undefined : asRecord(state.terminated);
    if ((terminated && Number(terminated.exitCode ?? 0) === 0) || (isSidecar(str(c.name)) && c.started === true)) {
      continue;
    }
    const waiting = str(asRecord(state.waiting).reason);
    if (terminated) reason = `Init:${terminatedWord(terminated)}`;
    else if (waiting && waiting !== "PodInitializing") reason = `Init:${waiting}`;
    else reason = `Init:${i}/${initSpecs.length}`;
    initializing = true;
    break;
  }

  if (!initializing || conditionTrue("Initialized")) {
    let hasRunning = false;
    let errorReason: string | undefined;
    // Last to first, overwriting, so the first container with something to say has the last word.
    for (const c of asArray(status.containerStatuses).map(asRecord).reverse()) {
      const state = asRecord(c.state);
      const waiting = str(asRecord(state.waiting).reason);
      if (waiting) {
        reason = waiting;
      } else if (state.terminated != null) {
        const t = asRecord(state.terminated);
        reason = terminatedWord(t);
        if (Number(t.exitCode ?? 0) !== 0) errorReason = reason;
      } else if (c.ready === true && state.running != null) {
        hasRunning = true;
      }
    }
    if (reason === "Completed") {
      if (hasRunning && conditionTrue("Ready")) reason = "Running";
      else if (errorReason !== undefined) reason = errorReason;
      else if (hasRunning) reason = "NotReady";
    }
  }

  if (asRecord(pod.metadata).deletionTimestamp != null) {
    if (podReason === "NodeLost") reason = "Unknown";
    else if (phase !== "Succeeded" && phase !== "Failed") reason = "Terminating";
  }
  return reason;
}
