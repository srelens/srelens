---
name: service-no-endpoints
description: Find services with no endpoints, then triage them
mode: discover
priority: 0
arguments:
  - { name: context, required: true, description: kube context to triage }
  - { name: namespace, default: "", description: "limit to one namespace; omit to search all" }
---
No service was named, so find which services on context `{{context}}` have no
endpoints, then explain why.

The evidence:

- Call `k8s.listServices` with `context: {{context}}`,
  `namespace: {{namespace}}`. An empty namespace lists across all
  namespaces; note each service's `type`.
- Call `k8s.listEndpointSlices` with `context: {{context}}`,
  `namespace: {{namespace}}`. The broken candidates are services with no
  slices, empty slices, or all addresses not-ready.
- To rank candidates by blast radius, Call `k8s.listIngresses` with
  `context: {{context}}`, `namespace: {{namespace}}`. Then for each Ingress,
  Call `k8s.getObject` with `context: {{context}}`, `kind: Ingress`,
  `namespace: <the ingress's namespace>`, `name: <the ingress>` and read
  `spec.rules[].http.paths[].backend.service.name` and
  `spec.defaultBackend.service.name` — an Ingress-fronted broken service
  matters more than one nothing routes to.
- For a few of the worst candidates, diagnose exactly as the targeted flow
  does. Call `k8s.getObject` with `context: {{context}}`, `kind: Service`,
  `namespace: <the service's namespace>`, `name: <the service>`. With a
  non-empty selector, Call `k8s.podsForSelector` with `context: {{context}}`,
  `namespace: <the service's namespace>`,
  `selector: <the Service's spec.selector map>`. To chase a label mismatch,
  Call `k8s.listPods` with `context: {{context}}`,
  `namespace: <the service's namespace>`. To chase failing readiness, Call
  `k8s.getObject` with `context: {{context}}`, `kind: Pod`,
  `namespace: <the service's namespace>`, `name: <the pod>`. Call
  `k8s.podLogs` with `context: {{context}}`,
  `namespace: <the service's namespace>`, `pod: <the pod>`,
  `tail_lines: 100`.

Pitfalls — each of these has produced a wrong diagnosis before:

- `ExternalName` services have no selector and no EndpointSlices BY DESIGN
  (DNS CNAME, no kube-proxy) — exclude them from the candidate set; they are
  not broken.
- Selector-less services are fed by manually managed EndpointSlices:
  diagnose those from the slice data, and never call `podsForSelector` with
  an empty selector — it always returns zero pods, which reads as a false
  label mismatch.
- `IngressSummary` carries no backend reference — only the Ingress object
  does — and a backend name resolves within the Ingress's OWN namespace.
  Key the routed-to set by the (namespace, name) PAIR: in a cluster-wide
  search, same-named Services in different namespaces are different
  services.
- If every service has healthy endpoints, say so plainly.

Then report per service: which link is broken — selector, readiness, or target port —
the evidence, and the minimal fix as a `kubectl` command the user can review.

Do not call any tool that changes cluster state.
