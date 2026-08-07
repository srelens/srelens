---
name: service-no-endpoints
description: Work out why a service has no endpoints
mode: targeted
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: default, description: namespace of the service }
  - { name: service, target: true, description: the service with no endpoints }
---
Service `{{service}}` in namespace `{{namespace}}` on context `{{context}}` has no
endpoints, so nothing is receiving its traffic. Work out where the chain breaks.

1. Call `k8s.getObject` with `context: {{context}}`, `kind: Service`,
   `namespace: {{namespace}}`, `name: {{service}}`. Read `spec.selector`,
   `spec.ports` and `spec.type`.
   - If `spec.type` is `ExternalName`, this Service intentionally has no
     selector and no EndpointSlices at all — Kubernetes resolves it via a DNS
     CNAME to `spec.externalName` instead of routing through kube-proxy.
     Having no endpoints is correct by design here, not a fault. Say that and
     stop; do not run steps 2-4.
   - Otherwise, a Service with no selector is fed by a manually managed
     EndpointSlice — if so, say that and check the slices directly in step 2,
     and skip step 3 entirely: do NOT call `k8s.podsForSelector` with an empty
     selector, since it always returns zero pods and that empty result would
     misread as a selector/label mismatch rather than "manually managed."
2. Call `k8s.listEndpointSlices` with `context: {{context}}`,
   `namespace: {{namespace}}`. Find the slices owned by `{{service}}`. Empty
   slices mean no pod matched or none is ready; slices with addresses marked
   not-ready mean the pods exist but are failing readiness.
3. If `spec.selector` from step 1 is non-empty, Call `k8s.podsForSelector` with
   `context: {{context}}`, `namespace: {{namespace}}`,
   `selector: <the Service's spec.selector map>`.
   - No pods matched → the selector does not match any pod's labels. Call
     `k8s.listPods` with `context: {{context}}`, `namespace: {{namespace}}`
     and compare labels with the selector; a single mismatched key is the
     usual cause.
   - Pods matched but are not ready → their readiness probe is failing. For
     one of them, Call `k8s.getObject` with `context: {{context}}`,
     `kind: Pod`, `namespace: {{namespace}}`, `name: <the pod>`. Read
     `status.conditions` and `spec.containers[].readinessProbe`. Call
     `k8s.podLogs` with `context: {{context}}`, `namespace: {{namespace}}`,
     `pod: <the pod>`, `tail_lines: 100`.
4. Check that the Service's `spec.ports[].targetPort` matches a `containerPort` the
   pods actually expose — a correct selector with the wrong target port still yields
   no usable endpoint.

Then tell the user which link in the chain is broken — selector, readiness, or port —
the evidence for it, and the minimal fix as a `kubectl` command they can review.

Do not call any tool that changes cluster state.
