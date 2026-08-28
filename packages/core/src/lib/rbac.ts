import { invokeCapability, type Invoker } from "../transport/transport";
import type { PodSummary } from "./workloads";

/** ServiceAccount row — mirrors `crates/kube/src/serviceaccounts.rs`. */
export interface ServiceAccountSummary {
  name: string;
  namespace: string;
  secrets: number;
  age: string;
}

/** Role row — mirrors `crates/kube/src/roles.rs`. */
export interface RoleSummary {
  name: string;
  namespace: string;
  rules: number;
  age: string;
}

/** ClusterRole row — mirrors `crates/kube/src/roles.rs` (cluster-scoped). */
export interface ClusterRoleSummary {
  name: string;
  rules: number;
  age: string;
}

/** RoleBinding row — mirrors `crates/kube/src/rolebindings.rs`. */
export interface RoleBindingSummary {
  name: string;
  namespace: string;
  /** The referenced role as "Kind/name". */
  role: string;
  subjects: number;
  age: string;
}

/** ClusterRoleBinding row — mirrors `crates/kube/src/rolebindings.rs` (cluster-scoped). */
export interface ClusterRoleBindingSummary {
  name: string;
  role: string;
  subjects: number;
  age: string;
}

/** List ServiceAccounts in a namespace via `k8s.listServiceAccounts`. */
export async function listServiceAccounts(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ serviceaccounts?: ServiceAccountSummary[]; error?: string }> {
  try {
    const out = await invoke<{ serviceaccounts: ServiceAccountSummary[] }>("k8s.listServiceAccounts", {
      context,
      namespace,
    });
    return { serviceaccounts: out.serviceaccounts };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List Roles in a namespace via `k8s.listRoles`. */
export async function listRoles(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ roles?: RoleSummary[]; error?: string }> {
  try {
    const out = await invoke<{ roles: RoleSummary[] }>("k8s.listRoles", { context, namespace });
    return { roles: out.roles };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List cluster ClusterRoles via `k8s.listClusterRoles`. */
export async function listClusterRoles(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ clusterroles?: ClusterRoleSummary[]; error?: string }> {
  try {
    const out = await invoke<{ clusterroles: ClusterRoleSummary[] }>("k8s.listClusterRoles", { context });
    return { clusterroles: out.clusterroles };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List RoleBindings in a namespace via `k8s.listRoleBindings`. */
export async function listRoleBindings(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ rolebindings?: RoleBindingSummary[]; error?: string }> {
  try {
    const out = await invoke<{ rolebindings: RoleBindingSummary[] }>("k8s.listRoleBindings", { context, namespace });
    return { rolebindings: out.rolebindings };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List cluster ClusterRoleBindings via `k8s.listClusterRoleBindings`. */
export async function listClusterRoleBindings(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ clusterrolebindings?: ClusterRoleBindingSummary[]; error?: string }> {
  try {
    const out = await invoke<{ clusterrolebindings: ClusterRoleBindingSummary[] }>("k8s.listClusterRoleBindings", {
      context,
    });
    return { clusterrolebindings: out.clusterrolebindings };
  } catch (e) {
    return { error: String(e) };
  }
}

/** A (Cluster)RoleBinding that grants a ServiceAccount its permissions. */
export interface SaBinding {
  name: string;
  /** Binding namespace (null for a ClusterRoleBinding). */
  namespace: string | null;
  /** "RoleBinding" or "ClusterRoleBinding". */
  kind: string;
  /** The granted role as "Kind/name". */
  role: string;
}

/** List the bindings that reference a ServiceAccount via `k8s.bindingsForServiceAccount`. */
export async function bindingsForServiceAccount(
  context: string,
  namespace: string,
  serviceaccount: string,
  invoke: Invoker = invokeCapability,
): Promise<{ bindings?: SaBinding[]; error?: string }> {
  try {
    const out = await invoke<{ bindings: SaBinding[] }>("k8s.bindingsForServiceAccount", {
      context,
      namespace,
      serviceaccount,
    });
    return { bindings: out.bindings };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List pods running as a ServiceAccount via `k8s.podsForServiceAccount`. */
export async function podsForServiceAccount(
  context: string,
  namespace: string,
  serviceaccount: string,
  invoke: Invoker = invokeCapability,
): Promise<{ pods?: PodSummary[]; error?: string }> {
  try {
    const out = await invoke<{ pods: PodSummary[] }>("k8s.podsForServiceAccount", {
      context,
      namespace,
      serviceaccount,
    });
    return { pods: out.pods };
  } catch (e) {
    return { error: String(e) };
  }
}
