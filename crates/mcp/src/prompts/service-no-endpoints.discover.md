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

1. Call `k8s.listServices` with `context: {{context}}`,
   `namespace: {{namespace}}`. An empty namespace lists across all namespaces.
   Note each service's `type`.
2. Call `k8s.listEndpointSlices` with `context: {{context}}`,
   `namespace: {{namespace}}`. Match slices to services. Collect the services
   with no slices, with empty slices, or whose addresses are all marked not-ready.
   Ignore headless services with no selector unless they also have no manually
   managed slices. Exclude any service whose `type` (from step 1) is
   `ExternalName` from this candidate set entirely — it intentionally has no
   selector and no EndpointSlices, resolved instead via a DNS CNAME, so having
   none is correct by design, not a fault this flow should surface. The
   ready count itself can UNDER-report: it treats an endpoint whose
   `conditions.ready` field is absent as not-ready, though Kubernetes
   conventionally treats an absent `conditions.ready` as ready — a manually
   managed EndpointSlice that omits the field can show `0/N` here while its
   endpoints are actually fine. Before concluding a service in this candidate
   set is genuinely broken, especially one fed by a manually managed slice,
   confirm against the slice objects themselves (or the service's actual
   reachability) rather than trusting the ready count alone.
3. Call `k8s.listIngresses` with `context: {{context}}`, `namespace: {{namespace}}`
   to enumerate Ingresses — but `IngressSummary` carries only name, namespace,
   class, hosts, address, ports and age, no backend service reference, so this
   alone cannot say which Service an Ingress actually fronts. For each Ingress
   returned, Call `k8s.getObject` with `context: {{context}}`, `kind: Ingress`,
   `namespace: <the ingress's namespace>`, `name: <the ingress>`. Read
   `spec.rules[].http.paths[].backend.service.name` and
   `spec.defaultBackend.service.name` to build the routed-to set — but a
   backend name is only ever resolved within its own Ingress's namespace, so
   key each entry by the PAIR (the ingress's namespace, the backend service
   name), never by name alone: in a cluster-wide search two different
   namespaces can hold same-named Services, and keying on name only would
   treat both as Ingress-backed.
4. Take up to three of the broken services from step 2, worst first — a
   service whose (namespace, name) pair is in step 3's routed-to set matters
   more than one that is not.
5. For each, Call `k8s.getObject` with `context: {{context}}`,
   `kind: Service`, `namespace: <the service's namespace>`,
   `name: <the service>`. Read `spec.selector` and `spec.ports`. If
   `spec.selector` is empty or absent, this Service is fed by a manually
   managed EndpointSlice, not a selector — stop here and diagnose it from
   step 2's slice data rather than continuing on; do NOT call
   `k8s.podsForSelector` with an empty selector, since it always returns zero
   pods and that empty result would misread as a selector/label mismatch.
   Otherwise, Call `k8s.podsForSelector` with `context: {{context}}`,
   `namespace: <the service's namespace>`,
   `selector: <the Service's spec.selector map>`. No match means a
   selector/label mismatch: Call `k8s.listPods` with `context: {{context}}`,
   `namespace: <the service's namespace>` and compare pod labels against the
   selector. A match that is not ready means a failing readiness probe: for
   one of the matched pods, Call `k8s.getObject` with `context: {{context}}`,
   `kind: Pod`, `namespace: <the service's namespace>`, `name: <the pod>`.
   Call `k8s.podLogs` with `context: {{context}}`,
   `namespace: <the service's namespace>`, `pod: <the pod>`,
   `tail_lines: 100`.
6. If every service has healthy endpoints, say so plainly.

Then report per service: which link is broken — selector, readiness, or target port —
the evidence, and the minimal fix as a `kubectl` command the user can review.

Do not call any tool that changes cluster state.
