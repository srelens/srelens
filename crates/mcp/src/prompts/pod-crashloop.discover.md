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

1. Call `k8s.listPods` with `context: {{context}}`, `namespace: {{namespace}}`.
   An empty namespace lists across all namespaces. Each entry carries
   `restarts`, `phase`, `ready`, `name` and `namespace`.
2. Rank the candidates: highest `restarts` first, and treat any pod whose `ready`
   count is below its total as suspect even if `restarts` is low.
3. Take the top three at most. For each candidate, using its `name` and
   `namespace` from step 1: Call `k8s.getObject` with `context: {{context}}`,
   `kind: Pod`, `namespace: <the candidate's namespace>`,
   `name: <the candidate's name>`. In `status.containerStatuses[]`, find the
   container with the highest `restartCount` — call it the crashing container;
   in a multi-container pod it is not necessarily the first one listed — and
   read its `lastState.terminated` block (exit code and reason). Call
   `k8s.podLogs` with `context: {{context}}`,
   `namespace: <the candidate's namespace>`, `pod: <the candidate's name>`,
   `container: <the crashing container>`, `previous: true`,
   `tail_lines: 200`. Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Pod`, `objectName: <the candidate's name>`,
   `namespace: <the candidate's namespace>`.
4. If nothing has restarted, say so plainly rather than inventing a problem.

Then report per pod: the most likely cause, the evidence for it, and the minimal
fix as a `kubectl` command the user can review. Finish with which pod to look at
first and why.

Do not call any tool that changes cluster state.
