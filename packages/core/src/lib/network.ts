import { invokeCapability, type Invoker } from "../transport/transport";

/** Ingress row — mirrors `crates/kube/src/ingresses.rs`. */
export interface IngressSummary {
  name: string;
  namespace: string;
  class: string;
  hosts: string;
  address: string;
  ports: string;
  age: string;
}

/** EndpointSlice row — mirrors `crates/kube/src/endpointslices.rs`. */
export interface EndpointSliceSummary {
  name: string;
  namespace: string;
  addressType: string;
  /** Ready-over-total endpoint count, e.g. "3/3". */
  endpoints: string;
  ports: string;
  service: string;
  age: string;
}

/** NetworkPolicy row — mirrors `crates/kube/src/networkpolicies.rs`. */
export interface NetworkPolicySummary {
  name: string;
  namespace: string;
  podSelector: string;
  ingress: number;
  egress: number;
  policyTypes: string;
  age: string;
}

/** List Ingresses in a namespace via `k8s.listIngresses`. */
export async function listIngresses(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ ingresses?: IngressSummary[]; error?: string }> {
  try {
    const out = await invoke<{ ingresses: IngressSummary[] }>("k8s.listIngresses", { context, namespace });
    return { ingresses: out.ingresses };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List EndpointSlices in a namespace via `k8s.listEndpointSlices`. */
export async function listEndpointSlices(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ endpointslices?: EndpointSliceSummary[]; error?: string }> {
  try {
    const out = await invoke<{ endpointslices: EndpointSliceSummary[] }>("k8s.listEndpointSlices", { context, namespace });
    return { endpointslices: out.endpointslices };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List NetworkPolicies in a namespace via `k8s.listNetworkPolicies`. */
export async function listNetworkPolicies(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ networkpolicies?: NetworkPolicySummary[]; error?: string }> {
  try {
    const out = await invoke<{ networkpolicies: NetworkPolicySummary[] }>("k8s.listNetworkPolicies", { context, namespace });
    return { networkpolicies: out.networkpolicies };
  } catch (e) {
    return { error: String(e) };
  }
}
