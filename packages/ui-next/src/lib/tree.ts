import type { ReactNode } from "react";
import { RESOURCE_LABELS, type CrdRef, type ResourceKind } from "@srelens/core";
import type { IconComponent, ResourceNode } from "@srelens/ui-kit";
import { Icons } from "./icons";

/**
 * What the sidebar offers, as data.
 *
 * Pure on purpose: the tree the shell draws is a value, so the shape of the
 * navigation can be asserted without rendering anything, and `Nav` is left with
 * only the wiring — the store, the CRD load, the tab it opens. The kit's
 * `ResourceTree` takes nodes and gives back ids; this module owns both ends of
 * that conversation, `kindNodes`/`crdNodes` building the ids and
 * {@link routeForNode} reading them back.
 *
 * Ids are prefixed by what they are — `kind:`, `crd:`, `crdgroup:`, `route:` —
 * rather than being bare slugs. A tree that mixes built-in kinds, discovered
 * custom resources and plain app routes in one flat namespace has no way to
 * tell `pods` the kind from a CRD that happens to be called `pods`, and the
 * prefix is what makes the reverse mapping total instead of a guess.
 */

/**
 * The resource groups: the Kubernetes kinds, in the order the classic sidebar's
 * `NAV_SECTIONS` kept them. The middle band of {@link navNodes}.
 */
export const NAV_GROUPS: ReadonlyArray<{ id: string; label: string; kinds: ResourceKind[] }> = [
  {
    id: "workloads",
    label: "Workloads",
    kinds: ["pods", "deployments", "statefulsets", "daemonsets", "replicasets", "jobs", "cronjobs"],
  },
  {
    id: "config",
    label: "Config",
    kinds: [
      "configmaps",
      "secrets",
      "resourcequotas",
      "limitranges",
      "horizontalpodautoscalers",
      "poddisruptionbudgets",
      "priorityclasses",
      "runtimeclasses",
      "leases",
      "mutatingwebhookconfigurations",
      "validatingwebhookconfigurations",
    ],
  },
  {
    id: "network",
    label: "Network",
    kinds: ["services", "endpointslices", "endpoints", "ingresses", "ingressclasses", "networkpolicies"],
  },
  { id: "storage", label: "Storage", kinds: ["persistentvolumeclaims", "persistentvolumes", "storageclasses"] },
  {
    id: "access",
    label: "Access control",
    kinds: ["serviceaccounts", "clusterroles", "roles", "clusterrolebindings", "rolebindings"],
  },
];

/** The kinds that stand in the cluster band as rows of their own, after Overview. */
export const CLUSTER_KINDS: ResourceKind[] = ["nodes", "namespaces", "events"];

/**
 * A core label in the sentence case the new design's navigation uses.
 *
 * Core keeps Title Case ("Port Forwards") because the classic design draws it,
 * and core is shared, so the mapping lives here. A word with a capital inside
 * it is a Kubernetes name (ConfigMaps, StatefulSets) and is left as it is.
 */
export function navLabel(label: string): string {
  return label.replace(/ ([A-Z][a-z]+)(?= |$)/g, (_, word: string) => ` ${word.toLowerCase()}`);
}

/**
 * The last band's built-in rows, after the apps: screens, not lists of a kind.
 *
 * Core lists `portforwards` and `helmreleases` in `ResourceKind` for the
 * classic sidebar's sake, but neither has a Kubernetes kind behind it: they are
 * screens, at `/forwards` and `/helm`. They keep core's labels, because they
 * are the same things by another door. Home is not here: it is where every new
 * tab starts, not a place in a cluster.
 */
export const TOOLS: ReadonlyArray<{ id: string; label: string; route: string }> = [
  { id: "helmreleases", label: navLabel(RESOURCE_LABELS.helmreleases), route: "/helm" },
  { id: "portforwards", label: navLabel(RESOURCE_LABELS.portforwards), route: "/forwards" },
  { id: "topology", label: "Topology", route: "/topology" },
  { id: "incidents", label: "Incidents", route: "/incidents" },
  { id: "agent", label: "Agent", route: "/agent" },
];

/**
 * The glyph for a name in the app's vocabulary, or the fallback.
 *
 * `Icons` is keyed by concept — `pods`, `workloads`, `incidents` — and every id
 * this module builds a node for is one of those concepts, so one lookup covers
 * groups, kinds and destinations alike. A kind added to core before a glyph is
 * chosen for it draws the fallback rather than nothing.
 */
export function glyph(name: string): IconComponent {
  return (Icons as Record<string, IconComponent | undefined>)[name] ?? Icons.fallback;
}

/**
 * What the sidebar shows at the end of a row, by node id (#402): `kind:pods`,
 * `crd:<name>`, `route:/forwards`. Only leaves read it — a group's row is a
 * fold, not a list, and has nothing to count.
 */
export type NodeCounts = ReadonlyMap<string, ReactNode>;

/** The node's `count` from `counts`, or nothing at all — not a `count: undefined` key. */
const counted = (id: string, counts?: NodeCounts): { count?: ReactNode } =>
  counts?.has(id) ? { count: counts.get(id) } : {};

