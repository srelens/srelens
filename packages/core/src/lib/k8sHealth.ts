/**
 * `HealthKind` is a severity, not a UI token: core has no React and may not
 * depend on either design's pill/badge type, so it declares its own instead of
 * importing classic's `StatusKind`. The five member names deliberately match
 * `StatusKind` (from `apps/desktop/src/ui/StatusPill.tsx`) and the kit's
 * equivalent, so both designs can pass a `HealthKind` value straight into
 * their own pill component with no mapping table. That overlap is a
 * convenience for today's two UIs, not a coupling — if either renames its
 * tokens, it maps at its own boundary, not here.
 */
import { asRecord, str } from "./k8sRaw";

export type HealthKind = "neutral" | "success" | "warning" | "danger" | "info";

/**
 * Classic's phase-to-tone mapping (`ResourceBrowser.tsx:135`), on the
 * `HealthKind` vocabulary above — the names already match one-for-one, so
 * either design passes the result straight into its own pill.
 *
 * `Ready`/`NotReady` are here because a Node reports readiness where a Pod
 * reports a phase, and both go through this one table; a Node never reports
 * `Running`.
 *
 * `NotReady` is no longer a Node's word alone. `podStatus` reaches for it for
 * a pod that is up and short of ready — a crash-looper caught between
 * restarts, which the API gives no phase and no waiting reason for — because
 * the fact is the same fact, and a second word for it would be a second
 * entry in this table to keep in step with the first.
 */
export function phaseKind(phase: string): HealthKind {
  switch (phase) {
    case "Running":
    case "Succeeded":
    case "Ready":
      return "success";
    case "Pending":
      return "warning";
    case "Failed":
    case "Unknown":
    case "NotReady":
      return "danger";
    default:
      return "neutral";
  }
}

export interface Condition {
  type: string;
  status: string;
  reason?: string;
  message?: string;
  lastTransitionTime?: string;
}

/**
 * Condition types whose polarity is inverted: `True` means the bad thing IS
 * happening. Matched as substrings, so one alternative covers a whole family
 * of types — `Pressure` covers a node's Memory/Disk/PID pressure, and
 * `Unavailable` covers `NetworkUnavailable` on its own.
 *
 * `Fail`, not `Failed`: the suffix is whatever the controller author
 * conjugated, and the same rule holds for every inflection of it. Reading
 * only `Failed` inverted five built-in types — a Deployment's and a
 * ReplicaSet's `ReplicaFailure`, a Namespace's three `…Failure` conditions,
 * and a Job's `FailureTarget` — painting a healthy `ReplicaFailure: False`
 * red, which is what the design mock caught.
 *
 * The alternatives after `Dangling` are the second round of the same lesson.
 * `Error` and `Remaining` are families for the same reason `Fail` is —
 * `Error` covers a PVC's `ControllerResizeError`, `NodeResizeError` AND its
 * `ModifyVolumeError`, and `Remaining` covers a Namespace's
 * `NamespaceContentRemaining` and `NamespaceFinalizersRemaining`, the two the
 * `Fail` round left behind on the very resource it fixed (the namespace
 * controller sets both `True` while deletion is blocked, and `False` with
 * "All content successfully removed" once it is not). The other three are
 * single types because nothing conjugates them: `Degraded` is the one
 * condition in the KEP-1623 vocabulary whose `True` is the bad state,
 * `DisruptionTarget` says a pod is about to be evicted, and `Denied` on a
 * CertificateSigningRequest says the request was refused.
 *
 * **A substring is a claim about every type that contains it**, which is the
 * cost of the family form and the reason one candidate is deliberately NOT
 * here. kstatus's (and Flux's) `Stalled` is exactly this shape — `True` means
 * reconciliation has given up — but "Installed" CONTAINS "stalled"
 * (i-n-**s-t-a-l-l-e-d**), so adding it would paint every operator's
 * `Installed: True` red: the same inversion this constant keeps being fixed
 * for, pointed the other way. Landing `Stalled` needs a whole-type match
 * rather than a substring one, and no type in srelens's reach needs it yet.
 * The test file's `POLARITY` table holds `Installed` as a live guard.
 */
const NEGATIVE_CONDITION = /Pressure|Unavailable|Fail|Dangling|Error|Remaining|Degraded|DisruptionTarget|Denied/i;

