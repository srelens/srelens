import { parseDetailRoute, parseEditRoute, parseNewRoute } from "../lib/detailRoute";
import { Icons } from "../lib/icons";
import { useProtection } from "../lib/protection";
import { isClusterScopedRoute } from "../lib/routes";

/**
 * The cluster a tab is about, for the strip (#101): the one pinned in its
 * route (a detail tab, the editor, "New resource"), which is what it shows
 * whatever the rail says; else the rail's, for a route that follows it — the
 * Agent's included, since the agent acts on that cluster; else none, for an
 * app route such as Home, Settings or Connections.
 *
 * ponytail: an extension route that names its own context gets the rail's
 * cluster here; read its `contextKey` if one is ever opened beside another
 * cluster's strip.
 */
export function protectedStripCluster(route: string, activeContextName: string | null): string | null {
  const pinned = parseDetailRoute(route)?.cluster ?? parseEditRoute(route)?.cluster ?? parseNewRoute(route)?.cluster;
  if (pinned) return pinned;
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
      <span>{`${protection.name} is protected (workspace ${protection.workspace}). Changes ask you to type its name.`}</span>
    </div>
  );
}
