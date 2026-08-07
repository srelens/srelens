---
name: pod-pending
description: Work out why a pod will not schedule
mode: targeted
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: default, description: namespace of the pod }
  - { name: pod, target: true, description: the pod stuck in Pending }
---
Pod `{{pod}}` in namespace `{{namespace}}` on context `{{context}}` is stuck in
Pending. Work out what is blocking it.

1. Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: {{namespace}}`, `name: {{pod}}`. Read `spec.nodeName` first.
   - If `spec.nodeName` is NON-EMPTY, the scheduler has already placed this
     pod: `Pending` here is not a scheduling failure, it means something else
     is blocking readiness on the node it was already assigned — an image
     pull, an init container, a volume mount, or sandbox creation. `Pending`
     is not a synonym for `FailedScheduling`, so do not run this pod through
     the predicate steps below. Instead, Call `k8s.listEvents` with
     `context: {{context}}`, `objectKind: Pod`, `objectName: {{pod}}`,
     `namespace: {{namespace}}`. Look for `Pulling`/`Failed`/`BackOff` events
     (image pull), an `Init` container status, or a mount/volume error, and
     report that instead of a scheduling predicate. Stop here.
   - If `spec.nodeName` is EMPTY, the pod is genuinely unscheduled. Read
     `spec.nodeSelector`, `spec.affinity`, `spec.tolerations` and
     `spec.volumes`, then continue with the predicate-based triage below.
2. Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Pod`,
   `objectName: {{pod}}`, `namespace: {{namespace}}`. The scheduler explains itself
   here — `FailedScheduling` messages name the exact predicate that failed. Start
   with this, not the manifest.
3. Match the event against the cause. None of this is available from
   `k8s.listNodes` or `k8s.nodeMetrics` alone — they report neither labels,
   taint detail, nor capacity — so the pattern below is always: Call
   `k8s.listNodes` with `context: {{context}}` to enumerate candidate nodes.
   Then, Call `k8s.getObject` with `context: {{context}}`, `kind: Node`,
   `name: <candidate>`. Read whichever field the cause below needs —
   `status.allocatable`, `metadata.labels`, or `spec.taints`.
   - insufficient cpu/memory → the `FailedScheduling` event from step 2 is
     already the scheduler's own authoritative statement of which resource is
     short and on how many nodes; treat it as the source of truth rather than
     trying to second-guess it by recomputing what the scheduler reserved.
     Kubernetes' exact reservation math — a per-resource maximum across init
     containers, restartable sidecar init containers accumulating with the
     app containers, and pod overhead — is not reproduced here. Read
     `status.allocatable` from the `getObject` call above, and Call
     `k8s.nodeMetrics` with `context: {{context}}` to judge the SCALE of the
     shortfall and whether it is cluster-wide or limited to a handful of
     nodes;
   - node selector or affinity mismatch → read `metadata.labels` from the
     `getObject` call above and compare against the pod's `spec.nodeSelector`
     / `spec.affinity`;
   - taint tolerations → read `spec.taints` (key, value, effect) from the same
     `getObject` call above and compare against the pod's `spec.tolerations`;
   - unbound PersistentVolumeClaim → Call `k8s.listPersistentVolumeClaims`
     with `context: {{context}}`, `namespace: {{namespace}}`. Call
     `k8s.listStorageClasses` with `context: {{context}}`. If the claim is
     shared, Call `k8s.podsForPvc` with `context: {{context}}`,
     `namespace: {{namespace}}`, `pvc: <the claim's name>`.
4. ResourceQuota and LimitRange are not a scheduling blocker for a pod that
   already exists in `Pending` state: both are enforced by an admission
   controller at CREATE time, so a pod that violated either would have been
   rejected and would never exist to triage here — a quota violation surfaces
   instead as a controller event (for example on a ReplicaSet) with no pod
   ever created, which is a different symptom from a Pending pod. Do not
   check quota or limit ranges for this flow.

Then tell the user the single blocking reason, the evidence for it, and the minimal
fix as a `kubectl` command they can review.

Do not call any tool that changes cluster state.
