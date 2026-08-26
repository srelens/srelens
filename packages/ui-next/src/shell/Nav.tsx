import { useEffect, useMemo, useState } from "react";
import { listCrds, type ClusterContext, type CrdRef } from "@srelens/core";
import { Mark, ResourceTree, Sidebar, StatusPill, type ResourceNode, type StatusKind } from "@srelens/ui-kit";
import { Icons } from "../lib/icons";
import { useMark } from "../lib/marks";
import { openTab, useActiveCluster, useTabs } from "../lib/tabsStore";
import { crdNodes, glyph, INVESTIGATE, kindNodes, NAV_GROUPS, routeForNode } from "../lib/tree";
import { useResource } from "../lib/useResource";
import { seedExpandedOnce, toggleExpanded, useWorkspaceView, type LinkState } from "../lib/workspace";

/** The one leaf the "Custom resources" group holds when discovery has failed. */
const CRD_ERROR_ID = "crd-error";

export interface NavProps {
  /** Every cluster the machine knows. The active one is looked up in here by `stableId`. */
  contexts: ClusterContext[];
}

/** How each link state reads, and which of the kit's five kinds draws it. */
const LINK: Record<LinkState, { word: string; kind: StatusKind }> = {
  connected: { word: "Connected", kind: "success" },
  connecting: { word: "Connecting", kind: "info" },
  disconnected: { word: "Disconnected", kind: "neutral" },
  error: { word: "Error", kind: "danger" },
};

/** Before anything has probed the cluster there is nothing to claim about it. */
const UNKNOWN = { word: "Unknown", kind: "neutral" } as const;

/**
 * The groups that stand open the first time a window is used: everything the
 * tree builds with children, minus the two that ask to start shut.
 */
const DEFAULT_EXPANDED = [...NAV_GROUPS.map((g) => g.id), "investigate"];

/**
 * The id of the node the active tab is on, or `undefined` when the tab is on
 * something the tree does not offer (a resource detail, settings, a terminal).
 *
 * Found by asking each leaf where it goes rather than by turning the route back
 * into an id: `routeForNode` is the one direction that has to be right, and a
 * second mapping written the other way round is a second thing to keep in step.
 */
function nodeForRoute(nodes: ResourceNode[], crds: CrdRef[], route: string): string | undefined {
  for (const node of nodes) {
    if (node.children) {
      const inside = nodeForRoute(node.children, crds, route);
      if (inside) return inside;
    } else if (routeForNode(node.id, crds) === route) {
      return node.id;
    }
  }
  return undefined;
}

/**
 * The sidebar: whose cluster this is, a filter, and everything in it worth
 * opening — the built-in kinds, whatever CRDs this cluster has, and the app's
 * own screens.
 *
 * All of the drawing is the kit's; what lives here is the wiring the kit is not
 * allowed to know. The shape of the tree is `lib/tree.ts`, which is pure and
 * tested as data; this reads the active cluster out of the tab store, asks core
 * for the CRDs, opens tabs, and keeps the folds in the workspace view.
 *
 * Two decisions worth naming. Activating a row opens a *preview* tab, the way
 * a single click in an editor's file tree does: walking down a sidebar is
 * browsing, and browsing twenty kinds should not leave twenty tabs behind —
 * `openTab` promotes the preview as soon as the row is opened for real.
 *
 * And the folds are stored, not defaulted. The kit's tree takes `expanded` as
 * the whole truth when it is given at all, so an empty list would mean every
 * group shut on first launch; the workspace view is therefore seeded, once
 * ever for the window's lifetime (`seedExpandedOnce`, in `workspace.ts`) with
 * the groups that should stand open. Once ever rather than once per mount,
 * because "the user closed all six" is a state the sidebar has to be able to
 * stay in across a remount, and a per-mount guard cannot tell that state apart
 * from "nothing has opened anything yet" — both leave `expanded` empty.
 */
