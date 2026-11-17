import { isBuiltInKind } from "./crdGvk";

/**
 * Whether Kubernetes serves its own kinds in `group`: the core group, the four
 * named before the `*.k8s.io` convention, and `*.k8s.io`.
 *
 * A CRD in a `*.k8s.io` group that reuses a built-in kind's name reads as the
 * built-in here; a complete built-in kind-to-group table would settle that if
 * one ever lives in the frontend.
 */
function servesBuiltIns(group: string): boolean {
  return group === "" || ["apps", "batch", "autoscaling", "policy"].includes(group) || group.endsWith(".k8s.io");
}

/** The group an event's object needs in its route, from its `apiVersion`. */
export function eventRouteGroup(kind: string, apiVersion: string | undefined): string | undefined {
  const slash = apiVersion?.lastIndexOf("/") ?? -1;
  const group = apiVersion && slash > 0 ? apiVersion.slice(0, slash) : "";
  if (isBuiltInKind(kind) && servesBuiltIns(group)) return undefined;
  return group || undefined;
}
