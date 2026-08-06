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

1. Call `k8s.listNodes` for `context: {{context}}`. Flag every node whose conditions
   include `MemoryPressure`, `DiskPressure` or `PIDPressure`, plus any that is not
   `Ready` or is unschedulable.
2. Call `k8s.nodeMetrics` and rank all nodes by cpu and memory use against capacity.
   Include nodes near their limit even without a pressure condition — those are the
   ones about to become the problem.
3. Take the worst node. Call `k8s.listEvents` with `objectKind: Node` and its name
   for eviction and `SystemOOM` events.
4. Call `k8s.listPods`, keep the pods scheduled on that node, and call
   `k8s.podMetrics` to rank them by use of the pressured resource. For the top
   consumers, call `k8s.getObject` with `kind: Pod` to compare usage against
   `resources.requests` and `.limits`.
5. If no node is under pressure, say so and report the closest one to its limit.

Then report: which nodes are affected, which resource, the workloads responsible,
and the minimal fix as a `kubectl` command the user can review.

Do not call any tool that changes cluster state.