const kindNode = (kind: ResourceKind, counts?: NodeCounts): ResourceNode => ({
  id: `kind:${kind}`,
  label: navLabel(RESOURCE_LABELS[kind]),
  icon: glyph(kind),
  ...counted(`kind:${kind}`, counts),
});

const routeNode = (kind: ResourceKind, route: string, counts?: NodeCounts): ResourceNode => ({
  id: `route:${route}`,
  label: navLabel(RESOURCE_LABELS[kind]),
  icon: glyph(kind),
  ...counted(`route:${route}`, counts),
});

/** The resource groups as nodes, open to begin with unless the caller keeps the folds. */
export function kindNodes(counts?: NodeCounts): ResourceNode[] {
  return NAV_GROUPS.map((group) => ({
    id: group.id,
    label: group.label,
    icon: glyph(group.id),
    children: group.kinds.map((kind) => kindNode(kind, counts)),
  }));
}

export interface NavInput {
  counts?: NodeCounts;
  /** Each enabled app's entry, from `appNavigation`. */
  apps?: ResourceNode[];
  /** What "Custom resources" holds: {@link crdNodes}, or its failure leaf. */
  crds: ResourceNode[];
  /** Whether the AI Assistant may act on this cluster: no Agent row where it may not (#968). */
  agent: boolean;
}

/**
 * The whole sidebar, in three unlabelled bands (#1001): the cluster itself
 * (Overview, Nodes, Namespaces, Events), its resources (the kind groups, then
 * Custom resources), then apps and tools (each app, Helm releases, Port
 * forwards, Topology, Incidents, Agent). Apps follow the built-ins, as they do
 * in Lens. The kit draws a rule where the band changes.
 */
export function navNodes({ counts, apps = [], crds, agent }: NavInput): ResourceNode[] {
  const inBand = (band: string) => (node: ResourceNode): ResourceNode => ({ ...node, band });
  const tools = TOOLS.filter((tool) => agent || tool.id !== "agent").map((tool) => ({
    id: `route:${tool.route}`,
    label: tool.label,
    icon: glyph(tool.id),
    ...counted(`route:${tool.route}`, counts),
  }));
  return [
    ...[routeNode("overview", "/overview", counts), ...CLUSTER_KINDS.map((kind) => kindNode(kind, counts))].map(inBand("cluster")),
    ...[
      ...kindNodes(counts),
      { id: "crds", label: "Custom resources", icon: Icons.crds, defaultExpanded: false, children: crds },
    ].map(inBand("resources")),
    ...[...apps, ...tools].map(inBand("tools")),
  ];
}

/**
 * Discovered CRDs, gathered under their API group.
 *
 * Flat would be unusable: a cluster with cert-manager, Argo and an operator or
 * two runs to a couple of hundred CRDs, and the group is the only thing that
 * makes that list navigable. Groups and the CRDs inside them are both sorted,
 * because discovery order is the backend's and changes between calls — a tree
 * that reshuffles itself between refreshes cannot be learned.
 *
 * Every group starts folded shut. Expanding all of them by default would drown
 * the built-in kinds above them the moment one operator is installed.
 */
export function crdNodes(crds: CrdRef[], counts?: NodeCounts): ResourceNode[] {
  const groups = new Map<string, CrdRef[]>();
  for (const crd of crds) {
    const key = crd.group || "core";
    const list = groups.get(key);
    if (list) list.push(crd);
    else groups.set(key, [crd]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([group, list]) => ({
      id: `crdgroup:${group}`,
      label: group,
      icon: Icons.crds,
      defaultExpanded: false,
      children: [...list]
        .sort((a, b) => a.kind.localeCompare(b.kind))
        .map((crd) => ({ id: `crd:${crd.name}`, label: crd.kind, icon: Icons.crds, ...counted(`crd:${crd.name}`, counts) })),
    }));
}

/**
 * Where a node id goes, or `null` for a node that goes nowhere.
 *
 * The groups are the `null` cases: the kit's tree already treats a node with
 * children as a fold rather than a destination, so this is the second line of
 * that same rule — a caller that activates a group opens no tab instead of a
 * tab at `/k/crdgroup:cert-manager.io`.
 *
 * A CRD is checked against the list it came from rather than trusted. Node ids
 * outlive the CRDs behind them — a tree rendered before the operator was
 * uninstalled, a persisted fold — and a route to a custom resource this cluster
 * has never heard of is a tab that can only fail.
 */
export function routeForNode(id: string, crds: CrdRef[]): string | null {
  if (id.startsWith("route:")) return id.slice("route:".length) || null;
  if (id.startsWith("kind:")) {
    const kind = id.slice("kind:".length);
    // Events are a screen of their own, not a table of a kind.
    return kind === "events" ? "/events" : `/k/${kind}`;
  }
  if (id.startsWith("crd:")) {
    const name = id.slice("crd:".length);
    return crds.some((crd) => crd.name === name) ? `/k/${name}` : null;
  }
  return null;
}
