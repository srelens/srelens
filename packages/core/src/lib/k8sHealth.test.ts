import { describe, it, expect } from "vitest";
import {
  conditionKind,
  conditionKindWithReason,
  containerStateText,
  orderPodConditions,
  phaseKind,
  waitingKind,
} from "./k8sHealth";

describe("orderPodConditions", () => {
  it("orders lifecycle conditions PodScheduled → Initialized → ContainersReady → Ready", () => {
    const shuffled = [
      { type: "Ready", status: "True" },
      { type: "PodScheduled", status: "True" },
      { type: "ContainersReady", status: "False" },
      { type: "Initialized", status: "True" },
    ];
    expect(orderPodConditions(shuffled).map((c) => c.type)).toEqual([
      "PodScheduled",
      "Initialized",
      "ContainersReady",
      "Ready",
    ]);
  });

  it("appends unknown condition types after the known lifecycle ones", () => {
    const conds = [
      { type: "DisruptionTarget", status: "True" },
      { type: "Ready", status: "True" },
      { type: "PodScheduled", status: "True" },
    ];
    expect(orderPodConditions(conds).map((c) => c.type)).toEqual([
      "PodScheduled",
      "Ready",
      "DisruptionTarget",
    ]);
  });
});

// classic's ResourceOverview.test.tsx did not cover conditionKind; written here
// against the body as moved (see k8sHealth.ts), not against the function name.
describe("conditionKind", () => {
  it("is a warning when the status itself is Unknown, regardless of the condition type", () => {
    expect(conditionKind({ type: "Ready", status: "Unknown" })).toBe("warning");
    expect(conditionKind({ type: "MemoryPressure", status: "Unknown" })).toBe("warning");
  });

  it("treats a True status on a normal (non-negative) type as healthy", () => {
    expect(conditionKind({ type: "Ready", status: "True" })).toBe("success");
    expect(conditionKind({ type: "Initialized", status: "True" })).toBe("success");
  });

  it("treats a False status on a normal (non-negative) type as unhealthy", () => {
    expect(conditionKind({ type: "Ready", status: "False" })).toBe("danger");
  });

  it("inverts the polarity for the Pressure alternative", () => {
    // For a "bad thing" type, True means the bad thing IS happening (danger),
    // and False means it is NOT happening (success) — the opposite of a
    // normal type's polarity.
    expect(conditionKind({ type: "MemoryPressure", status: "True" })).toBe("danger");
    expect(conditionKind({ type: "MemoryPressure", status: "False" })).toBe("success");
    expect(conditionKind({ type: "DiskPressure", status: "True" })).toBe("danger");
  });

  it("inverts the polarity for the bare Unavailable alternative, on a type that is not NetworkUnavailable", () => {
    expect(conditionKind({ type: "Unavailable", status: "True" })).toBe("danger");
    expect(conditionKind({ type: "Unavailable", status: "False" })).toBe("success");
  });

  it("still reads a node's NetworkUnavailable, through the bare Unavailable alternative", () => {
    // The regex used to name `NetworkUnavailable` as its own alternative,
    // which `Unavailable` already matched as a substring — dead weight no
    // test could prove necessary, since deleting it changed no behaviour. It
    // is gone; this asserts the type it named still reads correctly.
    expect(conditionKind({ type: "NetworkUnavailable", status: "True" })).toBe("danger");
    expect(conditionKind({ type: "NetworkUnavailable", status: "False" })).toBe("success");
  });

  it("inverts the polarity for the Failed alternative, on a type containing none of the others", () => {
    expect(conditionKind({ type: "JobFailed", status: "True" })).toBe("danger");
    expect(conditionKind({ type: "JobFailed", status: "False" })).toBe("success");
  });

  it("reads a Deployment's ReplicaFailure as the bad-thing type it is", () => {
    // The one the design mock caught: frame A draws `ReplicaFailure: False`
    // with an ok dot and an uncoloured name, because False means the bad
    // thing is NOT happening. Matching `Failed` but not `Failure` inverted
    // that and painted a healthy Deployment's condition red.
    expect(conditionKind({ type: "ReplicaFailure", status: "False" })).toBe("success");
    expect(conditionKind({ type: "ReplicaFailure", status: "True" })).toBe("danger");
  });

  it("reads every other built-in condition of the same word family", () => {
    // ReplicaFailure was not the only one: a Namespace reports three
    // conditions ending in `Failure`, and a Job (1.31+) reports
    // `FailureTarget`. All five were inverted by the same missing suffix.
    for (const type of [
      "NamespaceDeletionContentFailure",
      "NamespaceDeletionDiscoveryFailure",
      "NamespaceDeletionGroupVersionParsingFailure",
      "FailureTarget",
    ]) {
      expect(conditionKind({ type, status: "True" })).toBe("danger");
      expect(conditionKind({ type, status: "False" })).toBe("success");
    }
  });

  it("recognises the whole word family, not one inflection of it", () => {
    // `Fail` rather than `Failed`: whatever a controller author conjugates,
    // the polarity is the same, and getting it wrong inverts the colour
    // rather than merely missing it.
    expect(conditionKind({ type: "Failing", status: "True" })).toBe("danger");
    expect(conditionKind({ type: "Failure", status: "True" })).toBe("danger");
    expect(conditionKind({ type: "Failed", status: "True" })).toBe("danger");
  });

  it("inverts the polarity for the Dangling alternative, on a type containing none of the others", () => {
    expect(conditionKind({ type: "VolumeDangling", status: "True" })).toBe("danger");
    expect(conditionKind({ type: "VolumeDangling", status: "False" })).toBe("success");
  });

  it("ignores a reason entirely, which is what keeps classic's pills where they were", () => {
    // The rollout rule below is the NEW design's, off its mock. Classic renders
    // `conditionKind` and never asked for it, so this function must stay blind
    // to `reason`: teaching it one silently re-toned a frozen app's condition
    // pills, with no test to catch it.
    expect(conditionKind({ type: "Progressing", status: "True", reason: "ReplicaSetUpdated" })).toBe("success");
    expect(conditionKind({ type: "Progressing", status: "True", reason: "NewReplicaSetAvailable" })).toBe("success");
    expect(conditionKind({ type: "Progressing", status: "True", reason: "SomeFutureReason" })).toBe("success");
  });

  it("matches the negative-type regex case-insensitively", () => {
    expect(conditionKind({ type: "networkunavailable", status: "True" })).toBe("danger");
  });
});

