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

1. Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: {{namespace}}`, `name: {{pod}}`. Kubernetes records an init
   container's crash loop separately from the app containers: read
   `status.initContainerStatuses[]` AND `status.containerStatuses[]`
   independently — neither excludes the other. A classic init container that
   already ran to completion keeps a positive lifetime `restartCount` from an
   earlier failure forever, and a native restartable init sidecar
   (`spec.initContainers[]` whose `restartPolicy` is `Always`) keeps running
   alongside the app containers indefinitely — app containers do NOT
   necessarily wait for every init container to finish in that case. `state`
   is what is true right now; `lastState` is what happened before and is
   RETAINED even after the container recovers to `state.running` — so
   neither `lastState` nor lifetime `restartCount` may decide which container
   is currently failing. An entry in either array counts as a CURRENT crash
   source only if its own `state` shows a `waiting.reason` of
   `CrashLoopBackOff`, or a `state.terminated` with a non-zero `exitCode` and
   a `reason` other than `Completed`. Pool every entry across both arrays
   that meets this bar, then pick which one's logs to read: prefer whichever
   has a `waiting.reason` of `CrashLoopBackOff`, otherwise the one with the
   most recent `state.terminated.finishedAt` — a now-healthy container (init
   or app) with many past restarts is not the one to read, and in a
   multi-container pod the crashing one is not necessarily the first listed.
   Note whether the chosen container came from `initContainerStatuses` or
   `containerStatuses`; step 4 needs that to pick the matching resource spec.
   Read its crash detail — `state.terminated` if that is its current state,
   otherwise `lastState.terminated` (since `state.waiting` on
   `CrashLoopBackOff` carries no exit code of its own): `exitCode`, `reason`,
   `message`. Exit code 137 with reason `OOMKilled`
   means the container hit its memory limit. Exit code 1 or 2 usually means
   the process itself failed and the logs will say why. A `CrashLoopBackOff`
   waiting reason only tells you Kubernetes is backing off; the terminated
   block says what actually happened.
2. Call `k8s.podLogs` with `context: {{context}}`, `namespace: {{namespace}}`,
   `pod: {{pod}}`, `container: <the crashing container from step 1>`,
   `previous: true`, `tail_lines: 200`. The **previous** instance's output is
   where the crash reason is — the current instance may not have failed yet.
   Naming `container` matters in a multi-container pod: without it, this
   returns a healthy container's logs while the crashing one goes unread.
3. Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Pod`,
   `objectName: {{pod}}`, `namespace: {{namespace}}`. Look for failing probes,
   image pull failures, and repeated backoff events.
4. If step 1 showed an OOM kill, compare it against the resource limits of the
   SAME container that was chosen in step 1: read
   `spec.initContainers[].resources.limits.memory` if the crashing entry came
   from `initContainerStatuses`, otherwise
   `spec.containers[].resources.limits.memory` — against what the logs
   suggest the process needed.

Then tell the user, in this order: the single most likely cause, the evidence for
it, and the minimal fix stated as a `kubectl` command they can review.

Do not call any tool that changes cluster state. This is a read-only
investigation — the user decides whether to apply the fix.
