import { waitingKind } from "./k8sHealth";
import { plural } from "./k8sRaw";
import type { PodContainer } from "./workloads";

/**
 * What one container of a pod is doing, decided once so every place that
 * draws a container — the pods list's blocks, a node's pods, a pod's own page
 * — draws the same container the same way (#878).
 *
 * Six things a container can be, and one it cannot be told to be:
 *
 * - `ready` — running and passing its readiness probe. The ordinary case.
 * - `unready` — running, but not ready: still starting, or failing its probe.
 * - `starting` — waiting, on its way up (`ContainerCreating`,
 *   `PodInitializing`, or no reason given yet).
 * - `stuck` — waiting, and not going to get anywhere by waiting: a back-off,
 *   an image that cannot be pulled, a config that cannot be read.
 * - `completed` — terminated with exit code 0. Finished, not failed: an init
 *   container that did its job, a Job's container that ran to the end.
 * - `failed` — terminated with any other exit code, or `OOMKilled`.
 * - `unknown` — the kubelet has said nothing about it yet.
 *
 * **A restart is not one of these.** It is something that has happened to a
 * container, not what the container is doing now: one that restarted five
 * times is also `ready`, or also `stuck`, at this moment, and a state of its
 * own would hide which. It travels beside the state as {@link
 * ContainerVerdict.restarted}.
 */
export type ContainerStateKind =
  | "ready"
  | "unready"
  | "starting"
  | "stuck"
  | "completed"
  | "failed"
  | "unknown";

export interface ContainerVerdict {
  kind: ContainerStateKind;
  /** The state in words: `Running`, `Waiting (CrashLoopBackOff)`, `Exited 137 (OOMKilled)`. */
  word: string;
  /** The container has restarted at least once, whatever it is doing now. */
  restarted: boolean;
}

/**
 * A waiting reason that waiting will not fix. `waitingKind` already names the
 * back-offs; the rest are the errors the kubelet reports before it starts
 * backing off, or instead of it (`ErrImagePull`, `InvalidImageName`,
 * `CreateContainerConfigError`, `RunContainerError`).
 */
function isStuck(reason: string): boolean {
  return waitingKind(reason) === "danger" || /^Err|Error$|^Invalid/.test(reason);
}

export function containerVerdict(container: PodContainer): ContainerVerdict {
  const restarted = container.restarts > 0;
  const because = container.reason ? ` (${container.reason})` : "";
  switch (container.state) {
    case "running":
      return container.ready
        ? { kind: "ready", word: "Running", restarted }
        : { kind: "unready", word: "Running, not ready", restarted };
    case "waiting":
      return { kind: isStuck(container.reason) ? "stuck" : "starting", word: `Waiting${because}`, restarted };
    case "terminated": {
      // No exit code is not a clean exit: only a reported 0 is.
      if (container.exitCode === 0) return { kind: "completed", word: "Completed", restarted };
      const code = container.exitCode == null ? "" : ` ${container.exitCode}`;
      return { kind: "failed", word: `Exited${code}${because}`, restarted };
    }
    default:
      return { kind: "unknown", word: "Not reported yet", restarted };
  }
}

/**
 * One container in a sentence, for a tooltip and for a screen reader:
 * `worker: Waiting (CrashLoopBackOff), 14 restarts, acme/worker:1`.
 */
export function describeContainer(container: PodContainer): string {
  const what = container.kind === "app" ? "" : ` (${container.kind})`;
  const parts = [`${container.name}${what}: ${containerVerdict(container).word}`];
  if (container.restarts > 0) parts.push(plural(container.restarts, "restart"));
  if (container.image) parts.push(container.image);
  return parts.join(", ");
}

/** How bad each state is, worst first — what a list sorts the column by. */
const SEVERITY: Record<ContainerStateKind, number> = {
  failed: 6,
  stuck: 5,
  unready: 4,
  starting: 3,
  unknown: 2,
  ready: 1,
  completed: 0,
};

/**
 * A pod's containers as one number to sort by: the pod with the worst
 * container first, and among equals the one with more of them in that state.
 * Init containers that completed do not count against a pod.
 */
export function containersSortValue(containers: readonly PodContainer[]): number {
  let worst = 0;
  let count = 0;
  for (const container of containers) {
    const kind = containerVerdict(container).kind;
    // An init container that has finished is not part of what the pod is
    // doing: counted, it would sort a finished Job with two of them apart
    // from the same Job with none.
    if (container.kind === "init" && kind === "completed") continue;
    const severity = SEVERITY[kind];
    if (severity > worst) {
      worst = severity;
      count = 1;
    } else if (severity === worst) {
      count += 1;
    }
  }
  return worst * 1000 + Math.min(count, 999);
}

/**
 * The one word a row prints beside a pod's squares, or `null` when every
 * container is as it should be.
 *
 * A square's colour is not a signal on its own, so whatever is not ordinary
 * is also said: the worst container's reason, or its state where it has no
 * reason. A pod whose containers are all ready — or finished, for one that
 * was meant to finish — needs no word; the squares and the Status column
 * already say it.
 */
export function containersWord(containers: readonly PodContainer[]): string | null {
  let worst: PodContainer | undefined;
  let worstSeverity = SEVERITY.ready;
  for (const container of containers) {
    const severity = SEVERITY[containerVerdict(container).kind];
    if (severity > worstSeverity) {
      worst = container;
      worstSeverity = severity;
    }
  }
  if (!worst) return null;
  const { kind } = containerVerdict(worst);
  if (worst.reason) return worst.reason;
  switch (kind) {
    case "failed":
      return worst.exitCode == null ? "Exited" : `Exited ${worst.exitCode}`;
    case "unready":
      return "Not ready";
    case "starting":
      return "Starting";
    default:
      return "Not reported";
  }
}

/** `2 of 3 containers ready` — the figure the Ready column used to print. */
export function containersReadyText(containers: readonly PodContainer[]): string {
  const counted = containers.filter((c) => c.kind !== "init");
  const ready = counted.filter((c) => c.ready).length;
  return `${ready} of ${plural(counted.length, "container")} ready`;
}
