import React, { useEffect, useMemo, useRef, useState } from "react";
import { Plus, RefreshCw } from "lucide-react";
import {
  podMetrics,
  type PodSummary,
  type DeploymentSummary,
  type ServiceSummary,
} from "../lib/workloads";
import { useNamespaceOptions } from "../lib/useNamespaceOptions";
import {
  listNodes,
  listResource,
  nodeMetrics,
  type NodeSummary,
  type ResourceRow,
  type EventSummary,
} from "../lib/manifest";
import {
  type StatefulSetSummary,
  type DaemonSetSummary,
  type JobSummary,
  type CronJobSummary,
  type ConfigMapSummary,
  type SecretSummary,
  type ResourceQuotaSummary,
  type LimitRangeSummary,
} from "../lib/controllers";
import {
  type IngressSummary,
  type EndpointSliceSummary,
  type NetworkPolicySummary,
} from "../lib/network";
import {
  formatStorageSize,
  type PvcSummary,
  type PvSummary,
  type StorageClassSummary,
} from "../lib/storage";
import {
  type ServiceAccountSummary,
  type RoleSummary,
  type ClusterRoleSummary,
  type RoleBindingSummary,
  type ClusterRoleBindingSummary,
} from "../lib/rbac";

type NodeRow = NodeSummary & { cpu?: number; memory?: number };
type PodRow = PodSummary & { cpu?: number; memory?: number };
import { watchResource, WATCHABLE_KINDS, type WatchHandle, type WatchStatus } from "../lib/watch";
import {
  parseNamespaceSelection,
  serializeNamespaceSelection,
  watchNamespaceForSelection,
  rowInSelection,
} from "../lib/namespaces";
import { NamespaceMultiSelect } from "../ui/NamespaceMultiSelect";
import { PodActions, ResourceActions, ServiceForwardAction } from "./DetailActions";
import { NodeCordonAction } from "./NodeCordonAction";
import { ResourceDetail } from "./ResourceDetail";
import type { OpenResource } from "../lib/resourceNavigation";
import { describeError } from "../lib/errors";
import {
  Table,
  filterTableData,
  Spinner,
  LoadingState,
  Badge,
  Button,
  ColumnPicker,
  useColumnVisibility,
  Drawer,
  ErrorState,
  StatusPill,
  TextInput,
  Toolbar,
  type Column,
  type StatusKind,
} from "../ui";

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
  newresource: "",
  editresource: "",
};

const CLUSTER_SCOPED: ResourceKind[] = [
  "nodes",
  "namespaces",
  "persistentvolumes",
  "storageclasses",
  "priorityclasses",
  "runtimeclasses",
  "mutatingwebhookconfigurations",
  "validatingwebhookconfigurations",
  "ingressclasses",
  "clusterroles",
  "clusterrolebindings",
];
// Typed views with bespoke columns; everything else namespaced uses the generic table.
const TYPED_KINDS: ResourceKind[] = [
  "pods",
  "deployments",
  "statefulsets",
  "daemonsets",
  "jobs",
  "cronjobs",
  "configmaps",
  "secrets",
  "resourcequotas",
  "limitranges",
  "services",
  "ingresses",
  "endpointslices",
  "networkpolicies",
  "persistentvolumeclaims",
  "persistentvolumes",
  "storageclasses",
  "serviceaccounts",
  "roles",
  "clusterroles",
  "rolebindings",
  "clusterrolebindings",
  "nodes",
  "events",
];
const isGeneric = (kind: ResourceKind) => !TYPED_KINDS.includes(kind);
const isNamespaced = (kind: ResourceKind) => !CLUSTER_SCOPED.includes(kind);
const isWatchable = (kind: ResourceKind) => (WATCHABLE_KINDS as readonly string[]).includes(kind);
const POLL_MS = 5000;

function phaseKind(phase: string): StatusKind {
  switch (phase) {
    case "Running":
    case "Succeeded":
    case "Ready":
      return "success";
    case "Pending":
      return "warning";
    case "Failed":
    case "Unknown":
    case "NotReady":
      return "danger";
    default:
      return "neutral";
  }
}

function Muted({ children }: { children: React.ReactNode }) {
  return <span className="text-muted-foreground">{children}</span>;
}

const podColumns: Column<PodRow>[] = [
  { key: "name", header: "Pod", render: (p) => <Muted>{p.name}</Muted> },
  { key: "namespace", header: "Namespace", render: (p) => <span className="fl-link">{p.namespace}</span> },
  { key: "cpu", header: "CPU", render: (p) => <Muted>{p.cpu != null ? `${p.cpu}m` : "—"}</Muted> },
  { key: "memory", header: "Memory", render: (p) => <Muted>{p.memory != null ? `${p.memory}Mi` : "—"}</Muted> },
  { key: "ready", header: "Ready" },
  { key: "phase", header: "Phase", render: (p) => <StatusPill status={p.phase} kind={phaseKind(p.phase)} /> },
  { key: "restarts", header: "Restarts" },
  { key: "node", header: "Node", render: (p) => <Muted>{p.node}</Muted> },
  { key: "age", header: "Age", render: (p) => <Muted>{p.age}</Muted> },
];

