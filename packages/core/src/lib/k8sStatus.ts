/**
 * One resource's status line, derived from a fetched object rather than from a
 * list row.
 *
 * Every health predicate srelens already had — `podFlagged`,
 * `deploymentFlagged` and their siblings in the new design's column table — is
 * typed on a *summary row* the backend built (`PodSummary`, `DeploymentSummary`
 * …). A detail pane holds a `K8sObject` instead, and has no row to ask. This
 * module answers the same questions off the object, reusing the rules rather
 * than restating them: `phaseKind` for a phase word's tone, `waitingKind` for a
 * stuck container's.
 *
 * The word, the tone and the flag are decided together — see `Verdict` below.
 * That is deliberate: they were once derived by separate functions, and a
 * `Succeeded` pod ended up with a green pill and a red "needs attention" dot at
 * the same time. Two readings of one fact can disagree; one reading cannot.
 */
import { phaseKind, waitingKind, type HealthKind } from "./k8sHealth";
import { asArray, asRecord, str } from "./k8sRaw";
import type { K8sObject } from "./manifest";

/** A status word with the tone and the dot that go with it. */
export interface StatusVerdict {
  /**
   * The kind's own status word — "Running", "Degraded", "Pending",
   * "Succeeded", "Failed", "CrashLoopBackOff", "Complete", "Suspended",
   * "Ready,SchedulingDisabled". One kind's vocabulary is not another's; this
   * is whatever that kind calls the state it is in.
   */
  status: string;
  /** Tone for the word, and for the unhealthy dot when `flagged`. */
  health: HealthKind;
  /** Whether the resource needs attention — a list row's dot, or the detail header's. */
  flagged: boolean;
}

export interface ResourceStatusLine extends StatusVerdict {
  /**
   * The whole ready phrase, its noun included — "9/12 ready", "1/1 ready",
   * "3/3 complete" — for rendering verbatim, or `null` where the kind has no
   * such count. The noun is part of the string because it is not "ready" for
   * every kind: a Job counts completions, not readiness.
   */
  readyText: string | null;
}

/**
 * The six (tone, dot) pairs a status word may carry, and the ONLY place in
 * srelens where a tone and a flag are written side by side. Every branch below
 * picks one of these by name and supplies a word; none of them can pair a
 * green pill with a red dot, because there is no verdict that says that.
 *
 * The pairs are not derivable from the tone alone, which is why they are
 * enumerated rather than computed: a running Job is amber and NOT flagged
 * (`jobFlagged` says only a failure earns the dot — still running is not yet
 * wrong), while a Pending pod is amber and IS flagged.
 */
const WELL = { health: "success", flagged: false } as const;
/** Deliberately at rest: scaled to zero, suspended. Nothing to see. */
const AT_REST = { health: "neutral", flagged: false } as const;
/** Working, and expected to be: an active Job. Amber, but no dot. */
const IN_FLIGHT = { health: "warning", flagged: false } as const;
/** On its way, or held back — worth a look but not a failure. */
const UNSETTLED = { health: "warning", flagged: true } as const;
const BROKEN = { health: "danger", flagged: true } as const;
/** A word we do not know. No colour it has not earned, but still a dot: not
 *  recognising a state is not the same as knowing it is fine. */
const UNREADABLE = { health: "neutral", flagged: true } as const;

type Verdict =
  | typeof WELL
  | typeof AT_REST
  | typeof IN_FLIGHT
  | typeof UNSETTLED
  | typeof BROKEN
  | typeof UNREADABLE;

const verdict = (status: string, v: Verdict): StatusVerdict => ({ status, ...v });

/** A count off `status`/`spec`, absent meaning zero (as the backend's list summaries read it). */
function count(value: unknown): number {
  const n = Number(value ?? 0);
  return Number.isFinite(n) ? n : 0;
}

/**
 * The word a kind uses when nothing at all is desired. Only core knows which
 * kind uses which: a Deployment at zero replicas is "Scaled down", a DaemonSet
 * matching no node is "Not scheduled", and a caller that picked its own word
 * would be a second table of the same fact.
 */
