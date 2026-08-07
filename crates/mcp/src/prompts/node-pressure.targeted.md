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

1. Call `k8s.getObject` with `context: {{context}}`, `kind: Node`,
   `name: {{node}}`. Read `status.conditions` for `MemoryPressure`,
   `DiskPressure` and `PIDPressure`, `status.capacity` and
   `status.allocatable` for what the node has to work with, and
   `spec.unschedulable` / `spec.taints` for whether it is schedulable.
   `k8s.listNodes` does not return any of this — only a name, a
   Ready/NotReady/Unknown status, and a taint COUNT. A node can be under
   pressure and still `Ready`.
2. Call `k8s.nodeMetrics` with `context: {{context}}`. Compare actual cpu and
   memory use against the capacity read in step 1. Pressure with usage still
   far below capacity points at the kubelet's eviction thresholds rather than
   at a genuinely busy node.
3. Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Node`,
   `objectName: {{node}}`. Eviction and `SystemOOM` events name what the kubelet
   already acted on.
4. Call `k8s.listPods` with `context: {{context}}`, `namespace: ""` and keep
   the pods whose `node` is `{{node}}` — an empty namespace lists
   cluster-wide. `k8s.podMetrics` reports only `cpuMillicores` and
   `memoryMiB`, so a ranking is only possible for `MemoryPressure`. For
   `DiskPressure` or `PIDPressure`, no capability reports per-pod disk or
   PID-count usage, so which workload is responsible cannot be determined
   this way — tell the user that plainly rather than ranking by an unrelated
   number. For `MemoryPressure`: Call `k8s.podMetrics` with
   `context: {{context}}` for their real usage and rank by memory
   consumption.
5. For the top consumers under `MemoryPressure`: Call `k8s.getObject` with
   `context: {{context}}`, `kind: Pod`, `namespace: <the pod's namespace>`,
   `name: <the pod>`. Compare usage against
   `spec.containers[].resources.requests` and `.limits`. A pod using far more
   than it requests is the one making the node's scheduling decisions wrong.

Then tell the user: which resource is under pressure, the workloads responsible,
which pods are at risk of eviction next, and the minimal fix as a `kubectl` command
they can review.

Do not call any tool that changes cluster state.
