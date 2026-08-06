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

1. Call `k8s.listEvents` with `objectKind: Pod`, `objectName: {{pod}}`,
   `namespace: {{namespace}}`. The scheduler explains itself here — `FailedScheduling`
   messages name the exact predicate that failed. Start with this, not the manifest.
2. Call `k8s.getObject` with `kind: Pod`, `namespace: {{namespace}}`, `name: {{pod}}`
   and read `spec.nodeSelector`, `spec.affinity`, `spec.tolerations`,
   `spec.resources.requests` and `spec.volumes`.
3. Match the event against the cause:
   - insufficient cpu/memory → call `k8s.listNodes` and `k8s.nodeMetrics` and compare
     the request against what nodes have free;
   - node selector or affinity mismatch → call `k8s.listNodes` and check labels;
   - taint tolerations → check node taints from the same node objects;
   - unbound PersistentVolumeClaim → call `k8s.listPersistentVolumeClaims` for the
     namespace and `k8s.listStorageClasses`, then `k8s.podsForPvc` if the claim is
     shared.
4. Also check `k8s.listResourceQuotas` and `k8s.listLimitRanges` for the namespace —
   a quota that is already at its ceiling blocks admission with no node in sight.

Then tell the user the single blocking reason, the evidence for it, and the minimal
fix as a `kubectl` command they can review.

Do not call any tool that changes cluster state.