function zeroWord(kind: string): string {
  return kind === "DaemonSet" ? "Not scheduled" : "Scaled down";
}

/**
 * Ready-out-of-desired, shared by every kind that scales: fewer ready than
 * desired is Degraded, which is `deploymentFlagged`/`statefulSetFlagged`/
 * `daemonSetFlagged`'s rule (`ready < desired`) read off the object.
 *
 * Nothing desired is not a failure — a Deployment scaled to zero, or a
 * DaemonSet whose selector matches no node, is doing exactly what it was
 * asked. The list's flagged rules agree: `0 < 0` is false, so no dot.
 *
 * Exported on the two bare counts, not on an object, for the same reason
 * {@link podStatus} is: a list row carries the counts and no object at all
 * (`DeploymentSummary.ready` is the string "1/3", `DaemonSetSummary` a pair of
 * numbers), and the row's pill has to say what the header says about the same
 * workload. It did not — the Workloads table paired "Progressing"/amber and
 * "Available"/green by hand while this returned "Degraded"/red and
 * "Scaled down"/neutral for the identical object, so double-clicking a row
 * contradicted it. (#331)
 */
export function scaledStatus(kind: string, ready: number, desired: number): StatusVerdict {
  if (desired === 0) return verdict(zeroWord(kind), AT_REST);
  if (ready < desired) return verdict("Degraded", BROKEN);
  return verdict("Running", WELL);
}

/** {@link scaledStatus} with the ready phrase only a detail header shows. */
function scaledLine(kind: string, ready: number, desired: number): ResourceStatusLine {
  return { ...scaledStatus(kind, ready, desired), readyText: `${ready}/${desired} ready` };
}

/**
 * A workload's ready replicas out of its desired ones.
 *
 * `status.readyReplicas`, NOT `status.availableReplicas`. They are different
 * fields: available is the subset of ready replicas that have also outlived
 * `minReadySeconds`, so a healthy rollout sits at ready > available for a
 * while. A line that says "ready" has to count the ready ones — the backend's
 * `DeploymentSummary.ready` reads the same field, so the header and the list
 * agree on one number.
 */
function replicaStatusLine(kind: string, object: K8sObject): ResourceStatusLine {
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  return scaledLine(kind, count(status.readyReplicas), count(spec.replicas));
}

/** A DaemonSet counts nodes, not replicas: `numberReady` of `desiredNumberScheduled`. */
function daemonSetStatusLine(object: K8sObject): ResourceStatusLine {
  const status = asRecord(object.status);
  return scaledLine("DaemonSet", count(status.numberReady), count(status.desiredNumberScheduled));
}

/** The phases past which a pod is finished, and a container state cannot speak for it. */
const TERMINAL_POD_PHASES = ["Succeeded", "Failed"];

/**
 * The facts one pod carries, wherever it is read from.
 *
 * Taken as a record rather than as positional arguments because a
 * `PodSummary` — and every row type built on one — is already this shape, so
 * every call site hands over the whole row and CANNOT omit a field it does
 * not happen to be thinking about. That is not a style preference: the
 * flicker below existed because the two facts that were passed were the only
 * two the signature asked for, and the fact that would have settled it was
 * sitting unread on the same row.
 */
export interface PodVitals {
  /** `status.phase` — "Running", "Pending", "Succeeded", "Failed", "Unknown". */
  phase: string;
  /** Why the first waiting container is waiting; `""` or absent when none is. */
  waitingReason?: string;
  /**
   * Ready containers out of reported ones, exactly as a row prints it and
   * kubectl's READY column shows it: `"1/1"`, `"0/2"`. `"0/0"` means the
   * kubelet has reported no containers, which is an absence and not a
   * reading.
   */
  ready: string;
  /** Restarts summed across the pod's containers, as `summarise_pod` sums them. */
  restarts: number;
}

/**
 * Whether a READY cell says a container is short of ready.
 *
 * Anything that does not parse as two numbers is NOT short: `"0/0"` (nothing
 * reported), `""` (a fixture), a dash. Reading an absence as a failure is how
 * a healthy pod gets a red dot, and the backend's own `short_of_ready` errs
 * the other way for the opposite reason — there, a cell it cannot read is
 * worth one extra fetch; here, it would be worth a wrong verdict.
 */
