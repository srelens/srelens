---
name: pod-pending
description: Find the pods that will not schedule, then triage them
mode: discover
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: "", description: "limit to one namespace; omit to search all" }
---
No pod was named, so find what is failing to schedule on context `{{context}}`,
then explain why.

The evidence:

- Call `k8s.listPods` with `context: {{context}}`, `namespace: {{namespace}}`.
  An empty namespace lists across all namespaces. The candidates are the pods
  whose `phase` is `Pending`; each one's `node` field splits the triage
  (first pitfall).
- Group the genuinely unscheduled pods by likely shared cause — same
  namespace, node selector, or claim — and for one representative per group:
  Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Pod`,
  `objectName: <the pod>`, `namespace: <the pod's namespace>`. Call
  `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
  `namespace: <the pod's namespace>`, `name: <the pod>`.
- For node-side facts, Call `k8s.listNodes` with `context: {{context}}` to
  enumerate real node names. Then Call `k8s.getObject` with
  `context: {{context}}`, `kind: Node`, `name: <candidate>` for
  `status.allocatable`, `metadata.labels`, and `spec.taints`. Call
  `k8s.nodeMetrics` with `context: {{context}}` to judge scale.
- For volume binding, Call `k8s.listPersistentVolumeClaims` with
  `context: {{context}}`, `namespace: <the pod's namespace>`. Use the pod's
  OWN namespace here, not the search filter — the filter defaults to
  cluster-wide, and every claim in the cluster should not be scanned for one
  pod's problem. Call `k8s.listStorageClasses` with `context: {{context}}`.

Pitfalls — each of these has produced a wrong diagnosis before:

- `Pending` is not a synonym for `FailedScheduling`. A pod with a NON-EMPTY
  `node` is already placed: something else is blocking readiness there — an
  image pull, an init container, a volume mount, or sandbox creation. Read
  its events and report that; the scheduling analysis applies only to pods
  with an empty `node`.
- The `FailedScheduling` event is the scheduler's own authoritative statement
  of what failed. Do not recompute its reservation math (per-resource init
  maxima, restartable sidecars, pod overhead); use capacity and metrics only
  to judge the SCALE of what it names.
- `listNodes` and `nodeMetrics` are summaries — no labels, no taint detail,
  no capacity. Per-node facts come only from the Node object.
- ResourceQuota and LimitRange are admission-time checks: a pod that EXISTS
  in `Pending` already passed them, so do not check them here.
- If nothing is Pending, say so plainly.

Then report per group: the blocking reason, the evidence, and the minimal fix as a
`kubectl` command the user can review.

Do not call any tool that changes cluster state.