/**
 * Every condition type this rule claims to know, paired with the status that
 * means the resource is WELL — the whole polarity table in one place, walked
 * from both ends.
 *
 * Both directions are asserted for every row, and that is the point rather
 * than thoroughness for its own sake. A polarity bug is invisible from one
 * side: a rule that answered "danger" to everything would satisfy every
 * `True → danger` row in the negative half, and a rule that answered
 * "success" to everything would satisfy every `True → success` row in the
 * positive half. Only asserting the OTHER status of the same type catches
 * either. It is also why the positive half is here at all: the negative half
 * is matched by SUBSTRING, so an alternative added for one type silently
 * re-tones every type that happens to contain it, and the positive rows are
 * the only thing standing between this rule and that mistake.
 */
const POLARITY: [type: string, wellWhen: "True" | "False", note: string][] = [
  // ---- Positive types: True is the healthy answer. --------------------
  ["Ready", "True", "core/v1 Pod + Node"],
  ["Initialized", "True", "core/v1 Pod"],
  ["ContainersReady", "True", "core/v1 Pod"],
  ["PodScheduled", "True", "core/v1 Pod"],
  ["PodReadyToStartContainers", "True", "core/v1 Pod"],
  ["Available", "True", "apps/v1 Deployment"],
  ["Progressing", "True", "apps/v1 Deployment"],
  ["Complete", "True", "batch/v1 Job"],
  ["SuccessCriteriaMet", "True", "batch/v1 Job"],
  ["Approved", "True", "certificates/v1 CSR"],
  ["Established", "True", "apiextensions CRD"],
  ["NamesAccepted", "True", "apiextensions CRD"],
  ["DisruptionAllowed", "True", "policy/v1 PodDisruptionBudget"],
  // The collision guard, and it is not hypothetical: "Installed" CONTAINS
  // "Stalled" (i-n-**s-t-a-l-l-e-d**), so the kstatus/Flux convention's
  // `Stalled` cannot be added to the substring family without painting every
  // operator's `Installed: True` red — the exact inversion this rule keeps
  // being fixed for, in the other direction. See NEGATIVE_CONDITION.
  ["Installed", "True", "OLM / operator convention — contains 'Stalled'"],
  ["Uninstalled", "True", "same substring trap"],

  // ---- Negative types: False is the healthy answer. -------------------
  ["MemoryPressure", "False", "core/v1 Node"],
  ["DiskPressure", "False", "core/v1 Node"],
  ["PIDPressure", "False", "core/v1 Node"],
  ["NetworkUnavailable", "False", "core/v1 Node"],
  ["ReplicaFailure", "False", "apps/v1 Deployment + ReplicaSet"],
  ["Failed", "False", "batch/v1 Job, certificates/v1 CSR"],
  ["FailureTarget", "False", "batch/v1 Job"],
  ["NamespaceDeletionContentFailure", "False", "core/v1 Namespace"],
  ["NamespaceDeletionDiscoveryFailure", "False", "core/v1 Namespace"],
  ["NamespaceDeletionGroupVersionParsingFailure", "False", "core/v1 Namespace"],
  // The two the previous round left behind on the very resource it fixed:
  // the namespace controller sets these True while deletion is blocked, and
  // False with "All content successfully removed" once it is not.
  ["NamespaceContentRemaining", "False", "core/v1 Namespace — deletion blocked"],
  ["NamespaceFinalizersRemaining", "False", "core/v1 Namespace — deletion blocked"],
  ["Dangling", "False", "flowcontrol FlowSchema"],
  ["Degraded", "False", "KEP-1623 / operator convention"],
  ["DisruptionTarget", "False", "core/v1 Pod — about to be evicted"],
  ["Denied", "False", "certificates/v1 CSR"],
  ["ControllerResizeError", "False", "core/v1 PersistentVolumeClaim"],
  ["NodeResizeError", "False", "core/v1 PersistentVolumeClaim"],
  ["ModifyVolumeError", "False", "core/v1 PersistentVolumeClaim"],
];