function shortOfReady(cell: string): boolean {
  const [have, want] = cell.split("/").map(Number);
  return have < want;
}

/**
 * `podFlagged`'s rule as a verdict: anything the phase table does not call
 * healthy earns the dot, and keeps its own tone while doing so.
 */
function phaseVerdict(phase: string): Verdict {
  const health = phaseKind(phase);
  if (health === "success") return WELL;
  if (health === "warning") return UNSETTLED;
  if (health === "danger") return BROKEN;
  return UNREADABLE;
}

/**
 * A pod's status word, tone and dot, from the facts a list row and a fetched
 * object can both supply — see {@link PodVitals}.
 *
 * `status.phase` alone is not enough, and this is not a nicety: a pod whose
 * only container sits in `CrashLoopBackOff` still reports phase `Running`, so
 * anything reading the phase by itself draws a crash-looping pod green and
 * healthy. kubectl shows the waiting reason for exactly this reason, and so do
 * we — in the list and in the detail header, through this one function, so the
 * two can never say different things about the same pod.
 *
 * **The waiting reason is not enough either, because a crash-looping pod is
 * not always waiting.** Between restarts the container is genuinely up: phase
 * `Running`, no waiting reason, nothing to read — and the pod fell out of
 * every unhealthy list for that instant and came back when the container
 * failed again. On a real cluster two of four consecutive screenshots of the
 * overview showed the same pod in `NOT READY` and two did not, with nothing
 * changed but the moment of the capture. The waiting reason names a state the
 * pod is only intermittently in; the READY ratio names the state it is
 * actually in, and does not move between the two moments.
 *
 * So a pod that is up, short of ready, AND has restarted is `NotReady`. The
 * restart count is doing real work in that sentence and is not belt-and-
 * braces: a container that has been up for two seconds and has not yet passed
 * its readiness probe is a NORMAL pod mid-rollout, and the ratio alone cannot
 * tell it from a crash-looper — a row carries no clock, so "not ready yet"
 * and "not ready for an hour" are the same row. Having died at least once is
 * the only evidence in the snapshot that this is failure rather than
 * start-up. (A pod that has never restarted and never becomes ready — a
 * readiness probe that never passes — is therefore still read as healthy
 * here. That is a different bug with a different signal, and guessing at it
 * from these fields would cost every rolling update a red dot.)
 *
 * `NotReady` is not a word invented beside a colour: it is the word
 * {@link nodeStatus} already uses for the same fact, and `phaseKind` already
 * tones it, so the branch below picks a verdict by name like every other one
 * in this file.
 *
 * A terminal pod is left alone, and the ratio must not reach it. Its
 * containers are TERMINATED, so a `Succeeded` pod reports `0/1` forever —
 * reading that as "not ready" would flag every finished pod in the cluster,
 * and put a green pill beside a red dot on each of them, which is the exact
 * bug this file's `Verdict` union exists to prevent.
 */
export function podStatus(pod: PodVitals): StatusVerdict {
  const word = pod.phase || "Unknown";
  if (TERMINAL_POD_PHASES.includes(word)) return verdict(word, phaseVerdict(word));
  if (pod.waitingReason) {
    return verdict(pod.waitingReason, waitingKind(pod.waitingReason) === "danger" ? BROKEN : UNSETTLED);
  }
  // Only where the phase would otherwise say the pod is well. A Pending pod is
  // already amber and already flagged, and an unrecognised word already earns
  // its dot; overriding either with the ratio would lose the reader the more
  // specific fact for no gain in the verdict.
  //
  // Compared on the tone rather than on {@link WELL}'s identity, for the same
  // reason {@link nodeStatus} is: everything else in this file is safe by
  // construction, and an identity check would be safe only by coincidence.
  if (phaseVerdict(word).health === "success" && shortOfReady(pod.ready) && pod.restarts > 0) {
    return verdict("NotReady", phaseVerdict("NotReady"));
  }
  return verdict(word, phaseVerdict(word));
}

