---
name: node-pressure
description: Triage a node reporting resource pressure
mode: targeted
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: node, target: true, description: the node under pressure }
---
Node `{{node}}` on context `{{context}}` is reporting pressure. Work out what is
consuming it and what is at risk.

1. Call `k8s.listNodes` for `context: {{context}}` and find `{{node}}`. Read its
   conditions — `MemoryPressure`, `DiskPressure`, `PIDPressure` — and whether it is
   `Ready` and schedulable. A node can be under pressure and still Ready.
2. Call `k8s.nodeMetrics` for actual cpu and memory use against capacity. Compare
   this with the conditions: pressure with low usage points at the kubelet's
   eviction thresholds rather than at a busy workload.
3. Call `k8s.listEvents` with `objectKind: Node`, `objectName: {{node}}`. Eviction
   and `SystemOOM` events name what the kubelet already acted on.
4. Call `k8s.listPods` and keep the pods whose `node` is `{{node}}`, then
   `k8s.podMetrics` for their real usage. Rank by consumption of whichever resource
   is under pressure.
5. For the top consumers, call `k8s.getObject` with `kind: Pod` and compare usage
   against `spec.containers[].resources.requests` and `.limits`. A pod using far
   more than it requests is the one making the node's scheduling decisions wrong.

Then tell the user: which resource is under pressure, the workloads responsible,
which pods are at risk of eviction next, and the minimal fix as a `kubectl` command
they can review.

Do not call any tool that changes cluster state.