describe("conditionKind — the polarity of every type it claims to know", () => {
  it.each(POLARITY)("reads %s (%s is well — %s) from both ends", (type, wellWhen) => {
    const ill = wellWhen === "True" ? "False" : "True";
    expect({ type, status: wellWhen, kind: conditionKind({ type, status: wellWhen }) }).toEqual({
      type,
      status: wellWhen,
      kind: "success",
    });
    expect({ type, status: ill, kind: conditionKind({ type, status: ill }) }).toEqual({
      type,
      status: ill,
      kind: "danger",
    });
  });

  it("keeps Unknown amber for every one of them, whichever way the type points", () => {
    // The status trumps the polarity: nobody knows, so nobody is condemned.
    for (const [type] of POLARITY) {
      expect({ type, kind: conditionKind({ type, status: "Unknown" }) }).toEqual({ type, kind: "warning" });
    }
  });

  it("covers both polarities, so neither half can be satisfied by a constant answer", () => {
    // The table is only a polarity test while it holds rows of both kinds —
    // a defect on the previous plan survived its first pass because a fixture
    // held one state and a wrong rule agreed with the right one by accident.
    expect(POLARITY.filter(([, well]) => well === "True").length).toBeGreaterThan(0);
    expect(POLARITY.filter(([, well]) => well === "False").length).toBeGreaterThan(0);
  });
});

