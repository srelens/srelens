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

The evidence:

- Call `k8s.listPods` with `context: {{context}}`, `namespace: {{namespace}}`.
  An empty namespace lists across all namespaces.
- For each candidate, Call `k8s.getObject` with `context: {{context}}`,
  `kind: Pod`, `namespace: <the candidate's namespace>`,
  `name: <the candidate's name>`. This confirms whether it is failing NOW and
  which container is responsible.
- For a confirmed offender, Call `k8s.podLogs` with `context: {{context}}`,
  `namespace: <the candidate's namespace>`, `pod: <the candidate's name>`,
  `container: <the crashing container>`, `previous: true`, `tail_lines: 200`.
  Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Pod`,
  `objectName: <the candidate's name>`,
  `namespace: <the candidate's namespace>`.

Pitfalls — each of these has produced a wrong diagnosis before:

- `listPods`' `restarts` is a lifetime sum over app containers only. Use it
  as a cheap pre-filter, never as a ranking: a long-healthy pod keeps a large
  historical sum forever, and only the Pod object tells current from
  historical — via each container's `state`, not `lastState` or
  `restartCount`, which are retained history.
- A pod crash-looping in an INIT container reports ZERO restarts in
  `listPods` (init statuses are a separate array), so also shortlist pods
  whose `ready` count is below total with no restarts — that is where init
  loops hide, and neither status array excludes the other.
- Triage a handful of confirmed-current offenders rather than everything
  with history. A candidate with no current failure is an old incident:
  mention it and move on without reading its logs or events.
- If nothing is currently failing, say so plainly rather than inventing a
  problem.

Then report per pod: the most likely cause, the evidence for it, and the minimal
fix as a `kubectl` command the user can review. Finish with which pod to look at
first and why.

Do not call any tool that changes cluster state.
