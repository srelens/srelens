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

The evidence:

- Call `k8s.listNodes` with `context: {{context}}`. Not-`Ready` or
  unschedulable nodes are immediate candidates — but this list cannot rule a
  node OUT (first pitfall).
- Call `k8s.nodeMetrics` with `context: {{context}}` for raw cpu and memory
  use per node.
- For each node, Call `k8s.getObject` with `context: {{context}}`,
  `kind: Node`, `name: <node>`. Read `status.conditions` for the pressure
  flags and `status.capacity` / `status.allocatable` to turn raw use into a
  fraction of that node's own capacity. This is one call per node — on a
  large cluster, say plainly how many nodes were sampled rather than
  silently truncating.
- For the worst node: Call `k8s.listEvents` with `context: {{context}}`,
  `objectKind: Node`, `objectName: <node>`. Call `k8s.listPods` with
  `context: {{context}}`, `namespace: ""` and keep its pods. Under
  `MemoryPressure`, Call `k8s.podMetrics` with `context: {{context}}` and
  rank by memory use. For the top consumers, Call `k8s.getObject` with
  `context: {{context}}`, `kind: Pod`, `namespace: <the pod's namespace>`,
  `name: <the pod>` and compare use against `resources.requests` and
  `.limits`.

Pitfalls — each of these has produced a wrong diagnosis before:

- A node under `MemoryPressure`, `DiskPressure` or `PIDPressure` can still
  report `Ready` and schedulable, and `listNodes` carries no pressure or
  capacity detail — only the Node object does.
- Rank by usage as a fraction of each node's OWN capacity, never by raw
  usage: in a heterogeneous cluster a nearly-exhausted small node reports
  lower raw numbers than a large healthy one.
- `podMetrics` reports only `cpuMillicores` and `memoryMiB`, so the
  responsible workload can be ranked for `MemoryPressure` only; for disk or
  PID pressure, say plainly that no capability reports per-pod usage.
- If no node is under pressure, say so and report the closest one to its
  limit.

Then report: which nodes are affected, which resource, the workloads responsible,
and the minimal fix as a `kubectl` command the user can review.

Do not call any tool that changes cluster state.