/**
 * A condition's tone from its type and status alone. The rule both designs
 * share: `Unknown` is amber, and otherwise a positive type is green when `True`
 * while a `NEGATIVE_CONDITION` type is green when `False`.
 *
 * THE SHAPE of the rule is what classic has drawn since before this module
 * existed; the SET it is applied to is wider, and that does re-tone some of
 * classic's pills. Every type the substring families above added and the
 * hand-written regexes did not have re-tones in classic:
 * `ReplicaFailure: False` was danger and is now success (`Failed` did not match
 * `ReplicaFailure`; `Fail` does), and a PVC's `ControllerResizeError: True`,
 * `NodeResizeError: True` and `ModifyVolumeError: True`, a Namespace's
 * `NamespaceContentRemaining`/`NamespaceFinalizersRemaining`, `Degraded: True`,
 * `DisruptionTarget: True` and a CSR's `Denied: True` were success and are now
 * danger. All of them are the polarity being read correctly for the first time,
 * which is why they are kept for both designs — but they ARE changes, and the
 * sentence here used to claim classic was untouched.
 *
 * Deliberately blind to `reason`. Reading one is a judgement about a
 * particular controller's vocabulary, which the new design makes and classic
 * does not — see `conditionKindWithReason` below, which classic must not call.
 */
export function conditionKind(c: Condition): HealthKind {
  const negative = NEGATIVE_CONDITION.test(c.type);
  if (c.status === "Unknown") return "warning";
  const good = c.status === "True" ? !negative : negative;
  return good ? "success" : "danger";
}

/**
 * Condition types whose `True` means "still working on it" — mapped to the one
 * reason that means the work has landed.
 *
 * A Deployment's `Progressing` stays `True` after a rollout completes, so the
 * status alone cannot tell an operator whether their change is out yet; only
 * the reason can. The design mock draws both halves of the pair:
 * `Progressing · True · ReplicaSetUpdated` amber in the degraded frame, and
 * `Progressing · True · NewReplicaSetAvailable` green in the healthy one.
 *
 * Keyed by TYPE, and consulted only for the types listed here — the same
 * reason string can appear on a condition of another kind, where it means
 * something else. Named by the completion reason rather than by the in-flight
 * ones so a reason nobody has seen before reads amber: claiming a rollout has
 * finished takes evidence, and being unsure of a rollout is the safe half of
 * that bet.
 */
const COMPLETION_REASON: Record<string, string> = {
  Progressing: "NewReplicaSetAvailable",
};

/**
 * `conditionKind`, plus the new design's rollout rule: a `Progressing` that is
 * `True` for any reason other than the completion one softens from green to
 * amber — including no reason at all. Green is the claim that the rollout has
 * landed, and the only evidence for that claim is the reason matching the
 * completion one exactly; an absent reason is not a different kind of
 * evidence, it is the absence of the evidence green requires, so it reads the
 * same as a reason nobody has seen before.
 *
 * OPT-IN, and a separate function rather than a flag, because it is a DESIGN
 * decision read off the new design's mock and not a correctness fix. Classic
 * is frozen and never asked for it: it calls plain `conditionKind`, so a
 * `Progressing: True` reads green there whatever its reason, exactly as it
 * always has. Teaching the shared function this rule re-toned classic's
 * condition pills with no test to catch it, which is the mistake this split
 * exists to make impossible.
 *
 * What the split does NOT claim is that classic's tones are unchanged overall —
 * `conditionKind`'s own comment lists the types the widened negative set
 * re-tones, and `ResourceOverview.test.tsx` pins them. This protects classic
 * from the ROLLOUT rule specifically, which is the one that is a matter of taste
 * rather than of polarity.
 *
 * A reason may only ever soften a green to amber. It may not repaint a
 * condition the status has already condemned — that is the polarity trap, and
 * it is how a stalled `Progressing: False` would talk its way back to healthy.
 */
export function conditionKindWithReason(c: Condition): HealthKind {
  const base = conditionKind(c);
  if (base !== "success") return base;
  const landed = COMPLETION_REASON[c.type];
  if (landed && c.status === "True" && c.reason !== landed) return "warning";
  return base;
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
 * A waiting container's tone: a back-off is a failure — the kubelet has already
 * tried and given up for now — anything else is a container still on its way
 * up. One rule, one home: `containerStateText` below tones its own waiting
 * branch with it, and `podStatus` in `k8sStatus` tones a whole pod with it.
 */
export function waitingKind(reason: string): HealthKind {
  return reason.includes("BackOff") ? "danger" : "warning";
}

/** Describe a container's runtime state, e.g. "running, ready". */
export function containerStateText(st: Record<string, unknown>): { text: string; kind: HealthKind } {
  const state = asRecord(st.state);
  const ready = st.ready === true ? ", ready" : "";
  if ("running" in state) return { text: `running${ready}`, kind: "success" };
  if ("waiting" in state) {
    const reason = str(asRecord(state.waiting).reason) || "waiting";
    return { text: `waiting - ${reason}`, kind: waitingKind(reason) };
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
