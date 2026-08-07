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

1. Call `k8s.listNodes` with `context: {{context}}`. It reports readiness and
   schedulability, but not pressure conditions or capacity. Flag any node
   that is not `Ready` or is unschedulable as an immediate candidate — but do
   not stop there: a node under `MemoryPressure`, `DiskPressure` or
   `PIDPressure` can still report `Ready` and schedulable, so this list alone
   cannot rule a node out.
2. Call `k8s.nodeMetrics` with `context: {{context}}` for raw cpu and memory
   use per node. Raw usage cannot be used to shortlist which nodes to inspect
   next: in a heterogeneous cluster a nearly-exhausted small node can report
   lower raw usage than a large, healthy one, so ranking on the raw number
   before reading capacity would let the actually-pressured node fall out of
   consideration.
3. Pressure conditions live only on the Node object itself, and step 1's
   `Ready` status does not reveal them, so read every node's object BEFORE
   shortlisting: for each node returned in step 1, Call `k8s.getObject` with
   `context: {{context}}`, `kind: Node`, `name: <node>`. This is one call per
   node — on a large cluster, say plainly how many nodes were sampled rather
   than silently truncating the list. Read `status.conditions` for
   `MemoryPressure`, `DiskPressure` and `PIDPressure`, and `status.capacity` /
   `status.allocatable` to turn step 2's raw usage into a percentage of each
   node's own capacity.
4. Rank nodes by usage-as-a-fraction-of-capacity from step 3, not by step 2's
   raw usage, and combine that ranking with any pressure condition or
   not-Ready/unschedulable flag found in steps 1 and 3. Take the worst node.
5. Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Node`, `objectName: <node>`. Look for eviction and
   `SystemOOM` events.
6. Call `k8s.listPods` with `context: {{context}}`, `namespace: ""` and keep
   the pods scheduled on that node. `k8s.podMetrics` reports only
   `cpuMillicores` and `memoryMiB`, so a ranking is only possible for
   `MemoryPressure`. For `DiskPressure` or `PIDPressure`, no capability
   reports per-pod disk or PID-count usage, so the responsible workload
   cannot be determined this way — say that plainly instead of ranking by an
   unrelated number. For `MemoryPressure`: Call `k8s.podMetrics` with
   `context: {{context}}`. Rank those pods by memory use, then for the top
   consumers, Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: <the pod's namespace>`, `name: <the pod>`. Compare usage
   against `resources.requests` and `.limits`.
7. If no node is under pressure, say so and report the closest one to its limit.

Then report: which nodes are affected, which resource, the workloads responsible,
and the minimal fix as a `kubectl` command the user can review.

Do not call any tool that changes cluster state.