const deploymentColumns: Column<DeploymentSummary>[] = [
  { key: "name", header: "Deployment", render: (d) => <strong>{d.name}</strong> },
  { key: "namespace", header: "Namespace", render: (d) => <span className="fl-link">{d.namespace}</span> },
  { key: "ready", header: "Ready" },
  { key: "upToDate", header: "Up-to-date" },
  { key: "available", header: "Available" },
  { key: "age", header: "Age", render: (d) => <Muted>{d.age}</Muted> },
];

const statefulSetColumns: Column<StatefulSetSummary>[] = [
  { key: "name", header: "StatefulSet", render: (s) => <strong>{s.name}</strong> },
  { key: "namespace", header: "Namespace", render: (s) => <span className="fl-link">{s.namespace}</span> },
  { key: "ready", header: "Ready" },
  { key: "updated", header: "Updated" },
  { key: "service", header: "Service", render: (s) => <Muted>{s.service || "—"}</Muted> },
  { key: "age", header: "Age", render: (s) => <Muted>{s.age}</Muted> },
];

const daemonSetColumns: Column<DaemonSetSummary>[] = [
  { key: "name", header: "DaemonSet", render: (d) => <strong>{d.name}</strong> },
  { key: "namespace", header: "Namespace", render: (d) => <span className="fl-link">{d.namespace}</span> },
  { key: "desired", header: "Desired" },
  { key: "current", header: "Current" },
  { key: "ready", header: "Ready" },
  { key: "upToDate", header: "Up-to-date" },
  { key: "available", header: "Available" },
  { key: "age", header: "Age", render: (d) => <Muted>{d.age}</Muted> },
];

const jobColumns: Column<JobSummary>[] = [
  { key: "name", header: "Job", render: (j) => <strong>{j.name}</strong> },
  { key: "namespace", header: "Namespace", render: (j) => <span className="fl-link">{j.namespace}</span> },
  { key: "completions", header: "Completions" },
  {
    key: "status",
    header: "Status",
    render: (j) => {
      const [status, kind]: [string, StatusKind] =
        j.failed > 0 ? ["Failed", "danger"] : j.active > 0 ? ["Active", "warning"] : ["Complete", "success"];
      return <StatusPill status={status} kind={kind} />;
    },
  },
  { key: "duration", header: "Duration", render: (j) => <Muted>{j.duration || "—"}</Muted> },
  { key: "owner", header: "Owner", render: (j) => <Muted>{j.owner || "—"}</Muted> },
  { key: "age", header: "Age", render: (j) => <Muted>{j.age}</Muted> },
];

const cronJobColumns: Column<CronJobSummary>[] = [
  { key: "name", header: "CronJob", render: (c) => <strong>{c.name}</strong> },
  { key: "namespace", header: "Namespace", render: (c) => <span className="fl-link">{c.namespace}</span> },
  { key: "schedule", header: "Schedule", render: (c) => <Muted>{c.schedule}</Muted> },
  {
    key: "suspended",
    header: "State",
    render: (c) =>
      c.suspended ? <StatusPill status="Suspended" kind="neutral" /> : <StatusPill status="Active" kind="success" />,
  },
  { key: "active", header: "Active" },
  { key: "lastSchedule", header: "Last run", render: (c) => <Muted>{c.lastSchedule || "—"}</Muted> },
  { key: "age", header: "Age", render: (c) => <Muted>{c.age}</Muted> },
];

const configMapColumns: Column<ConfigMapSummary>[] = [
  { key: "name", header: "ConfigMap", render: (c) => <strong>{c.name}</strong> },
  { key: "namespace", header: "Namespace", render: (c) => <span className="fl-link">{c.namespace}</span> },
  { key: "keys", header: "Keys" },
  { key: "age", header: "Age", render: (c) => <Muted>{c.age}</Muted> },
];

const secretColumns: Column<SecretSummary>[] = [
  { key: "name", header: "Secret", render: (s) => <strong>{s.name}</strong> },
  { key: "namespace", header: "Namespace", render: (s) => <span className="fl-link">{s.namespace}</span> },
  { key: "type", header: "Type", render: (s) => <Muted>{s.type}</Muted> },
  { key: "keys", header: "Keys" },
  { key: "age", header: "Age", render: (s) => <Muted>{s.age}</Muted> },
];

