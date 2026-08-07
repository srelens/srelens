---
name: pod-crashloop
description: Find the pods that keep restarting, then triage them
mode: discover
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: "", description: "limit to one namespace; omit to search all" }
---
No pod was named, so find the restarting pods on context `{{context}}` first, then
triage them. Use only srelens tools.

1. Call `k8s.listPods` with `context: {{context}}` and `namespace: {{namespace}}` —
   an empty namespace lists across all namespaces. Each entry carries `restarts`,
   `phase` and `ready`.
2. Rank the candidates: highest `restarts` first, and treat any pod whose `ready`
   count is below its total as suspect even if `restarts` is low.
3. Take the top three at most. For each, call `k8s.getObject` with
   `context: {{context}}`, `kind: Pod` to read
   `status.containerStatuses[].lastState.terminated` (exit code and reason), then
   `k8s.podLogs` with `context: {{context}}`, `previous: true` and
   `tail_lines: 200`, then `k8s.listEvents` with `context: {{context}}`, filtered
   with `objectKind: Pod` and that pod's name.
4. If nothing has restarted, say so plainly rather than inventing a problem.

Then report per pod: the most likely cause, the evidence for it, and the minimal
fix as a `kubectl` command the user can review. Finish with which pod to look at
first and why.

Do not call any tool that changes cluster state.
