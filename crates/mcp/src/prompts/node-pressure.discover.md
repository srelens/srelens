---
name: node-pressure
description: Find nodes under resource pressure, then triage them
mode: discover
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
---
No node was named, so find which nodes on context `{{context}}` are under pressure,
then explain why.

1. Call `k8s.listNodes` with `context: {{context}}` to enumerate nodes — it reports
   readiness and schedulability, but not pressure conditions or capacity. Flag any
   node that is not `Ready` or is unschedulable as an immediate candidate.
2. Call `k8s.nodeMetrics` with `context: {{context}}` and rank all nodes by cpu and
   memory use. This is usage, not capacity, so it is a ranking signal, not a
   verdict on its own.
3. For the top few by rank plus any flagged in step 1, call `k8s.getObject` with
   `context: {{context}}`, `kind: Node`, `name: <candidate>` and read
   `status.conditions` for `MemoryPressure`, `DiskPressure` and `PIDPressure`, and
   `status.capacity` / `status.allocatable` to turn step 2's usage numbers into a
   percentage of capacity.
4. Take the worst node. Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Node`, `objectName: <node>` for eviction and `SystemOOM` events.
5. Call `k8s.listPods` with `context: {{context}}`, `namespace: ""`, keep the pods
   scheduled on that node, and call `k8s.podMetrics` with `context: {{context}}` to
   rank them by use of the pressured resource. For the top consumers, call
   `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: <the pod's namespace>`, `name: <the pod>` to compare usage against
   `resources.requests` and `.limits`.
6. If no node is under pressure, say so and report the closest one to its limit.

Then report: which nodes are affected, which resource, the workloads responsible,
and the minimal fix as a `kubectl` command the user can review.

Do not call any tool that changes cluster state.