const resourceQuotaColumns: Column<ResourceQuotaSummary>[] = [
  { key: "name", header: "Resource Quota", render: (q) => <strong>{q.name}</strong> },
  { key: "namespace", header: "Namespace", render: (q) => <span className="fl-link">{q.namespace}</span> },
  { key: "resources", header: "Resources" },
  { key: "age", header: "Age", render: (q) => <Muted>{q.age}</Muted> },
];

const limitRangeColumns: Column<LimitRangeSummary>[] = [
  { key: "name", header: "Limit Range", render: (l) => <strong>{l.name}</strong> },
  { key: "namespace", header: "Namespace", render: (l) => <span className="fl-link">{l.namespace}</span> },
  { key: "limits", header: "Limits" },
  { key: "age", header: "Age", render: (l) => <Muted>{l.age}</Muted> },
];

const serviceColumns: Column<ServiceSummary>[] = [
  { key: "name", header: "Service", render: (s) => <strong>{s.name}</strong> },
  { key: "namespace", header: "Namespace", render: (s) => <span className="fl-link">{s.namespace}</span> },
  { key: "type", header: "Type" },
  { key: "clusterIP", header: "Cluster IP", render: (s) => <Muted>{s.clusterIP}</Muted> },
  { key: "ports", header: "Ports" },
  { key: "age", header: "Age", render: (s) => <Muted>{s.age}</Muted> },
];

const ingressColumns: Column<IngressSummary>[] = [
  { key: "name", header: "Ingress", render: (i) => <strong>{i.name}</strong> },
  { key: "namespace", header: "Namespace", render: (i) => <span className="fl-link">{i.namespace}</span> },
  { key: "class", header: "Class", render: (i) => <Muted>{i.class}</Muted> },
  { key: "hosts", header: "Hosts", render: (i) => <Muted>{i.hosts || "*"}</Muted> },
  { key: "address", header: "Address", render: (i) => <Muted>{i.address || "—"}</Muted> },
  { key: "ports", header: "Ports", render: (i) => <Muted>{i.ports}</Muted> },
  { key: "age", header: "Age", render: (i) => <Muted>{i.age}</Muted> },
];

const endpointSliceColumns: Column<EndpointSliceSummary>[] = [
  { key: "name", header: "Endpoint Slice", render: (e) => <strong>{e.name}</strong> },
  { key: "namespace", header: "Namespace", render: (e) => <span className="fl-link">{e.namespace}</span> },
  { key: "addressType", header: "Address Type", render: (e) => <Muted>{e.addressType}</Muted> },
  { key: "endpoints", header: "Endpoints", render: (e) => <Muted>{e.endpoints}</Muted> },
  { key: "ports", header: "Ports", render: (e) => <Muted>{e.ports || "—"}</Muted> },
  { key: "service", header: "Service", render: (e) => <span className="fl-link">{e.service || "—"}</span> },
  { key: "age", header: "Age", render: (e) => <Muted>{e.age}</Muted> },
];

const networkPolicyColumns: Column<NetworkPolicySummary>[] = [
  { key: "name", header: "Network Policy", render: (n) => <strong>{n.name}</strong> },
  { key: "namespace", header: "Namespace", render: (n) => <span className="fl-link">{n.namespace}</span> },
  { key: "podSelector", header: "Pod Selector", render: (n) => <Muted>{n.podSelector}</Muted> },
  { key: "ingress", header: "Ingress" },
  { key: "egress", header: "Egress" },
  { key: "policyTypes", header: "Policy Types", render: (n) => <Muted>{n.policyTypes || "—"}</Muted> },
  { key: "age", header: "Age", render: (n) => <Muted>{n.age}</Muted> },
];

const pvcColumns: Column<PvcSummary>[] = [
  { key: "name", header: "Claim", render: (p) => <strong>{p.name}</strong> },
  { key: "namespace", header: "Namespace", render: (p) => <span className="fl-link">{p.namespace}</span> },
  { key: "status", header: "Status", render: (p) => <StatusPill status={p.status} kind={phaseKind(p.status === "Bound" ? "Ready" : p.status)} /> },
  { key: "capacity", header: "Capacity", render: (p) => <Muted>{formatStorageSize(p.capacity)}</Muted> },
  { key: "accessModes", header: "Access Modes", render: (p) => <Muted>{p.accessModes || "—"}</Muted> },
  { key: "storageClass", header: "Storage Class", render: (p) => <span className="fl-link">{p.storageClass || "—"}</span> },
  { key: "volume", header: "Volume", render: (p) => <span className="fl-link">{p.volume || "—"}</span> },
  { key: "age", header: "Age", render: (p) => <Muted>{p.age}</Muted> },
];

