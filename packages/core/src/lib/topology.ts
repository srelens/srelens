import { invokeCapability, type Invoker } from "../transport/transport";

/**
 * The Topology screen's graph — mirrors `crates/kube/src/topology.rs`.
 *
 * The joins live in Rust because the list capabilities do not carry the fields
 * they need: no Service summary has `spec.selector`, no Ingress summary names
 * its backend Services. Doing it here would mean one `getObject` per object on
 * every render. See that module for the reasoning, and for why the graph has
 * four lanes rather than the mock's seven.
 */

/** Which column a node stands in, left to right. */
export type TopologyLane = "route" | "service" | "workload" | "replicaset" | "external";

/**
 * How well a node is serving, from ready-over-desired and nothing else.
 *
 * `unknown` is not a failure to read: an Ingress and a Service have no
 * replicas of their own, and a workload scaled to zero has none to judge.
 */
export type TopologyHealth = "ok" | "degraded" | "failing" | "unknown";

/**
 * `routes` is traffic reaching something, `owns` is a controller having made
 * it, `calls` is one thing depending on another.
 */
export type TopologyEdgeKind = "routes" | "owns" | "calls";

/**
 * HOW an edge is known.
 *
 * `topology` is the API server saying so outright — an ownerReference, a
 * selector, an Ingress rule. `declared` is a host named in configuration: the
 * workload was BUILT to reach it, which is not the same as it ever having done
 * so. `allowed` is a NetworkPolicy permitting it, and `observed` is telemetry
 * having measured it — neither has a producer yet.
 *
 * The screen must draw these differently. A diagram that renders a measured
 * call and a string found in an environment variable identically is worse than
 * one with fewer edges, because a reader trusts both equally.
 */
export type TopologyProvenance = "topology" | "declared" | "allowed" | "observed";

export interface TopologyNode {
  /** `Kind/namespace/name`, and what every edge refers to. */
  id: string;
  kind: string;
  /**
   * What to draw. A ReplicaSet shows its revision (`rev 119`) rather than the
   * generated hash nobody reads; {@link id} keeps the real name.
   */
  name: string;
  namespace: string;
  lane: TopologyLane;
  /** The line under the name: `9/12`, `:80`, `3/3 ready`. */
  detail: string;
  /** Present only for kinds that have replicas to count. */
  ready: number | null;
  desired: number | null;
  health: TopologyHealth;
}

export interface TopologyEdge {
  from: string;
  to: string;
  kind: TopologyEdgeKind;
  provenance: TopologyProvenance;
  /** The health of the node this edge points at, so a path can be coloured
   *  without walking back to the node table. */
  health: TopologyHealth;
}

export interface TopologyGraph {
  nodes: TopologyNode[];
  edges: TopologyEdge[];
}

/**
 * The route/service/workload/revision/dependency graph of one or more
 * namespaces, via `k8s.topologyGraph`.
 *
 * Several namespaces, because a dependency rarely respects the boundary: a
 * `checkout` calling `payments-api.payments.svc` is only half a picture with
 * `payments` left out. Still a list rather than "all" — every namespace on a
 * real cluster is thousands of nodes, which is not a picture of anything — and
 * an empty list draws nothing rather than everything.
 */
export async function topologyGraph(
  context: string,
  namespaces: string[],
  invoke: Invoker = invokeCapability,
): Promise<{ graph?: TopologyGraph; error?: string }> {
  try {
    const out = await invoke<TopologyGraph>("k8s.topologyGraph", { context, namespaces });
    return { graph: { nodes: out.nodes, edges: out.edges } };
  } catch (e) {
    return { error: String(e) };
  }
}
