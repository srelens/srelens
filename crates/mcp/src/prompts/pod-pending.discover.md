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

1. Call `k8s.listPods` with `context: {{context}}`, `namespace: {{namespace}}`.
   An empty namespace lists across all namespaces. Collect every pod whose
   `phase` is `Pending`, then split them by `node`. A pod with an EMPTY `node`
   is genuinely unscheduled — the scheduler has not placed it, and the
   predicate-based triage in steps 2-4 applies. A pod with a NON-EMPTY `node`
   already has one: `Pending` there is not a scheduling failure, it means
   something else is blocking readiness on the node it was already assigned —
   an image pull, an init container, a volume mount, or sandbox creation.
   `Pending` is not a synonym for `FailedScheduling`, so do not run these
   already-scheduled pods through the predicate steps below. Instead, for
   each, Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Pod`, `objectName: <the pod>`,
   `namespace: <the pod's namespace>`. Look for `Pulling`/`Failed`/`BackOff`
   events (image pull), an `Init` container status, or a mount/volume error,
   and report that instead of a scheduling predicate.
2. Among the genuinely unscheduled pods, if several are Pending, group them:
   pods blocked by the same cause usually share a namespace, a node selector,
   or a PersistentVolumeClaim. Triage one per group rather than all of them.
3. For each representative, Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Pod`, `objectName: <the pod>`,
   `namespace: <the pod's namespace>`. `FailedScheduling` names the failed
   predicate. Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: <the pod's namespace>`, `name: <the pod>`. Read its
   `nodeSelector`, `affinity`, `tolerations`, `containers[].resources.requests`
   and `volumes`.
4. Depending on the predicate:
   - for capacity, labels or taints: Call `k8s.listNodes` with
     `context: {{context}}` to enumerate candidate nodes. Call
     `k8s.getObject` with `context: {{context}}`, `kind: Node`,
     `name: <candidate>`. Read `metadata.labels`, `spec.taints` and
     `status.allocatable` — `k8s.listNodes` returns none of those, only a
     taint COUNT and no labels or capacity at all. Reason about a node's free
     capacity as `allocatable` minus the requests of the pods already on it:
     Call `k8s.listPods` with `context: {{context}}`, `namespace: ""` to find
     the pods on that node. For each, Call `k8s.getObject` with
     `context: {{context}}`, `kind: Pod`, `namespace: <that pod's namespace>`,
     `name: <that pod's name>` and sum `spec.containers[].resources.requests`
     — no capability reports free capacity directly;
   - for volume binding: Call `k8s.listPersistentVolumeClaims` with
     `context: {{context}}`, `namespace: {{namespace}}`. Call
     `k8s.listStorageClasses` with `context: {{context}}`;
   - for admission limits: Call `k8s.listResourceQuotas` with
     `context: {{context}}`, `namespace: {{namespace}}`. Call
     `k8s.listLimitRanges` with `context: {{context}}`,
     `namespace: {{namespace}}`.
5. If nothing is Pending, say so plainly.

Then report per group: the blocking reason, the evidence, and the minimal fix as a
`kubectl` command the user can review.

Do not call any tool that changes cluster state.