const pvColumns: Column<PvSummary>[] = [
  { key: "name", header: "Volume", render: (p) => <strong>{p.name}</strong> },
  { key: "capacity", header: "Capacity", render: (p) => <Muted>{formatStorageSize(p.capacity)}</Muted> },
  { key: "accessModes", header: "Access Modes", render: (p) => <Muted>{p.accessModes || "—"}</Muted> },
  { key: "reclaimPolicy", header: "Reclaim", render: (p) => <Muted>{p.reclaimPolicy || "—"}</Muted> },
  { key: "status", header: "Status", render: (p) => <StatusPill status={p.status} kind={phaseKind(p.status === "Bound" || p.status === "Available" ? "Ready" : p.status)} /> },
  { key: "claim", header: "Claim", render: (p) => <span className="fl-link">{p.claim || "—"}</span> },
  { key: "storageClass", header: "Storage Class", render: (p) => <span className="fl-link">{p.storageClass || "—"}</span> },
  { key: "age", header: "Age", render: (p) => <Muted>{p.age}</Muted> },
];

const storageClassColumns: Column<StorageClassSummary>[] = [
  { key: "name", header: "Storage Class", render: (s) => <strong>{s.name}</strong> },
  { key: "provisioner", header: "Provisioner", render: (s) => <Muted>{s.provisioner}</Muted> },
  { key: "reclaimPolicy", header: "Reclaim", render: (s) => <Muted>{s.reclaimPolicy || "—"}</Muted> },
  { key: "volumeBindingMode", header: "Binding Mode", render: (s) => <Muted>{s.volumeBindingMode || "—"}</Muted> },
  { key: "default", header: "Default", render: (s) => (s.default ? <StatusPill status="Default" kind="success" /> : <Muted>—</Muted>) },
  { key: "age", header: "Age", render: (s) => <Muted>{s.age}</Muted> },
];

const serviceAccountColumns: Column<ServiceAccountSummary>[] = [
  { key: "name", header: "Service Account", render: (s) => <strong>{s.name}</strong> },
  { key: "namespace", header: "Namespace", render: (s) => <span className="fl-link">{s.namespace}</span> },
  { key: "secrets", header: "Secrets" },
  { key: "age", header: "Age", render: (s) => <Muted>{s.age}</Muted> },
];

const roleColumns: Column<RoleSummary>[] = [
  { key: "name", header: "Role", render: (r) => <strong>{r.name}</strong> },
  { key: "namespace", header: "Namespace", render: (r) => <span className="fl-link">{r.namespace}</span> },
  { key: "rules", header: "Rules" },
  { key: "age", header: "Age", render: (r) => <Muted>{r.age}</Muted> },
];

const clusterRoleColumns: Column<ClusterRoleSummary>[] = [
  { key: "name", header: "Cluster Role", render: (r) => <strong>{r.name}</strong> },
  { key: "rules", header: "Rules" },
  { key: "age", header: "Age", render: (r) => <Muted>{r.age}</Muted> },
];

const roleBindingColumns: Column<RoleBindingSummary>[] = [
  { key: "name", header: "Role Binding", render: (b) => <strong>{b.name}</strong> },
  { key: "namespace", header: "Namespace", render: (b) => <span className="fl-link">{b.namespace}</span> },
  { key: "role", header: "Role", render: (b) => <span className="fl-link">{b.role}</span> },
  { key: "subjects", header: "Subjects" },
  { key: "age", header: "Age", render: (b) => <Muted>{b.age}</Muted> },
];

const clusterRoleBindingColumns: Column<ClusterRoleBindingSummary>[] = [
  { key: "name", header: "Cluster Role Binding", render: (b) => <strong>{b.name}</strong> },
  { key: "role", header: "Role", render: (b) => <span className="fl-link">{b.role}</span> },
  { key: "subjects", header: "Subjects" },
  { key: "age", header: "Age", render: (b) => <Muted>{b.age}</Muted> },
];

const nodeColumns: Column<NodeRow>[] = [
  { key: "name", header: "Node", render: (n) => <strong>{n.name}</strong> },
  {
    key: "status",
    header: "Status",
    render: (n) => (
      <span className="inline-flex items-center gap-1.5">
        <StatusPill status={n.status} kind={phaseKind(n.status)} />
        {n.unschedulable && <Badge variant="warning">SchedulingDisabled</Badge>}
        {n.taints > 0 && (
          <Badge variant="neutral">{n.taints > 1 ? `Tainted (${n.taints})` : "Tainted"}</Badge>
        )}
      </span>
    ),
  },
  { key: "roles", header: "Roles" },
  { key: "cpu", header: "CPU", render: (n) => <Muted>{n.cpu != null ? `${n.cpu}m` : "—"}</Muted> },
  { key: "memory", header: "Memory", render: (n) => <Muted>{n.memory != null ? `${n.memory}Mi` : "—"}</Muted> },
  { key: "version", header: "Version", render: (n) => <Muted>{n.version}</Muted> },
  { key: "age", header: "Age", render: (n) => <Muted>{n.age}</Muted> },
];