// The new design's opt-in extension of `conditionKind`: same rule, plus a
// reason read for the types that need one. Classic must never call this.
describe("conditionKindWithReason", () => {
  it("separates a rollout in flight from one that has landed, on the reason", () => {
    // The design mock draws both frames of this pair: frame A's degraded
    // Deployment has `Progressing · True · ReplicaSetUpdated` in amber, and
    // frame B's healthy one has `Progressing · True · NewReplicaSetAvailable`
    // in green. Same type, same status — the reason is the only thing that
    // separates "working on it" from "done", which is the one fact an
    // operator opens a Deployment to find out.
    expect(conditionKindWithReason({ type: "Progressing", status: "True", reason: "ReplicaSetUpdated" })).toBe("warning");
    expect(conditionKindWithReason({ type: "Progressing", status: "True", reason: "NewReplicaSetAvailable" })).toBe("success");
  });

  it("reads any other in-flight reason as unsettled too, not only the mock's one", () => {
    // Named the other way round on purpose: only the completion reason is
    // listed, so a rollout reason nobody has seen before reads amber rather
    // than claiming a rollout has finished on no evidence.
    for (const reason of ["NewReplicaSetCreated", "FoundNewReplicaSet", "DeploymentPaused", "SomeFutureReason"]) {
      expect(conditionKindWithReason({ type: "Progressing", status: "True", reason })).toBe("warning");
    }
  });

  it("reads a reasonless Progressing as unsettled, the same as an unrecognised one", () => {
    // Green here is a claim that the rollout has landed, and the only
    // evidence for that claim is the completion reason matching exactly. No
    // reason is not a different kind of evidence than a reason nobody has
    // seen before — it is the absence of the one piece of evidence that
    // would justify green, so it reads amber for the same reason an unknown
    // reason does. (The mock's "—" for an absent reason is about what the
    // Reason column prints, not what tone the pill takes.)
    expect(conditionKindWithReason({ type: "Progressing", status: "True" })).toBe("warning");
    expect(conditionKindWithReason({ type: "Progressing", status: "True", reason: "" })).toBe("warning");
  });

  it("keeps the reason from ever rescuing a condition the status already condemns", () => {
    // Polarity first: the reason may only soften a green to amber, never
    // repaint a red. `Progressing: False` is a stalled rollout whatever it
    // says about itself.
    expect(conditionKindWithReason({ type: "Progressing", status: "False", reason: "ProgressDeadlineExceeded" })).toBe("danger");
    expect(conditionKindWithReason({ type: "Progressing", status: "False", reason: "NewReplicaSetAvailable" })).toBe("danger");
    expect(conditionKindWithReason({ type: "Progressing", status: "Unknown", reason: "NewReplicaSetAvailable" })).toBe("warning");
  });

  it("tones on the type-and-reason pair, never on the reason alone", () => {
    // The same reason string can appear on a condition of another type, where
    // it means something else entirely. Only the types named in the table
    // consult a reason at all.
    expect(conditionKindWithReason({ type: "Available", status: "True", reason: "ReplicaSetUpdated" })).toBe("success");
    expect(conditionKindWithReason({ type: "Available", status: "True", reason: "MinimumReplicasAvailable" })).toBe("success");
    expect(conditionKindWithReason({ type: "ReplicaFailure", status: "False", reason: "ReplicaSetUpdated" })).toBe("success");
    expect(conditionKindWithReason({ type: "Ready", status: "True", reason: "KubeletReady" })).toBe("success");
  });


  it("agrees with conditionKind on every condition that carries no reason", () => {
    // The extension is exactly that — an extension. A condition without a
    // reason, or of a type the table does not name, tones identically.
    for (const c of [
      { type: "Ready", status: "True" },
      { type: "Ready", status: "False" },
      { type: "MemoryPressure", status: "True" },
      { type: "ReplicaFailure", status: "False" },
      { type: "Available", status: "Unknown" },
      { type: "Available", status: "True", reason: "MinimumReplicasAvailable" },
    ]) {
      expect(conditionKindWithReason(c)).toBe(conditionKind(c));
    }
  });
});

