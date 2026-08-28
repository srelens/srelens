// The external address of a Service, for the detail view (#264).
//
// The list gets this field from `k8s.listServices`, which computes it in Rust;
// the detail view renders the raw object, so the same rule lives here. It
// follows `kubectl get svc`'s EXTERNAL-IP column.

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function array(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/**
 * Where the service can be reached from outside the cluster.
 *
 * A LoadBalancer publishes ingress entries in `status`, each carrying an ip or
 * a hostname (AWS gives hostnames, most others ips); `spec.externalIPs` is a
 * separate, manually assigned set that any service type may carry, and both are
 * shown. An ExternalName has no address of its own — it resolves to the name it
 * points at, which is what kubectl prints here.
 *
 * A LoadBalancer with nothing yet is `<pending>` rather than empty: waiting on
 * a cloud provider and having no external address at all are different states,
 * and only the first resolves itself.
 */
export function serviceExternalAddress(service: unknown): string {
  const spec = record(record(service).spec);
  const type = text(spec.type) || "ClusterIP";
  if (type === "ExternalName") return text(spec.externalName);

  const ingress = array(record(record(record(service).status).loadBalancer).ingress)
    .map((entry) => text(record(entry).ip) || text(record(entry).hostname))
    .filter((address) => address !== "");
  const assigned = array(spec.externalIPs).map(text).filter((address) => address !== "");
  const addresses = [...ingress, ...assigned];

  if (addresses.length > 0) return addresses.join(", ");
  return type === "LoadBalancer" ? "<pending>" : "";
}
