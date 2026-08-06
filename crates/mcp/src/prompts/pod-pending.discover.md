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

1. Call `k8s.listPods` with `context: {{context}}` and `namespace: {{namespace}}` —
   an empty namespace lists across all namespaces. Collect every pod whose `phase`
   is `Pending`.
2. If several are Pending, group them: pods blocked by the same cause usually share
   a namespace, a node selector, or a PersistentVolumeClaim. Triage one per group
   rather than all of them.
3. For each representative, call `k8s.listEvents` with `objectKind: Pod` and that
   pod's name — `FailedScheduling` names the failed predicate — then
   `k8s.getObject` with `kind: Pod` for its `nodeSelector`, `affinity`,
   `tolerations`, `resources.requests` and `volumes`.
4. Depending on the predicate, call `k8s.listNodes` and `k8s.nodeMetrics` for
   capacity and labels, `k8s.listPersistentVolumeClaims` and
   `k8s.listStorageClasses` for volume binding, or `k8s.listResourceQuotas` and
   `k8s.listLimitRanges` for admission limits.
5. If nothing is Pending, say so plainly.

Then report per group: the blocking reason, the evidence, and the minimal fix as a
`kubectl` command the user can review.

Do not call any tool that changes cluster state.
