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

The evidence:

- Call `k8s.getObject` with `context: {{context}}`, `kind: Node`,
  `name: {{node}}`. Read `status.conditions` for `MemoryPressure`,
  `DiskPressure` and `PIDPressure`, `status.capacity` /
  `status.allocatable` for what the node has to work with, and
  `spec.unschedulable` / `spec.taints` for whether it is schedulable.
- Call `k8s.nodeMetrics` with `context: {{context}}`. Compare actual use
  against that capacity — pressure with usage still far below capacity
  points at the kubelet's eviction thresholds, not a genuinely busy node.
- Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Node`,
  `objectName: {{node}}`. Eviction and `SystemOOM` events name what the
  kubelet already acted on.
- Call `k8s.listPods` with `context: {{context}}`, `namespace: ""`. An empty
  namespace lists cluster-wide; keep the pods whose `node` is `{{node}}`.
  Under `MemoryPressure`, Call `k8s.podMetrics` with `context: {{context}}`
  and rank those pods by memory use. For the top consumers, Call
  `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
  `namespace: <the pod's namespace>`, `name: <the pod>` and compare use
  against `spec.containers[].resources.requests` and `.limits` — a pod using
  far more than it requests is what makes the node's scheduling decisions
  wrong.

Pitfalls — each of these has produced a wrong diagnosis before:

- `listNodes` is a summary — a name, a Ready status, a taint COUNT. Pressure
  conditions, capacity, and labels live only on the Node object, and a node
  can be under pressure while still `Ready`.
- `podMetrics` reports only `cpuMillicores` and `memoryMiB`, so a
  responsible-workload ranking exists for `MemoryPressure` only. For
  `DiskPressure` or `PIDPressure` no capability reports per-pod usage — say
  that plainly instead of ranking by an unrelated number.

Then tell the user: which resource is under pressure, the workloads responsible,
which pods are at risk of eviction next, and the minimal fix as a `kubectl` command
they can review.

Do not call any tool that changes cluster state.