export function Nav({ contexts }: NavProps) {
  const activeCluster = useActiveCluster();
  const ctx = contexts.find((c) => c.stableId === activeCluster) ?? null;
  const view = useWorkspaceView();
  const [query, setQuery] = useState("");
  const mark = useMark(ctx?.stableId ?? "", ctx?.name ?? "");

  // Subscribes the sidebar to the strip: which row is highlighted is a fact
  // about the active tab, and that changes from a dozen places that are not here.
  const { tabs, activeId } = useTabs();
  const route = tabs.find((t) => t.id === activeId)?.route ?? "/";

  const name = ctx?.name;
  const discovery = useResource<CrdRef[]>(
    async () => {
      if (!name) return [];
      const out = await listCrds(name);
      // `listCrds` reports failure in the result rather than by rejecting, and
      // an empty tree is not the same news as "we were not allowed to look".
      if (out.error) throw new Error(out.error);
      return out.crds ?? [];
    },
    [name],
  );
  const crds = useMemo(() => discovery.data ?? [], [discovery.data]);

  // A failed discovery is not rendered as the tree-level `error` — that kit
  // prop replaces the whole tree with an announced failure, and CRD discovery
  // commonly fails for a reason that has nothing to do with the built-in
  // kinds (ordinary RBAC does not grant list on CRDs). The built-ins and
  // Investigate must survive that; only "Custom resources" loses its
  // contents, to one leaf that retries.
  const crdChildren: ResourceNode[] =
    discovery.status === "error"
      ? [{ id: CRD_ERROR_ID, label: "Custom resources unavailable — retry", icon: Icons.warn }]
      : crdNodes(crds);

  const nodes = useMemo<ResourceNode[]>(
    () => [
      ...kindNodes(),
      { id: "crds", label: "Custom resources", icon: Icons.crds, defaultExpanded: false, children: crdChildren },
      {
        id: "investigate",
        label: "Investigate",
        icon: Icons.investigate,
        children: INVESTIGATE.map((i) => ({ id: `route:${i.route}`, label: i.label, icon: glyph(i.id) })),
      },
    ],
    [crds, crdChildren],
  );

  useEffect(() => {
    seedExpandedOnce(DEFAULT_EXPANDED);
  }, []);

  const link = ctx ? (view.links[ctx.stableId] ? LINK[view.links[ctx.stableId].state] : UNKNOWN) : UNKNOWN;

  return (
    <Sidebar
      label="Cluster navigation"
      query={query}
      onQueryChange={setQuery}
      emptyTitle="No cluster selected"
      emptyHint="Pick a cluster from the rail to browse what is in it."
      header={
        ctx && (
          <div className="flex items-center gap-2">
            <Mark
              name={mark.name}
              short={mark.short}
              color={mark.color}
              size="sm"
              decorative
              withBadge={mark.withText}
              icon={mark.mark === "icon" && mark.icon ? glyph(mark.icon) : undefined}
              imageSrc={mark.mark === "image" ? mark.imageSrc : undefined}
            />
            <span className="min-w-0 flex-1 truncate text-[0.8125rem] font-medium">{ctx.name}</span>
            <StatusPill status={link.word} kind={link.kind} />
          </div>
        )
      }
    >
      {ctx && (
        <ResourceTree
          label="Cluster resources"
          nodes={nodes}
          active={nodeForRoute(nodes, crds, route)}
          onActivate={(id) => {
            // Handled before `routeForNode` is even consulted: the retry leaf
            // is not a destination, it is the group's own failure reporting
            // itself, and `routeForNode` returns `null` for it precisely so a
            // caller that forgot this branch opens no tab either.
            if (id === CRD_ERROR_ID) {
              discovery.reload();
              return;
            }
            const next = routeForNode(id, crds);
            // A tab of its own, not a preview. The preview pattern — one
            // italic tab the next click replaces — keeps a strip tidy while
            // browsing, and it costs the reader the thing the strip is for:
            // clicking four kinds to compare them left one tab, not four.
            // The strip scrolls, so accumulating is affordable.
            if (next) openTab(next, { clusterName: ctx.name });
          }}
          expanded={view.expanded}
          onExpandedChange={toggleExpanded}
          query={query}
        />
      )}
    </Sidebar>
  );
}
