---
name: pod-pending
description: Work out why a pod will not schedule
mode: targeted
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: default, description: namespace of the pod }
  - { name: pod, target: true, description: the pod stuck in Pending }
---
Pod `{{pod}}` in namespace `{{namespace}}` on context `{{context}}` is stuck in
Pending. Work out what is blocking it.

The evidence:

- Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
  `namespace: {{namespace}}`, `name: {{pod}}`. `spec.nodeName` decides the
  shape of the whole triage (first pitfall); `spec.nodeSelector`,
  `spec.affinity`, `spec.tolerations` and `spec.volumes` are the pod's side
  of any scheduling comparison.
- Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Pod`,
  `objectName: {{pod}}`, `namespace: {{namespace}}`. Start here — the
  scheduler explains itself in `FailedScheduling` messages, and image pull,
  init, and mount problems surface here too.
- For node-side facts, Call `k8s.listNodes` with `context: {{context}}` to
  enumerate real node names. Then Call `k8s.getObject` with
  `context: {{context}}`, `kind: Node`, `name: <candidate>` and read
  whichever the cause needs: `status.allocatable`, `metadata.labels`, or
  `spec.taints`. Call `k8s.nodeMetrics` with `context: {{context}}` to judge
  the scale of a capacity shortfall — cluster-wide or a handful of nodes.
- For volume binding, Call `k8s.listPersistentVolumeClaims` with
  `context: {{context}}`, `namespace: {{namespace}}`. Call
  `k8s.listStorageClasses` with `context: {{context}}`. For a shared claim,
  Call `k8s.podsForPvc` with `context: {{context}}`,
  `namespace: {{namespace}}`, `pvc: <the claim's name>`.

Pitfalls — each of these has produced a wrong diagnosis before:

- `Pending` is not a synonym for `FailedScheduling`. A NON-EMPTY
  `spec.nodeName` means the scheduler already placed this pod and something
  else is blocking readiness — an image pull, an init container, a volume
  mount, or sandbox creation. Read the events, report that, and stop;
  scheduling analysis does not apply to a placed pod.
- The `FailedScheduling` event is the scheduler's own authoritative statement
  of what failed and on how many nodes. Do not recompute its reservation math
  (per-resource init maxima, restartable sidecars, pod overhead — none of it
  is reproduced here); use capacity and metrics only to judge the SCALE of
  what the event already names.
- `listNodes` and `nodeMetrics` are summaries: no labels, no taint detail,
  no capacity. Per-node facts come only from the Node object.
- ResourceQuota and LimitRange are admission-time checks: a pod that EXISTS
  in `Pending` already passed them, so do not check them here — a quota
  violation surfaces as a controller event with no pod created at all.

Then tell the user the single blocking reason, the evidence for it, and the minimal
fix as a `kubectl` command they can review.

Do not call any tool that changes cluster state.
