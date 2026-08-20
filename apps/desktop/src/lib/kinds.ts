/**
 * The resource kinds the app can browse, and their Kubernetes and display
 * names. Pure data: it lived in `ResourceBrowser` for historical reasons,
 * which made `lib/paletteActions.ts` import a UI component to read a table.
 */

export type ResourceKind =
  | "overview"
  | "pods"
  | "deployments"
  | "statefulsets"
  | "daemonsets"
  | "replicasets"
  | "jobs"
  | "cronjobs"
  | "configmaps"
  | "secrets"
  | "resourcequotas"
  | "limitranges"
  | "horizontalpodautoscalers"
  | "poddisruptionbudgets"
  | "priorityclasses"
  | "runtimeclasses"
  | "leases"
  | "mutatingwebhookconfigurations"
  | "validatingwebhookconfigurations"
  | "serviceaccounts"
  | "clusterroles"
  | "roles"
  | "clusterrolebindings"
  | "rolebindings"
  | "services"
  | "endpoints"
  | "endpointslices"
  | "ingresses"
  | "ingressclasses"
  | "networkpolicies"
  | "persistentvolumeclaims"
  | "persistentvolumes"
  | "storageclasses"
  | "namespaces"
  | "events"
  | "nodes"
  | "portforwards"
  | "helmreleases"
  | "settings"
  | "toolbox"
  | "assistant"
  | "newresource"
  | "editresource";

export const RESOURCE_LABELS: Record<ResourceKind, string> = {
  overview: "Overview",
  pods: "Pods",
  deployments: "Deployments",
  statefulsets: "StatefulSets",
  daemonsets: "DaemonSets",
  replicasets: "ReplicaSets",
  jobs: "Jobs",
  cronjobs: "CronJobs",
  configmaps: "ConfigMaps",
  secrets: "Secrets",
  resourcequotas: "Resource Quotas",
  limitranges: "Limit Ranges",
  horizontalpodautoscalers: "Horizontal Pod Autoscalers",
  poddisruptionbudgets: "Pod Disruption Budgets",
  priorityclasses: "Priority Classes",
  runtimeclasses: "Runtime Classes",
  leases: "Leases",
  mutatingwebhookconfigurations: "Mutating Webhook Configs",
  validatingwebhookconfigurations: "Validating Webhook Configs",
  serviceaccounts: "Service Accounts",
  clusterroles: "Cluster Roles",
  roles: "Roles",
  clusterrolebindings: "Cluster Role Bindings",
  rolebindings: "Role Bindings",
  services: "Services",
  endpoints: "Endpoints",
  endpointslices: "Endpoint Slices",
  ingresses: "Ingresses",
  ingressclasses: "Ingress Classes",
  networkpolicies: "Network Policies",
  persistentvolumeclaims: "Persistent Volume Claims",
  persistentvolumes: "Persistent Volumes",
  storageclasses: "Storage Classes",
  namespaces: "Namespaces",
  events: "Events",
  nodes: "Nodes",
  portforwards: "Port Forwards",
  helmreleases: "Helm Releases",
  settings: "Settings",
  toolbox: "Toolbox",
  assistant: "Assistant",
  newresource: "New Resource",
  editresource: "Edit Resource",
};

export const K8S_KIND: Record<ResourceKind, string> = {
  overview: "",
  pods: "Pod",
  deployments: "Deployment",
  statefulsets: "StatefulSet",
  daemonsets: "DaemonSet",
  replicasets: "ReplicaSet",
  jobs: "Job",
  cronjobs: "CronJob",
  configmaps: "ConfigMap",
  secrets: "Secret",
  resourcequotas: "ResourceQuota",
  limitranges: "LimitRange",
  horizontalpodautoscalers: "HorizontalPodAutoscaler",
  poddisruptionbudgets: "PodDisruptionBudget",
  priorityclasses: "PriorityClass",
  runtimeclasses: "RuntimeClass",
  leases: "Lease",
  mutatingwebhookconfigurations: "MutatingWebhookConfiguration",
  validatingwebhookconfigurations: "ValidatingWebhookConfiguration",
  serviceaccounts: "ServiceAccount",
  clusterroles: "ClusterRole",
  roles: "Role",
  clusterrolebindings: "ClusterRoleBinding",
  rolebindings: "RoleBinding",
  services: "Service",
  endpoints: "Endpoints",
  endpointslices: "EndpointSlice",
  ingresses: "Ingress",
  ingressclasses: "IngressClass",
  networkpolicies: "NetworkPolicy",
  persistentvolumeclaims: "PersistentVolumeClaim",
  persistentvolumes: "PersistentVolume",
  storageclasses: "StorageClass",
  namespaces: "Namespace",
  events: "Event",
  nodes: "Node",
  portforwards: "",
  helmreleases: "",
  settings: "",
  toolbox: "",
  assistant: "",
  newresource: "",
  editresource: "",
};