const genericColumns: Column<ResourceRow>[] = [
  { key: "name", header: "Name", render: (r) => <strong>{r.name}</strong> },
  { key: "namespace", header: "Namespace", render: (r) => <span className="fl-link">{r.namespace}</span> },
  { key: "age", header: "Age", render: (r) => <Muted>{r.age}</Muted> },
];

const eventColumns: Column<EventSummary & { name: string }>[] = [
  {
    key: "type",
    header: "Type",
    render: (e) => <StatusPill status={e.type} kind={e.type === "Warning" ? "danger" : "info"} />,
  },
  { key: "reason", header: "Reason", render: (e) => <strong>{e.reason}</strong> },
  { key: "object", header: "Object", render: (e) => <span className="fl-link">{e.object}</span> },
  { key: "message", header: "Message" },
  { key: "age", header: "Age", render: (e) => <Muted>{e.age}</Muted> },
];

interface ResourceState {
  rows: Array<{ name: string }>;
  error: string;
  loading: boolean;
}

interface OtherDetail {
  kind: string;
  namespace: string | null;
  name: string;
}

export function ResourceBrowser({
  context,
  kind,
  query = "",
  onQueryChange,
  onOpenTerminal,
  onOpenLogs,
  onOpenWorkloadLogs,
  onOpenNew,
  onOpenEdit,
  onOpenResource,
  focus,
  initialNamespace = "",
  onNamespaceChange,
  detailDrawerWidth = 480,
  kubeconfigFiles = [],
}: {
  context: string;
  kind: ResourceKind;
  query?: string;
  onQueryChange?: (q: string) => void;
  onOpenTerminal?: (s: { context: string; namespace: string; pod: string; container?: string }) => void;
  onOpenLogs?: (s: { context: string; namespace: string; pod: string; container?: string }) => void;
  onOpenWorkloadLogs?: (s: { context: string; namespace: string; kind: string; name: string }) => void;
  /** Open a "new resource" editor tab, optionally seeded with this kind's template. */
  onOpenNew?: (initialKind?: string) => void;
  /** Open a full-tab editor preloaded with a resource's manifest. */
  onOpenEdit?: (kind: string, namespace: string | null, name: string) => void;
  onOpenResource?: OpenResource;
  /** Deep-link target (from global search): open this resource's detail once it loads. */
  focus?: { name: string; namespace: string | null; nonce: number };
  /** Namespace to start on (empty = all); persisted per tab/cluster by the parent. */
  initialNamespace?: string;
  /** Notified when the namespace filter changes, so the parent can preserve it. */
  onNamespaceChange?: (namespace: string) => void;
  detailDrawerWidth?: number;
  /**
   * All configured kubeconfig files (default path + pasted/additional). Used to
   * register their paths in the backend client cache before we build a client
   * for this context — otherwise a restored tab for a context from an additional
   * file races the app's initial listContexts and fails with "failed to load
   * current context".
   */
  kubeconfigFiles?: string[];
}) {
  const { namespaces, scope: nsScope, error: nsError } = useNamespaceOptions(context, kubeconfigFiles);
  // Namespace selection is a set (empty = all namespaces), serialized to/from
  // the persisted comma string. One selected namespace watches that namespace
  // directly; none or many watch all namespaces, filtered client-side.
  const [selection, setSelection] = useState(() => parseNamespaceSelection(initialNamespace));
  const changeNamespaces = (next: string[]) => {
    setSelection(next);
    onNamespaceChange?.(serializeNamespaceSelection(next));
  };
  // Restricted credentials scope the view to a single namespace — force the
  // selection to match rather than leaving it at whatever was selected before.
  useEffect(() => {
    if (nsScope) setSelection([nsScope]);
  }, [nsScope]);
  const watchNamespace = watchNamespaceForSelection(selection);
  const selectionKey = serializeNamespaceSelection(selection);
  const [res, setRes] = useState<ResourceState>({ rows: [], error: "", loading: false });
  const [watchStatus, setWatchStatus] = useState<WatchStatus>("live");
  const [selectedPod, setSelectedPod] = useState<PodSummary | null>(null);
  const [otherDetail, setOtherDetail] = useState<OtherDetail | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  // Bumped after a write action so the open detail overview re-fetches.
  const [detailReload, setDetailReload] = useState(0);
  const [filterColumn, setFilterColumn] = useState<string | null>(null);
  // Per-pod CPU/memory (millicores / MiB), merged into the pods table.
  const [podCpuMem, setPodCpuMem] = useState<Map<string, { cpu: number; mem: number }>>(new Map());
  const viewKeyRef = useRef("");

  const namespaced = isNamespaced(kind);

  useEffect(() => setFilterColumn(null), [kind]);

  useEffect(() => {
    if (namespaced && namespaces === null) return; // wait for the namespace list
    let cancelled = false;
    // Only reset the table for a genuinely new view; a poll keeps current rows.
    const viewKey = `${context}|${watchNamespace}|${kind}`;
    const fresh = viewKeyRef.current !== viewKey;
    viewKeyRef.current = viewKey;
    if (fresh) {
      setSelectedPod(null);
      setOtherDetail(null);
      setRes({ rows: [], error: "", loading: true });
    } else {
      setRes((r) => ({ ...r, loading: true }));
    }

    if (isWatchable(kind)) {
      if (fresh) setWatchStatus("live");
      let handle: WatchHandle | null = null;
      void watchResource(
        context,
        watchNamespace,
        kind,
        (rows) => {
          if (!cancelled) setRes({ rows, error: "", loading: false });
        },
        (status) => {
          if (!cancelled) setWatchStatus(status);
        },
        (err) => {
          if (!cancelled) setRes({ rows: [], error: err, loading: false });
        },
        kubeconfigFiles,
      )
        .then((h) => (cancelled ? h.stop() : (handle = h)))
        .catch((e) => {
          if (!cancelled) setRes({ rows: [], error: String(e), loading: false });
        });
      return () => {
        cancelled = true;
        handle?.stop();
      };
    }

    // Non-watchable kinds (nodes, generic) load on demand + poll. (Events now
    // stream via watch.)
    const loader: Promise<{ rows?: Array<{ name: string }>; error?: string }> =
      kind === "nodes"
        ? Promise.all([listNodes(context), nodeMetrics(context)]).then(([n, m]) => {
            const mm = new Map((m.metrics ?? []).map((x) => [x.name, x]));
            const rows: NodeRow[] = (n.nodes ?? []).map((nd) => ({
              ...nd,
              cpu: mm.get(nd.name)?.cpuMillicores,
              memory: mm.get(nd.name)?.memoryMiB,
            }));
            return { rows, error: n.error }; // metrics are best-effort
          })
        : listResource(context, K8S_KIND[kind], watchNamespace).then((o) => ({
            rows: o.items,
            error: o.error,
          }));
    void loader.then(({ rows, error }) => {
      if (!cancelled) setRes({ rows: rows ?? [], error: error ?? "", loading: false });
    });
    return () => {
      cancelled = true;
    };
  }, [context, watchNamespace, kind, namespaced, namespaces, reloadKey]);

  // Poll non-watchable kinds for a live-updating feel (true watch streams
  // cover pods/deployments/services).
  useEffect(() => {
    if (isWatchable(kind)) return;
    if (namespaced && namespaces === null) return;
    const t = setInterval(() => setReloadKey((k) => k + 1), POLL_MS);
    return () => clearInterval(t);
  }, [kind, watchNamespace, namespaced, namespaces, context]);

  // Pods stream over watch (no metrics) — poll pod CPU/memory separately and
  // merge by name. Best-effort: a missing metrics-server just leaves "—".
  useEffect(() => {
    if (kind !== "pods") {
      setPodCpuMem(new Map());
      return;
    }
    let active = true;
    const fetchMetrics = () =>
      void podMetrics(context, watchNamespace).then((o) => {
        if (!active) return;
        setPodCpuMem(
          new Map((o.metrics ?? []).map((m) => [m.name, { cpu: m.cpuMillicores, mem: m.memoryMiB }])),
        );
      });
    fetchMetrics();
    const t = setInterval(fetchMetrics, 10000);
    return () => {
      active = false;
      clearInterval(t);
    };
  }, [kind, context, watchNamespace]);

  const columns = useMemo(() => {
    if (kind === "events") return eventColumns as unknown as Column<{ name: string }>[];
    if (isGeneric(kind)) return genericColumns as Column<{ name: string }>[];
    switch (kind) {
      case "pods":
        return podColumns as Column<{ name: string }>[];
      case "deployments":
        return deploymentColumns as Column<{ name: string }>[];
      case "statefulsets":
        return statefulSetColumns as Column<{ name: string }>[];
      case "daemonsets":
        return daemonSetColumns as Column<{ name: string }>[];
      case "jobs":
        return jobColumns as Column<{ name: string }>[];
      case "cronjobs":
        return cronJobColumns as Column<{ name: string }>[];
      case "configmaps":
        return configMapColumns as Column<{ name: string }>[];
      case "secrets":
        return secretColumns as Column<{ name: string }>[];
      case "resourcequotas":
        return resourceQuotaColumns as Column<{ name: string }>[];
      case "limitranges":
        return limitRangeColumns as Column<{ name: string }>[];
      case "services":
        return serviceColumns as Column<{ name: string }>[];
      case "ingresses":
        return ingressColumns as Column<{ name: string }>[];
      case "endpointslices":
        return endpointSliceColumns as Column<{ name: string }>[];
      case "networkpolicies":
        return networkPolicyColumns as Column<{ name: string }>[];
      case "persistentvolumeclaims":
        return pvcColumns as Column<{ name: string }>[];
      case "persistentvolumes":
        return pvColumns as Column<{ name: string }>[];
      case "storageclasses":
        return storageClassColumns as Column<{ name: string }>[];
      case "serviceaccounts":
        return serviceAccountColumns as Column<{ name: string }>[];
      case "roles":
        return roleColumns as Column<{ name: string }>[];
      case "clusterroles":
        return clusterRoleColumns as Column<{ name: string }>[];
      case "rolebindings":
        return roleBindingColumns as Column<{ name: string }>[];
      case "clusterrolebindings":
        return clusterRoleBindingColumns as Column<{ name: string }>[];
      default:
        return nodeColumns as Column<{ name: string }>[];
    }
  }, [kind]);

  // Show/hide columns, persisted per resource kind. The first (identifier)
  // column is always kept.
  const { visibleColumns, columnOptions, hidden, toggle: toggleColumn, pinnedKey: pinnedColumnKey } =
    useColumnVisibility(kind, columns);
  // A hidden column can't remain the active search filter.
  useEffect(() => {
    if (filterColumn && !visibleColumns.some((column) => column.key === filterColumn)) {
      setFilterColumn(null);
    }
  }, [visibleColumns, filterColumn]);

  function onRowClick(row: { name: string }) {
    if (kind === "events") return; // events have no manifest detail
    if (kind === "pods") {
      setSelectedPod(row as PodSummary);
    } else {
      const rowNs = (row as { namespace?: string }).namespace;
      setOtherDetail({
        kind: K8S_KIND[kind],
        namespace: namespaced ? rowNs || watchNamespace || null : null,
        name: row.name,
      });
    }
  }

  // Deep-link from global search: once rows load, open the target's detail.
  const focusHandledRef = useRef(0);
  useEffect(() => {
    if (!focus || focus.nonce === focusHandledRef.current) return;
    const row = res.rows.find(
      (r) =>
        r.name === focus.name &&
        (focus.namespace == null || (r as { namespace?: string }).namespace === focus.namespace),
    );
    if (row) {
      focusHandledRef.current = focus.nonce;
      onRowClick(row);
    }
    // onRowClick is stable enough for this one-shot; deps intentionally minimal.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus, res.rows]);

  const selectedKey = kind === "pods" ? selectedPod?.name : otherDetail?.name;

  // Merge live pod metrics into the pod rows, and restrict namespaced rows to
  // the selected namespaces (when watching all-namespaces for a multi-select).
  const tableRows = useMemo(() => {
    let rows: Array<{ name: string; namespace?: string }> =
      kind === "pods"
        ? (res.rows as PodSummary[]).map((p) => {
            const m = podCpuMem.get(p.name);
            return { ...p, cpu: m?.cpu, memory: m?.mem } as PodRow;
          })
        : (res.rows as Array<{ name: string; namespace?: string }>);
    if (namespaced && selection.length > 0) {
      rows = rows.filter((r) => rowInSelection(r.namespace ?? "", selection));
    }
    return rows;
  }, [res.rows, kind, podCpuMem, namespaced, selectionKey]);

  const filtered = useMemo(
    () => filterTableData(tableRows, visibleColumns, query, filterColumn),
    [visibleColumns, filterColumn, query, tableRows],
  );
  const filterLabel = filterColumn
    ? visibleColumns.find((column) => column.key === filterColumn)?.header
    : null;

  function closeDetail() {
    setSelectedPod(null);
    setOtherDetail(null);
  }

  const detailTitle = selectedPod ? (
    <>Pod: <code>{selectedPod.name}</code></>
  ) : otherDetail ? (
    <>{otherDetail.kind}: <code>{otherDetail.name}</code></>
  ) : null;

  const detailActions = selectedPod ? (
    <PodActions
      context={context}
      pod={selectedPod}
      onDeleted={closeDetail}
      onOpenTerminal={onOpenTerminal}
      onOpenLogs={onOpenLogs}
      onEdit={onOpenEdit ? () => onOpenEdit("Pod", selectedPod.namespace, selectedPod.name) : undefined}
    />
  ) : otherDetail ? (
    <>
      {otherDetail.kind === "Node" && (
        <NodeCordonAction context={context} name={otherDetail.name} />
      )}
      {otherDetail.kind === "Service" && (
        <ServiceForwardAction
          context={context}
          namespace={otherDetail.namespace}
          name={otherDetail.name}
        />
      )}
      <ResourceActions
        context={context}
        kind={otherDetail.kind}
        namespace={otherDetail.namespace}
        name={otherDetail.name}
        cronjobSuspended={
          otherDetail.kind === "CronJob"
            ? (res.rows as CronJobSummary[]).find((r) => r.name === otherDetail.name)?.suspended
            : undefined
        }
        onDeleted={closeDetail}
        onChanged={() => setDetailReload((k) => k + 1)}
        onOpenLogs={onOpenWorkloadLogs}
        onEdit={
          onOpenEdit ? () => onOpenEdit(otherDetail.kind, otherDetail.namespace, otherDetail.name) : undefined
        }
      />
    </>
  ) : null;

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        {namespaces === null && (
          <div className="p-3">
            <Spinner label="Loading namespaces" />
          </div>
        )}
        {namespaces !== null && (
          <>
            {nsScope && (
              <p className="px-3 py-1 text-xs text-muted-foreground" role="status">
                Scoped to namespace {nsScope} — you don’t have permission to list all namespaces.
              </p>
            )}
            {nsError && (
              <p className="px-3 py-1 text-xs text-muted-foreground" role="status">
                {describeError(nsError).title} — can’t list namespaces; showing all.
              </p>
            )}
            <Toolbar className="fl-resource-toolbar shrink-0 flex-wrap">
              {namespaced && (
                <div className="fl-resource-toolbar__namespace flex items-center gap-2">
                  <span>Namespace</span>
                  <NamespaceMultiSelect
                    namespaces={namespaces ?? []}
                    selection={selection}
                    onChange={changeNamespaces}
                    ariaLabel="Namespace"
                    className="min-w-44"
                  />
                </div>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setReloadKey((k) => k + 1)}
                disabled={res.loading}
              >
                <RefreshCw data-icon="inline-start" />
                Refresh
              </Button>
              {onOpenNew && (
                <Button variant="ghost" size="sm" onClick={() => onOpenNew(K8S_KIND[kind] || undefined)}>
                  <Plus data-icon="inline-start" />
                  New
                </Button>
              )}
              {isWatchable(kind) &&
                !res.loading &&
                (watchStatus === "reconnecting" ? (
                  <Badge variant="warning">reconnecting…</Badge>
                ) : (
                  <Badge variant="success">live</Badge>
                ))}
              {res.loading && filtered.length > 0 && <Spinner label="Loading resources" />}
              <div className="ml-auto">
                <ColumnPicker
                  columns={columnOptions}
                  hidden={hidden}
                  onToggle={toggleColumn}
                  pinnedKey={pinnedColumnKey}
                />
              </div>
              <div className="fl-resource-toolbar__search w-56">
                <TextInput
                  value={query}
                  onValueChange={(q) => onQueryChange?.(q)}
                  type="search"
                  placeholder={typeof filterLabel === "string" ? `Search ${filterLabel}…` : "Search all columns…"}
                  aria-label="Search resources"
                />
              </div>
              {!res.error && (
                <span className="fl-resource-toolbar__count tabular-nums">
                  {filtered.length} {filtered.length === 1 ? "item" : "items"}
                </span>
              )}
            </Toolbar>

            <div className="min-h-0 flex-1 overflow-auto">
              {res.error && (
                <ErrorState
                  title={describeError(res.error).title}
                  detail={describeError(res.error).detail}
                  onRetry={() => setReloadKey((k) => k + 1)}
                />
              )}
              {!res.error && res.loading && filtered.length === 0 && (
                <LoadingState label={`Loading ${kind}`} />
              )}
              {!res.error && !(res.loading && filtered.length === 0) && (
                <Table
                  columns={visibleColumns}
                  data={filtered}
                  getRowKey={(r) => r.name}
                  selectedKey={selectedKey}
                  onRowClick={kind === "events" ? undefined : onRowClick}
                  activeFilterKey={filterColumn}
                  onActiveFilterKeyChange={setFilterColumn}
                  emptyText={
                    query
                      ? "No matches"
                      : `No ${kind}${
                          namespaced && selection.length === 1
                            ? ` in ${selection[0]}`
                            : namespaced && selection.length > 1
                              ? ` in ${selection.length} namespaces`
                              : ""
                        }`
                  }
                />
              )}
            </div>
          </>
        )}
      </div>

      <Drawer
        open={!!selectedPod || !!otherDetail}
        defaultWidth={detailDrawerWidth}
        title={detailTitle}
        headerActions={detailActions}
        onClose={closeDetail}
      >
        {selectedPod && (
          <ResourceDetail
            context={context}
            kind="Pod"
            namespace={selectedPod.namespace}
            name={selectedPod.name}
            onOpenResource={onOpenResource}
            onOpenLogs={(container) =>
              onOpenLogs?.({ context, namespace: selectedPod.namespace, pod: selectedPod.name, container })
            }
            onOpenExec={(container) =>
              onOpenTerminal?.({ context, namespace: selectedPod.namespace, pod: selectedPod.name, container })
            }
          />
        )}
        {otherDetail && (
          <ResourceDetail
            context={context}
            kind={otherDetail.kind}
            namespace={otherDetail.namespace}
            name={otherDetail.name}
            reloadKey={detailReload}
            onOpenResource={onOpenResource}
          />
        )}
      </Drawer>
    </div>
  );
}
