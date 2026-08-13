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
endpoints, so nothing is receiving its traffic. Work out where the chain breaks:
type, then selector, then readiness, then port.

The evidence:

- Call `k8s.getObject` with `context: {{context}}`, `kind: Service`,
  `namespace: {{namespace}}`, `name: {{service}}`. Read `spec.type`,
  `spec.selector` and `spec.ports`.
- Call `k8s.listEndpointSlices` with `context: {{context}}`,
  `namespace: {{namespace}}`. Find the slices owned by `{{service}}`: empty
  slices mean no pod matched or none is ready; addresses marked not-ready
  mean the pods exist but are failing readiness.
- With a non-empty selector, Call `k8s.podsForSelector` with
  `context: {{context}}`, `namespace: {{namespace}}`,
  `selector: <the Service's spec.selector map>`. To chase a label mismatch,
  Call `k8s.listPods` with `context: {{context}}`, `namespace: {{namespace}}`
  and compare pod labels against the selector — a single mismatched key is
  the usual cause. To chase failing readiness, Call `k8s.getObject` with
  `context: {{context}}`, `kind: Pod`, `namespace: {{namespace}}`,
  `name: <the pod>` for its conditions and readiness probe. Call
  `k8s.podLogs` with `context: {{context}}`, `namespace: {{namespace}}`,
  `pod: <the pod>`, `tail_lines: 100`.

Pitfalls — each of these has produced a wrong diagnosis before:

- A `spec.type` of `ExternalName` has no selector and no EndpointSlices BY
  DESIGN — it resolves via a DNS CNAME, never through kube-proxy. No
  endpoints is correct there: say so and stop.
- A Service with no selector is fed by manually managed EndpointSlices.
  Never call `podsForSelector` with an empty selector — it always returns
  zero pods, and that empty result reads as a false label mismatch. Diagnose
  from the slice data instead.
- A correct selector with a wrong port still yields nothing usable: check
  that `spec.ports[].targetPort` matches a `containerPort` the pods actually
  expose.

Then tell the user which link in the chain is broken — selector, readiness, or port —
the evidence for it, and the minimal fix as a `kubectl` command they can review.

Do not call any tool that changes cluster state.
