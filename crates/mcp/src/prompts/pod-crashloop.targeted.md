---
name: pod-crashloop
description: Work out why a pod keeps restarting
mode: targeted
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: default, description: namespace of the pod }
  - { name: pod, target: true, description: the pod that keeps restarting }
---
Pod `{{pod}}` in namespace `{{namespace}}` on context `{{context}}` is restarting
repeatedly. Work out why, using only srelens tools.

The evidence:

- Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
  `namespace: {{namespace}}`, `name: {{pod}}`. The container statuses say
  which container is failing right now, and its terminated detail (exit code,
  reason, message) says how it died — exit 137 WITH reason `OOMKilled` is a
  memory-limit kill (137 alone is just SIGKILL, which an external eviction or
  forced deletion also produces); exit 1 or 2 usually means the process
  failed and the logs say why.
- Call `k8s.podLogs` with `context: {{context}}`, `namespace: {{namespace}}`,
  `pod: {{pod}}`, `container: <the crashing container>`, `previous: true`,
  `tail_lines: 200`. The PREVIOUS instance's output is where the crash reason
  lives — the current instance may not have failed yet.
- Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Pod`,
  `objectName: {{pod}}`, `namespace: {{namespace}}`. Failing probes, image
  pull failures, and backoff history show up here.

Pitfalls — each of these has produced a wrong diagnosis before:

- `state` is now; `lastState` and lifetime `restartCount` are history, and
  history is RETAINED after a container recovers. Decide which container is
  currently failing from `state` alone; read `lastState.terminated` only for
  the crash detail of a container whose current `state` already shows it
  failing (a `CrashLoopBackOff` wait carries no exit code of its own).
- Init and app container statuses are separate arrays
  (`status.initContainerStatuses[]` / `status.containerStatuses[]`) and
  neither excludes the other — restartable init sidecars run alongside app
  containers. Check both, and when comparing an OOM kill against limits, read
  the matching spec array (`spec.initContainers[]` vs `spec.containers[]`)
  for the SAME container that is crashing.
- In a multi-container pod, always name the `container` when reading logs —
  otherwise a healthy container's logs come back while the crashing one goes
  unread.

Then tell the user, in this order: the single most likely cause, the evidence for
it, and the minimal fix stated as a `kubectl` command they can review.

Do not call any tool that changes cluster state. This is a read-only
investigation — the user decides whether to apply the fix.
