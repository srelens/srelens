import { invokeCapability, type Invoker } from "../transport/transport";

/** StatefulSet row — mirrors `crates/kube/src/statefulsets.rs`. */
export interface StatefulSetSummary {
  name: string;
  namespace: string;
  ready: string;
  updated: number;
  service: string;
  age: string;
}

/** DaemonSet row — mirrors `crates/kube/src/daemonsets.rs`. */
export interface DaemonSetSummary {
  name: string;
  namespace: string;
  desired: number;
  current: number;
  ready: number;
  upToDate: number;
  available: number;
  age: string;
}

/** Job row — mirrors `crates/kube/src/jobs.rs`. */
export interface JobSummary {
  name: string;
  namespace: string;
  completions: string;
  active: number;
  failed: number;
  duration: string;
  owner: string;
  age: string;
}

/** CronJob row — mirrors `crates/kube/src/cronjobs.rs`. */
export interface CronJobSummary {
  name: string;
  namespace: string;
  schedule: string;
  suspended: boolean;
  active: number;
  lastSchedule: string;
  age: string;
}

/** List StatefulSets in a namespace via `k8s.listStatefulSets`. */
export async function listStatefulSets(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ statefulsets?: StatefulSetSummary[]; error?: string }> {
  try {
    const out = await invoke<{ statefulsets: StatefulSetSummary[] }>("k8s.listStatefulSets", {
      context,
      namespace,
    });
    return { statefulsets: out.statefulsets };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List DaemonSets in a namespace via `k8s.listDaemonSets`. */
export async function listDaemonSets(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ daemonsets?: DaemonSetSummary[]; error?: string }> {
  try {
    const out = await invoke<{ daemonsets: DaemonSetSummary[] }>("k8s.listDaemonSets", {
      context,
      namespace,
    });
    return { daemonsets: out.daemonsets };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List Jobs in a namespace via `k8s.listJobs`. */
export async function listJobs(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ jobs?: JobSummary[]; error?: string }> {
  try {
    const out = await invoke<{ jobs: JobSummary[] }>("k8s.listJobs", { context, namespace });
    return { jobs: out.jobs };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List CronJobs in a namespace via `k8s.listCronJobs`. */
export async function listCronJobs(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ cronjobs?: CronJobSummary[]; error?: string }> {
  try {
    const out = await invoke<{ cronjobs: CronJobSummary[] }>("k8s.listCronJobs", { context, namespace });
    return { cronjobs: out.cronjobs };
  } catch (e) {
    return { error: String(e) };
  }
}

/** ConfigMap row — mirrors `crates/kube/src/configmaps.rs`. */
export interface ConfigMapSummary {
  name: string;
  namespace: string;
  keys: number;
  age: string;
}

/** Secret row — mirrors `crates/kube/src/secrets.rs`. Carries type + key count only, never values. */
export interface SecretSummary {
  name: string;
  namespace: string;
  type: string;
  keys: number;
  age: string;
}

/** ResourceQuota row — mirrors `crates/kube/src/resourcequotas.rs`. */
export interface ResourceQuotaSummary {
  name: string;
  namespace: string;
  resources: number;
  age: string;
}

/** LimitRange row — mirrors `crates/kube/src/limitranges.rs`. */
export interface LimitRangeSummary {
  name: string;
  namespace: string;
  limits: number;
  age: string;
}

/** List ConfigMaps in a namespace via `k8s.listConfigMaps`. */
export async function listConfigMaps(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ configmaps?: ConfigMapSummary[]; error?: string }> {
  try {
    const out = await invoke<{ configmaps: ConfigMapSummary[] }>("k8s.listConfigMaps", { context, namespace });
    return { configmaps: out.configmaps };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List Secrets in a namespace via `k8s.listSecrets` (type + key count only). */
export async function listSecrets(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ secrets?: SecretSummary[]; error?: string }> {
  try {
    const out = await invoke<{ secrets: SecretSummary[] }>("k8s.listSecrets", { context, namespace });
    return { secrets: out.secrets };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List ResourceQuotas in a namespace via `k8s.listResourceQuotas`. */
export async function listResourceQuotas(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ resourcequotas?: ResourceQuotaSummary[]; error?: string }> {
  try {
    const out = await invoke<{ resourcequotas: ResourceQuotaSummary[] }>("k8s.listResourceQuotas", { context, namespace });
    return { resourcequotas: out.resourcequotas };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List LimitRanges in a namespace via `k8s.listLimitRanges`. */
export async function listLimitRanges(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ limitranges?: LimitRangeSummary[]; error?: string }> {
  try {
    const out = await invoke<{ limitranges: LimitRangeSummary[] }>("k8s.listLimitRanges", { context, namespace });
    return { limitranges: out.limitranges };
  } catch (e) {
    return { error: String(e) };
  }
}
