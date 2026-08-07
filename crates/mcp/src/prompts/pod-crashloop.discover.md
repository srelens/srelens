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
2. Use `restarts` only as a cheap PRE-FILTER to narrow the field — it sums
   lifetime `status.containerStatuses[]` counts and does not say which pod is
   failing right now. Shortlist every pod with `restarts > 0`, plus every pod
   whose `ready` count is below its total but `restarts` is exactly zero:
   `restarts` sums only `status.containerStatuses[]`, and Kubernetes records
   an init container's crash loop under `status.initContainerStatuses[]`
   instead, so a pod repeatedly failing an init container reports zero
   restarts here even though it IS crash-looping. Do not assume such a pod is
   only an image pull or a slow readiness probe — step 3 confirms which it
   is. Do not rank or cap this shortlist yet: a handful of long-healthy pods
   with large historical `restarts` sums would outrank a pod that is
   currently crash-looping with a smaller lifetime sum, and a high lifetime
   count on an otherwise-healthy pod is an old incident, not a finding — only
   reading the Pod object in step 3 can tell current from historical.
3. For each shortlisted candidate, using its `name` and `namespace` from step
   1: Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: <the candidate's namespace>`, `name: <the candidate's name>`.
   Read `status.initContainerStatuses[]` AND `status.containerStatuses[]`
   independently — neither excludes the other. A classic init container that
   already ran to completion keeps a positive lifetime `restartCount` from an
   earlier failure forever, and a native restartable init sidecar
   (`spec.initContainers[]` whose `restartPolicy` is `Always`) keeps running
   alongside the app containers indefinitely — app containers do NOT
   necessarily wait for every init container to finish in that case. `state`
   is what is true right now; `lastState` is what happened before and is
   RETAINED even after the container recovers to `state.running` — so
   neither `lastState` nor lifetime `restartCount` may decide which pod or
   container is currently failing. An entry in either array counts as a
   CURRENT crash source only if its own `state` shows a `waiting.reason` of
   `CrashLoopBackOff`, or a `state.terminated` with a non-zero `exitCode` and
   a `reason` other than `Completed`. Discard any shortlisted candidate with
   no such entry in either array — for a zero-restart candidate from step 2
   this means it is genuinely out of scope here (an image pull or a
   readiness stall, not a crash loop); for a `restarts > 0` candidate this
   means its high lifetime count is an old incident, not a finding — mention
   it to the user separately and move on without calling `podLogs` or
   `listEvents` for it. Order the pods that remain worst first —
   `CrashLoopBackOff` in `state.waiting` first, then the most recent
   `state.terminated.finishedAt`, lifetime `restartCount` only as a final
   tie-break — and take the top three. For each: pool every
   current-crash-source entry across both arrays and pick the one to read by
   that same order: `CrashLoopBackOff` first, otherwise the most recent
   `state.terminated.finishedAt`. Read its crash detail — `state.terminated`
   if that is its current state, otherwise `lastState.terminated` (since
   `state.waiting` on `CrashLoopBackOff` carries no exit code of its own).
   Call `k8s.podLogs` with `context: {{context}}`,
   `namespace: <the candidate's namespace>`, `pod: <the candidate's name>`,
   `container: <the crashing container>`, `previous: true`,
   `tail_lines: 200`. Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Pod`, `objectName: <the candidate's name>`,
   `namespace: <the candidate's namespace>`.
4. If nothing shortlisted in step 2, or nothing in the shortlist shows a
   current crash source in step 3, say so plainly rather than inventing a
   problem.

Then report per pod: the most likely cause, the evidence for it, and the minimal
fix as a `kubectl` command the user can review. Finish with which pod to look at
first and why.

Do not call any tool that changes cluster state.