// classic's ResourceOverview.test.tsx did not cover containerStateText either;
// written here against its actual branches.
describe("containerStateText", () => {
  it("reports a running container as success, with ready appended when ready", () => {
    expect(containerStateText({ state: { running: { startedAt: "2026-01-01T00:00:00Z" } } })).toEqual({
      text: "running",
      kind: "success",
    });
    expect(
      containerStateText({ state: { running: { startedAt: "2026-01-01T00:00:00Z" } }, ready: true }),
    ).toEqual({ text: "running, ready", kind: "success" });
  });

  it("reports a waiting container as warning, using the wait reason", () => {
    expect(containerStateText({ state: { waiting: { reason: "ContainerCreating" } } })).toEqual({
      text: "waiting - ContainerCreating",
      kind: "warning",
    });
  });

  it("falls back to a bare 'waiting' reason when none is given", () => {
    expect(containerStateText({ state: { waiting: {} } })).toEqual({
      text: "waiting - waiting",
      kind: "warning",
    });
  });

  it("reports a CrashLoopBackOff-style waiting reason as danger", () => {
    expect(containerStateText({ state: { waiting: { reason: "CrashLoopBackOff" } } })).toEqual({
      text: "waiting - CrashLoopBackOff",
      kind: "danger",
    });
  });

  it("reports a terminated container with a Completed reason as neutral", () => {
    expect(
      containerStateText({ state: { terminated: { reason: "Completed", exitCode: 0 } } }),
    ).toEqual({ text: "terminated - Completed (exit code: 0)", kind: "neutral" });
  });

  it("reports a terminated container with a non-Completed reason as danger, appends ready", () => {
    expect(
      containerStateText({
        state: { terminated: { reason: "Error", exitCode: 1 } },
        ready: true,
      }),
    ).toEqual({ text: "terminated, ready - Error (exit code: 1)", kind: "danger" });
  });

  it("omits the exit code segment when exitCode is unset", () => {
    expect(containerStateText({ state: { terminated: { reason: "OOMKilled" } } })).toEqual({
      text: "terminated - OOMKilled",
      kind: "danger",
    });
  });

  it("falls back to a bare 'terminated' reason when none is given", () => {
    expect(containerStateText({ state: { terminated: {} } })).toEqual({
      text: "terminated - terminated",
      kind: "danger",
    });
  });

  it("returns a dash with neutral kind when the state has none of running/waiting/terminated", () => {
    expect(containerStateText({ state: {} })).toEqual({ text: "—", kind: "neutral" });
    expect(containerStateText({})).toEqual({ text: "—", kind: "neutral" });
  });
});

describe("phaseKind", () => {
  it("calls the three settled-and-well phases success", () => {
    expect(phaseKind("Running")).toBe("success");
    expect(phaseKind("Succeeded")).toBe("success");
    expect(phaseKind("Ready")).toBe("success");
  });

  it("calls a Pending phase a warning — on its way, not yet wrong", () => {
    expect(phaseKind("Pending")).toBe("warning");
  });

  it("calls the failed, unknown and not-ready phases danger", () => {
    expect(phaseKind("Failed")).toBe("danger");
    expect(phaseKind("Unknown")).toBe("danger");
    expect(phaseKind("NotReady")).toBe("danger");
  });

  it("leaves any word it does not recognise neutral rather than guessing a tone", () => {
    expect(phaseKind("CrashLoopBackOff")).toBe("neutral");
    expect(phaseKind("")).toBe("neutral");
  });
});

describe("waitingKind", () => {
  it("calls any back-off a failure — the kubelet has already tried and stopped", () => {
    expect(waitingKind("CrashLoopBackOff")).toBe("danger");
    expect(waitingKind("ImagePullBackOff")).toBe("danger");
  });

  it("calls a container still on its way up a warning", () => {
    expect(waitingKind("ContainerCreating")).toBe("warning");
    expect(waitingKind("PodInitializing")).toBe("warning");
    expect(waitingKind("CreateContainerConfigError")).toBe("warning");
    expect(waitingKind("")).toBe("warning");
  });
});
