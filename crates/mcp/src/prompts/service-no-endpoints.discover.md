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

1. Call `k8s.listServices` with `context: {{context}}` and `namespace: {{namespace}}` —
   an empty namespace lists across all namespaces.
2. Call `k8s.listEndpointSlices` for the same scope and match slices to services.
   Collect the services with no slices, with empty slices, or whose addresses are all
   marked not-ready. Ignore headless services with no selector unless they also have
   no manually managed slices.
3. Take up to three, worst first — a service backing an Ingress matters more than an
   unused one, so call `k8s.listIngresses` for the scope to see which are actually
   routed to.
4. For each, call `k8s.getObject` with `kind: Service` for `spec.selector` and
   `spec.ports`, then `k8s.podsForSelector` with that selector. No match means a
   selector/label mismatch — compare against `k8s.listPods`. A match that is not
   ready means a failing readiness probe: call `k8s.getObject` with `kind: Pod` and
   `k8s.podLogs` with `tailLines: 100` for one of them.
5. If every service has healthy endpoints, say so plainly.

Then report per service: which link is broken — selector, readiness, or target port —
the evidence, and the minimal fix as a `kubectl` command the user can review.

Do not call any tool that changes cluster state.
