import { parseExtensionOperationRoute, parseExtensionRoute, type ClusterContext } from "@srelens/core";
import { parseDetailRoute, parseEditRoute, parseNewRoute } from "../lib/detailRoute";
import { Icons } from "../lib/icons";
import { useProtection } from "../lib/protection";
import { isClusterScopedRoute } from "../lib/routes";

/**
 * The context name an extension app's page names for itself, resolved the way
 * `ExtensionPage` and `ExtensionOperation` resolve it: by key, else by stable ID
 * while one context holds it, else (a route from before keys) the display name
 * itself. `undefined` when the route is not an app's page; `null` when it names
 * a cluster the list does not hold, or not alone: the page opens nothing then,
 * so there is nothing for the strip to be about.
 *
 * ponytail: a route from before keys names a display name, and two contexts can
 * share one. The page opens the first; `protectionFor` answers for any of them,
 * which is the safe side. Resolve it by `find` here if the strip ever has to
 * agree with the page about which.
 */
function appPageCluster(route: string, contexts: readonly ClusterContext[]): string | null | undefined {
  const operation = parseExtensionOperationRoute(route);
  if (operation) return contexts.find((c) => c.key === operation.contextKey)?.name ?? null;
  const page = parseExtensionRoute(route);
  if (!page) return undefined;
  if (page.contextKey !== undefined) return contexts.find((c) => c.key === page.contextKey)?.name ?? null;
  if (page.clusterId !== undefined) {
    const holders = contexts.filter((c) => c.stableId === page.clusterId);
    return holders.length === 1 ? holders[0].name : null;
  }
  return page.context;
}

/**
 * The cluster a tab is about, for the strip (#101): the one pinned in its
 * route (a detail tab, the editor, "New resource"), which is what it shows
 * whatever the rail says; the one an extension app's page names in its route,
 * which is likewise independent of the rail; else the rail's, for a route that
 * follows it — the Agent's included, since the agent acts on that cluster; else
 * none, for an app-level screen such as Home, Settings or Connections.
 *
 * `contexts` resolves an app page's route, which names its cluster by a key or
 * a stable ID that `protectionFor` does not read.
 */
export function protectedStripCluster(
  route: string,
  activeContextName: string | null,
  contexts: readonly ClusterContext[],
): string | null {
  const pinned = parseDetailRoute(route)?.cluster ?? parseEditRoute(route)?.cluster ?? parseNewRoute(route)?.cluster;
  if (pinned) return pinned;
  const page = appPageCluster(route, contexts);
  if (page !== undefined) return page;
  return isClusterScopedRoute(route) ? activeContextName : null;
}

/**
 * One line above a protected cluster's tab, so the mode is never a surprise
 * at the moment a dialog asks for the name. A note, not a live region:
 * switching tabs must not announce it again.
 */
export function ProtectedStrip({ contextName }: { contextName: string | null }) {
  const protection = useProtection(contextName);
  if (!protection) return null;
  return (
    <div role="note" className="flex shrink-0 items-center gap-2 border-b border-warn/40 bg-warn-wash px-3 py-1 text-[0.75rem] text-warn">
      <Icons.shield size={12} aria-hidden="true" className="shrink-0" />
      <span className="min-w-0 break-words">{`${protection.name} is protected (workspace ${protection.workspace}). Changes ask you to type its name.`}</span>
    </div>
  );
}
