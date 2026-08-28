import { invokeCapability, type Invoker } from "../transport/transport";
import type { PodSummary } from "./workloads";

const BINARY_UNITS: Array<[string, number]> = [
  ["Pi", 2 ** 50],
  ["Ti", 2 ** 40],
  ["Gi", 2 ** 30],
  ["Mi", 2 ** 20],
  ["Ki", 2 ** 10],
];
const UNIT_FACTORS: Record<string, number> = {
  Ki: 2 ** 10, Mi: 2 ** 20, Gi: 2 ** 30, Ti: 2 ** 40, Pi: 2 ** 50, Ei: 2 ** 60,
  k: 1e3, M: 1e6, G: 1e9, T: 1e12, P: 1e15, E: 1e18,
};

/** Parse a Kubernetes storage Quantity ("10Gi", "5G", "7586630231655") to bytes. */
function quantityToBytes(quantity: string): number | null {
  const m = /^([0-9.]+)\s*([a-zA-Z]*)$/.exec((quantity ?? "").trim());
  if (!m) return null;
  const n = parseFloat(m[1]);
  if (Number.isNaN(n)) return null;
  const unit = m[2];
  if (unit === "") return n;
  return UNIT_FACTORS[unit] ? n * UNIT_FACTORS[unit] : n;
}

/**
 * Render a storage Quantity as a compact binary size ("6.9Ti", "10Gi"). Raw
 * byte quantities (as MinIO and some provisioners report) become the nearest
 * Ki/Mi/Gi/Ti/Pi; values below 1Ki stay as bytes, and an empty value shows "—".
 */
export function formatStorageSize(quantity: string): string {
  if (!quantity) return "—";
  const bytes = quantityToBytes(quantity);
  if (bytes == null) return quantity;
  if (bytes === 0) return "0";
  for (const [suffix, factor] of BINARY_UNITS) {
    if (bytes >= factor) {
      const value = bytes / factor;
      return `${value.toFixed(1).replace(/\.0$/, "")}${suffix}`;
    }
  }
  return `${bytes}`;
}

/** PersistentVolumeClaim row — mirrors `crates/kube/src/pvcs.rs`. */
export interface PvcSummary {
  name: string;
  namespace: string;
  status: string;
  capacity: string;
  accessModes: string;
  storageClass: string;
  volume: string;
  age: string;
}

/** PersistentVolume row — mirrors `crates/kube/src/persistentvolumes.rs` (cluster-scoped). */
export interface PvSummary {
  name: string;
  capacity: string;
  accessModes: string;
  reclaimPolicy: string;
  status: string;
  /** Bound claim as "namespace/name", empty when unbound. */
  claim: string;
  storageClass: string;
  age: string;
}

/** StorageClass row — mirrors `crates/kube/src/storageclasses.rs` (cluster-scoped). */
export interface StorageClassSummary {
  name: string;
  provisioner: string;
  reclaimPolicy: string;
  volumeBindingMode: string;
  default: boolean;
  age: string;
}

/** List PVCs in a namespace via `k8s.listPersistentVolumeClaims`. */
export async function listPersistentVolumeClaims(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ persistentvolumeclaims?: PvcSummary[]; error?: string }> {
  try {
    const out = await invoke<{ persistentvolumeclaims: PvcSummary[] }>("k8s.listPersistentVolumeClaims", {
      context,
      namespace,
    });
    return { persistentvolumeclaims: out.persistentvolumeclaims };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List cluster PersistentVolumes via `k8s.listPersistentVolumes`. */
export async function listPersistentVolumes(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ persistentvolumes?: PvSummary[]; error?: string }> {
  try {
    const out = await invoke<{ persistentvolumes: PvSummary[] }>("k8s.listPersistentVolumes", { context });
    return { persistentvolumes: out.persistentvolumes };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List pods that mount a PVC via `k8s.podsForPvc` (the claim's consumers). */
export async function podsForPvc(
  context: string,
  namespace: string,
  pvc: string,
  invoke: Invoker = invokeCapability,
): Promise<{ pods?: PodSummary[]; error?: string }> {
  try {
    const out = await invoke<{ pods: PodSummary[] }>("k8s.podsForPvc", { context, namespace, pvc });
    return { pods: out.pods };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List cluster StorageClasses via `k8s.listStorageClasses`. */
export async function listStorageClasses(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ storageclasses?: StorageClassSummary[]; error?: string }> {
  try {
    const out = await invoke<{ storageclasses: StorageClassSummary[] }>("k8s.listStorageClasses", { context });
    return { storageclasses: out.storageclasses };
  } catch (e) {
    return { error: String(e) };
  }
}
