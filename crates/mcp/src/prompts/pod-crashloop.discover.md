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
2. Rank the candidates: highest `restarts` first among pods with
   `restarts > 0` — these are confirmed crash loops. Separately, note every
   pod whose `ready` count is below its total but `restarts` is exactly zero:
   `restarts` sums only `status.containerStatuses[]`, and Kubernetes records
   an init container's crash loop under `status.initContainerStatuses[]`
   instead, so a pod repeatedly failing an init container reports zero
   restarts here even though it IS crash-looping. Do not assume such a pod is
   only an image pull or a slow readiness probe — step 3 confirms which it
   is. Append these zero-restart, not-fully-ready pods to the candidate list
   AFTER the ones with `restarts > 0`, still subject to the three-candidate
   cap below.
3. Take the top three at most. For each candidate, using its `name` and
   `namespace` from step 1: Call `k8s.getObject` with `context: {{context}}`,
   `kind: Pod`, `namespace: <the candidate's namespace>`,
   `name: <the candidate's name>`. Check `status.initContainerStatuses[]`
   FIRST: if any entry there has a `waiting.reason` of `CrashLoopBackOff` or a
   `restartCount` above zero, an init container is failing — app containers
   never even start until every init container succeeds, so this is the
   pod's real crash loop regardless of what `status.containerStatuses[]` or
   step 1's `restarts` showed. If a zero-restart candidate from step 2 shows
   no such entry, it is genuinely out of scope here (an image pull or a
   readiness stall, not a crash loop) — mention it to the user separately and
   move on without calling `podLogs` or `listEvents` for it. Otherwise, pick
   the crashing container (init or app) by its CURRENT state, not by the
   largest lifetime `restartCount`: prefer whichever container has a
   `waiting.reason` of `CrashLoopBackOff`, falling back to the one with the
   most recent `lastState.terminated` if none is currently in back-off — a
   now-healthy container with many past restarts is not the one to read.
   Read its `lastState.terminated` block (exit code and reason). Call
   `k8s.podLogs` with `context: {{context}}`,
   `namespace: <the candidate's namespace>`, `pod: <the candidate's name>`,
   `container: <the crashing container>`, `previous: true`,
   `tail_lines: 200`. Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Pod`, `objectName: <the candidate's name>`,
   `namespace: <the candidate's namespace>`.
4. If nothing has restarted and no init container is crash-looping, say so
   plainly rather than inventing a problem.

Then report per pod: the most likely cause, the evidence for it, and the minimal
fix as a `kubectl` command the user can review. Finish with which pod to look at
first and why.

Do not call any tool that changes cluster state.