/**
 * The same reading, off a fetched Pod: pull the phase, the first waiting
 * reason, the ready ratio and the restart total out of the object — the four
 * fields `summarise_pod` puts on a `PodSummary` — and hand them to
 * `podStatus`, then add the ready phrase only a detail header shows.
 */
function podStatusLine(object: K8sObject): ResourceStatusLine {
  const status = asRecord(object.status);
  const statuses = asArray(status.containerStatuses).map(asRecord);
  const ready = statuses.filter((c) => c.ready === true).length;
  // No container statuses at all means the kubelet has not reported yet —
  // "0/0 ready" would read as a fact when it is an absence.
  const readyText = statuses.length > 0 ? `${ready}/${statuses.length} ready` : null;
  // First waiting container, in container order — the same one the backend
  // summarises onto a row.
  const waitingReason = statuses
    .map((c) => str(asRecord(asRecord(c.state).waiting).reason))
    .find((reason) => reason !== "");
  const vitals: PodVitals = {
    phase: str(status.phase),
    waitingReason,
    ready: `${ready}/${statuses.length}`,
    // Summed across containers, as `summarise_pod` sums `restart_count`: a
    // header that read only the first container would disagree with its own
    // row on a sidecar pod.
    restarts: statuses.reduce((total, c) => total + count(c.restartCount), 0),
  };
  return { ...podStatus(vitals), readyText };
}

/**
 * A Job's outcome, on the list's own rule (`jobColumns`' status pill and
 * `jobFlagged`): a failure is a failure, an in-flight Job is amber but is NOT
 * flagged — still running is not yet wrong — and anything else has finished.
 */
export function jobStatus(failed: number, active: number): StatusVerdict {
  if (failed > 0) return verdict("Failed", BROKEN);
  if (active > 0) return verdict("Active", IN_FLIGHT);
  return verdict("Complete", WELL);
}

/** {@link jobStatus} off a fetched Job, plus the completion count a header shows. */
function jobStatusLine(object: K8sObject): ResourceStatusLine {
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  // An unset `completions` means one, per the Job API's own default.
  const completions = spec.completions != null ? count(spec.completions) : 1;
  const readyText = `${count(status.succeeded)}/${completions} complete`;
  return { ...jobStatus(count(status.failed), count(status.active)), readyText };
}

/**
 * A CronJob is suspended or it is not — the list's two pills. It has no
 * unhealthy state of its own (a CronJob deliberately has no `flagged` rule:
 * the health lives in the Jobs it spawns), and no ready count.
 */
export function cronJobStatus(suspended: boolean): StatusVerdict {
  return suspended ? verdict("Suspended", AT_REST) : verdict("Active", WELL);
}

/** {@link cronJobStatus} off a fetched CronJob's own `spec.suspend`. */
function cronJobStatusLine(object: K8sObject): ResourceStatusLine {
  return { ...cronJobStatus(asRecord(object.spec).suspend === true), readyText: null };
}

/**
 * A node's readiness, as the backend's `NodeSummary.status` derives it: the
 * `Ready` condition True is "Ready", any other value "NotReady", and no such
 * condition at all "Unknown".
 *
 * Cordoning is appended the way kubectl prints it, and warns: the list already
 * badges `SchedulingDisabled` in the warning tone, and a node that is refusing
 * new pods is a thing the reader wants marked. A node that is also NotReady
 * keeps the worse of the two verdicts.
 */
export function nodeStatus(readiness: string, unschedulable: boolean): StatusVerdict {
  const word = readiness || "Unknown";
  const verdictForWord = phaseVerdict(word);
  if (!unschedulable) return verdict(word, verdictForWord);
  // Compared on the tone, not on the constant's identity: everything else in
  // this file is safe by construction, and an identity check would be safe
  // only by coincidence — a `phaseVerdict` that ever returned a fresh
  // structurally-equal object would break this line silently.
  return verdict(
    `${word},SchedulingDisabled`,
    verdictForWord.health === "danger" ? BROKEN : UNSETTLED,
  );
}

