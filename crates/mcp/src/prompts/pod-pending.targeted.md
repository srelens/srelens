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
Pending, which means the scheduler has not placed it. Work out what is blocking it.

1. Call `k8s.listEvents` with `context: {{context}}`, `objectKind: Pod`,
   `objectName: {{pod}}`, `namespace: {{namespace}}`. The scheduler explains itself
   here — `FailedScheduling` messages name the exact predicate that failed. Start
   with this, not the manifest.
2. Call `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
   `namespace: {{namespace}}`, `name: {{pod}}`. Read `spec.nodeSelector`,
   `spec.affinity`, `spec.tolerations`, `spec.containers[].resources.requests`
   and `spec.volumes`.
3. Match the event against the cause. None of this is available from
   `k8s.listNodes` or `k8s.nodeMetrics` alone — they report neither labels,
   taint detail, nor capacity — so the pattern below is always: Call
   `k8s.listNodes` with `context: {{context}}` to enumerate candidate nodes.
   Then, Call `k8s.getObject` with `context: {{context}}`, `kind: Node`,
   `name: <candidate>`. Read whichever field the cause below needs —
   `status.allocatable`, `metadata.labels`, or `spec.taints`.
   - insufficient cpu/memory → read `status.allocatable` from the `getObject`
     call above. Since no capability reports a node's FREE capacity directly,
     reason it out: Call `k8s.listPods` with `context: {{context}}`,
     `namespace: ""` to find the pods already on that node. For each, Call
     `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
     `namespace: <that pod's namespace>`, `name: <that pod's name>`. Sum
     `spec.containers[].resources.requests`, and subtract that sum from
     `allocatable`;
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
4. Also Call `k8s.listResourceQuotas` with `context: {{context}}`,
   `namespace: {{namespace}}`. Call `k8s.listLimitRanges` with
   `context: {{context}}`, `namespace: {{namespace}}`. A quota that is
   already at its ceiling blocks admission with no node in sight.

Then tell the user the single blocking reason, the evidence for it, and the minimal
fix as a `kubectl` command they can review.

Do not call any tool that changes cluster state.
