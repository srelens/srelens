export interface ResourceTarget {
  kind: string;
  namespace: string | null;
  name: string;
}

export type OpenResource = (target: ResourceTarget) => void;

const NAVIGABLE_KINDS = new Set([
  "ClusterRole",
  "ClusterRoleBinding",
  "ConfigMap",
  "CronJob",
  "DaemonSet",
  "Deployment",
  "Endpoints",
  "EndpointSlice",
  "HorizontalPodAutoscaler",
  "Ingress",
  "IngressClass",
  "Job",
  "Lease",
  "LimitRange",
  "MutatingWebhookConfiguration",
  "Namespace",
  "NetworkPolicy",
  "Node",
  "PersistentVolume",
  "PersistentVolumeClaim",
  "Pod",
  "PodDisruptionBudget",
  "PriorityClass",
  "ReplicaSet",
  "ResourceQuota",
  "Role",
  "RoleBinding",
  "RuntimeClass",
  "Secret",
  "Service",
  "ServiceAccount",
  "StatefulSet",
  "StorageClass",
  "ValidatingWebhookConfiguration",
]);

export function isNavigableResourceKind(kind: string): boolean {
  return NAVIGABLE_KINDS.has(kind);
}

const CLUSTER_SCOPED_KINDS = new Set([
  "ClusterRole",
  "ClusterRoleBinding",
  "IngressClass",
  "MutatingWebhookConfiguration",
  "Namespace",
  "Node",
  "PersistentVolume",
  "PriorityClass",
  "RuntimeClass",
  "StorageClass",
  "ValidatingWebhookConfiguration",
]);

/** True for kinds that exist outside any namespace (Node, ClusterRole, …). */
export function isClusterScopedKind(kind: string): boolean {
  return CLUSTER_SCOPED_KINDS.has(kind);
}

export function targetNamespace(kind: string, namespace: string | null): string | null {
  return CLUSTER_SCOPED_KINDS.has(kind) ? null : namespace;
}
