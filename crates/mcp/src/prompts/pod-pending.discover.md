---
name: pod-pending
description: Find the pods that will not schedule, then triage them
mode: discover
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: "", description: "limit to one namespace; omit to search all" }
---
No pod was named, so find what is failing to schedule on context `{{context}}`,
then explain why.

1. Call `k8s.listPods` with `context: {{context}}`, `namespace: {{namespace}}`.
   An empty namespace lists across all namespaces. Collect every pod whose
   `phase` is `Pending`, then split them by `node`. A pod with an EMPTY `node`
   is genuinely unscheduled — the scheduler has not placed it, and the
   predicate-based triage in steps 2-4 applies. A pod with a NON-EMPTY `node`
   already has one: `Pending` there is not a scheduling failure, it means
   something else is blocking readiness on the node it was already assigned —
   an image pull, an init container, a volume mount, or sandbox creation.
   `Pending` is not a synonym for `FailedScheduling`, so do not run these
   already-scheduled pods through the predicate steps below. Instead, for
   each, Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Pod`, `objectName: <the pod>`,
   `namespace: <the pod's namespace>`. Look for `Pulling`/`Failed`/`BackOff`
   events (image pull), an `Init` container status, or a mount/volume error,
   and report that instead of a scheduling predicate.
2. Among the genuinely unscheduled pods, if several are Pending, group them:
   pods blocked by the same cause usually share a namespace, a node selector,
   or a PersistentVolumeClaim. Triage one per group rather than all of them.
3. For each representative, Call `k8s.listEvents` with `context: {{context}}`,
   `objectKind: Pod`, `objectName: <the pod>`,
   `namespace: <the pod's namespace>`. `FailedScheduling` names the failed
   predicate. Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: <the pod's namespace>`, `name: <the pod>`. Read its
   `nodeSelector`, `affinity`, `tolerations` and `volumes`.
4. For a capacity or a label/taint predicate, first Call `k8s.listNodes` with
   `context: {{context}}` to enumerate candidate nodes — the per-node
   `getObject` calls below need a real node name to read, and `listNodes` is
   the only call that supplies one. Then, depending on the predicate:
   - for capacity: the `FailedScheduling` event from step 3 is already the
     scheduler's own authoritative statement of which resource is short and
     on how many nodes; treat it as the source of truth rather than trying to
     second-guess it by recomputing what the scheduler reserved. Kubernetes'
     exact reservation math — a per-resource maximum across init containers,
     restartable sidecar init containers accumulating with the app
     containers, and pod overhead — is not reproduced here. Call
     `k8s.getObject` with `context: {{context}}`, `kind: Node`,
     `name: <candidate>`. Read `status.allocatable`, and Call
     `k8s.nodeMetrics` with `context: {{context}}` to judge the SCALE of the
     shortfall and whether it is cluster-wide or limited to a handful of
     nodes;
   - for labels or taints: Call `k8s.getObject` with `context: {{context}}`,
     `kind: Node`, `name: <candidate>`. Read `metadata.labels` and
     `spec.taints` — `k8s.listNodes` returns neither, only a taint COUNT and
     no labels at all;
   - for volume binding: Call `k8s.listPersistentVolumeClaims` with
     `context: {{context}}`, `namespace: <the pod's namespace>`.
     `{{namespace}}` is the optional SEARCH filter from step 1 and defaults
     to `""`, which lists cluster-wide — use the pod's own namespace here
     instead, or every claim in the cluster gets scanned for one pod's
     problem. Call `k8s.listStorageClasses` with `context: {{context}}`;
5. ResourceQuota and LimitRange are not scheduling predicates and cannot be
   the cause here: both are enforced by an admission controller at CREATE
   time, so a pod already existing in `Pending` state already passed them —
   a violation would have been rejected before it was ever persisted, and
   quota exhaustion surfaces instead as a controller event (for example on a
   ReplicaSet) with no pod created, a different symptom from a Pending pod.
   Do not check quota or limit ranges for this flow.
6. If nothing is Pending, say so plainly.

Then report per group: the blocking reason, the evidence, and the minimal fix as a
`kubectl` command the user can review.

Do not call any tool that changes cluster state.