/**
 * {@link nodeStatus} off a fetched Node: the readiness word the backend's
 * `NodeSummary.status` carries, derived here from the `Ready` condition
 * instead — True is "Ready", any other value "NotReady", no such condition at
 * all "Unknown".
 */
function nodeStatusLine(object: K8sObject): ResourceStatusLine {
  const conditions = asArray(asRecord(object.status).conditions).map(asRecord);
  const ready = conditions.find((c) => str(c.type) === "Ready");
  const word = ready === undefined ? "Unknown" : str(ready.status) === "True" ? "Ready" : "NotReady";
  return { ...nodeStatus(word, asRecord(object.spec).unschedulable === true), readyText: null };
}

/**
 * `eventVerdict`'s own two-member union, deliberately narrower than the
 * general `{ health: HealthKind; bad: boolean }` shape it used to return:
 * that wider shape does not stop `{ health: "success", bad: true }` from
 * typechecking — the exact "green pill, red dot" pairing this file's
 * `Verdict` union exists to make unrepresentable. Declaring the two legal
 * pairs by name closes the same hole here: constructing the illegal pair
 * against this type is a compile error (see the test that proves it).
 */
type EventVerdict = { health: "danger"; bad: true } | { health: "neutral"; bad: false };

/**
 * An event's tone — the ONE rule for it, replacing two hand-paired ones: the
 * classic list's danger/info and the detail pane's warning/neutral. Per the
 * design (mock-full-design §B.2): `Warning` is danger and bold; everything
 * else — `Normal`, or a type this cluster invented — reads plain.
 *
 * Reuses the same six-pair table above rather than writing a seventh: a
 * `Warning` is exactly {@link BROKEN}'s (danger, dot-worthy) pair, and
 * everything else is exactly {@link AT_REST}'s (neutral, nothing to see)
 * pair. `bad` is that pair's `flagged`, renamed because an event has no
 * status word of its own to hang a dot off — it names only whether the WORD
 * is worth colouring, which is what the kit's `StatusPill` calls `tinted`.
 *
 * An unrecognised type is deliberately read the SAME as `Normal`, not as
 * {@link UNREADABLE} — and this is a principled asymmetry, not a special
 * case. `UNREADABLE` exists because a resource's status WORD is a vocabulary
 * this file claims to know (a phase, a condition reason); failing to
 * recognise one means the reader is looking at a state nobody has assessed,
 * and a dot is the honest answer. An event's `type` is not a status at all —
 * the API defines exactly two values, and the event carries its own
 * separately-toned `reason` and `message` to say what actually happened; the
 * field itself asserts no health about anything. "Not Warning" is therefore
 * not "an unknown state" the way an unrecognised phase is — it is "not the
 * alarm channel". Colouring it would train the reader to ignore red.
 */
export function eventVerdict(type: string): EventVerdict {
  if (type === "Warning") return { health: BROKEN.health, bad: BROKEN.flagged };
  return { health: AT_REST.health, bad: AT_REST.flagged };
}

/**
 * The status line for a fetched resource, or `null` for a kind that has none.
 *
 * `kind` is passed rather than read off `object.kind` for the same reason the
 * detail bodies take it: the caller already knows which kind it asked for, and
 * a `K8sObject` is whatever JSON came back.
 *
 * A kind that is not listed here is not a gap. A ConfigMap has no health, and
 * a custom resource's `status` is its own CRD's business — a `status.phase`
 * that happens to read "Degraded" on some operator's object means whatever
 * that operator decided. Returning `null` says "this pane draws no status
 * line", which is the honest answer; guessing would put a red dot on a healthy
 * resource.
 */
export function resourceStatusLine(kind: string, object: K8sObject): ResourceStatusLine | null {
  switch (kind) {
    case "Pod":
      return podStatusLine(object);
    case "Deployment":
    case "StatefulSet":
    case "ReplicaSet":
      return replicaStatusLine(kind, object);
    case "DaemonSet":
      return daemonSetStatusLine(object);
    case "Job":
      return jobStatusLine(object);
    case "CronJob":
      return cronJobStatusLine(object);
    case "Node":
      return nodeStatusLine(object);
    default:
      return null;
  }
}
