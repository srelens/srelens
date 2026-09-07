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
 * The groups, mirroring the classic sidebar's `NAV_SECTIONS` so that someone
 * running both builds during the migration finds the same kinds in the same
 * places. Only the routes moved (see {@link CLUSTER_ROUTES}).
 */
export const NAV_GROUPS: ReadonlyArray<{ id: string; label: string; kinds: ResourceKind[] }> = [
  { id: "cluster", label: "Cluster", kinds: ["nodes", "namespaces", "events"] },
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

/**
 * The three rows in the Cluster group that are not kinds at all.
 *
 * Core lists `overview`, `portforwards` and `helmreleases` in `ResourceKind`
 * for the classic sidebar's sake, but none of them has a Kubernetes kind behind
 * it (`K8S_KIND` is `""` for all three) and none is a `/k/<kind>` list here:
 * they are screens, at `/overview`, `/forwards` and `/helm`. They keep core's
 * labels, because they are the same three things by another door.
 */
const CLUSTER_ROUTES: ReadonlyArray<{ kind: ResourceKind; route: string; before?: true }> = [
  { kind: "overview", route: "/overview", before: true },
  { kind: "portforwards", route: "/forwards" },
  { kind: "helmreleases", route: "/helm" },
];

/** Where the app can be sent that is not a list of resources. */
export const INVESTIGATE: ReadonlyArray<{ id: string; label: string; route: string }> = [
  { id: "control", label: "Control room", route: "/" },
  { id: "incidents", label: "Incidents", route: "/incidents" },
  { id: "topology", label: "Topology", route: "/topology" },
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

const kindNode = (kind: ResourceKind): ResourceNode => ({
  id: `kind:${kind}`,
  label: RESOURCE_LABELS[kind],
  icon: glyph(kind),
});

const routeNode = (kind: ResourceKind, route: string): ResourceNode => ({
  id: `route:${route}`,
  label: RESOURCE_LABELS[kind],
  icon: glyph(kind),
});

/** The built-in half of the tree: the six groups, open to begin with. */
export function kindNodes(): ResourceNode[] {
  return NAV_GROUPS.map((group) => {
    const kinds = group.kinds.map(kindNode);
    const children =
      group.id === "cluster"
        ? [
            ...CLUSTER_ROUTES.filter((r) => r.before).map((r) => routeNode(r.kind, r.route)),
            ...kinds,
            ...CLUSTER_ROUTES.filter((r) => !r.before).map((r) => routeNode(r.kind, r.route)),
          ]
        : kinds;
    return { id: group.id, label: group.label, icon: glyph(group.id), children };
  });
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
export function crdNodes(crds: CrdRef[]): ResourceNode[] {
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
        .map((crd) => ({ id: `crd:${crd.name}`, label: crd.kind, icon: Icons.fallback })),
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
