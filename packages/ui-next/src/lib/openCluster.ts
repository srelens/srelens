import type { ClusterContext } from "@srelens/core";
import { currentWorkspace, openTab, setActiveCluster, setWorkspaceClusters } from "./tabsStore";

/**
 * Open a cluster: put it in this workspace, focus it, and open its overview.
 *
 * **The workspace step is not a flourish.** Both connection screens list every
 * context on the machine, including ones no workspace holds, and
 * `setActiveCluster` refuses an id the workspace does not have — so without it
 * `Open` on exactly those rows would do nothing at all, silently. That is the
 * whole reason this is three calls rather than one.
 *
 * **Here rather than on either screen.** `/connections`' row action and
 * `/connect`'s card row were two copies of this, with the same doc comment
 * written out twice — the same argument that promoted `STATUS` and `bySource`
 * into `screens/connections/clusterText`. They had not drifted yet, and two
 * copies of one gesture is how the two screens start disagreeing about what
 * opening a cluster means.
 *
 * The workspace is read HERE, at the moment of the click, rather than taken
 * from a `useTabs()` subscription in the calling screen: neither screen renders
 * anything from the workspace, so a subscription bought them only the risk of
 * acting on a stale copy of the cluster list.
 */
export function openCluster(context: ClusterContext): void {
  const workspace = currentWorkspace();
  const id = context.stableId;
  if (!workspace.clusters.includes(id)) {
    setWorkspaceClusters(workspace.id, [...workspace.clusters, id]);
  }
  setActiveCluster(id);
  // By NAME, not by stableId: core's `list*` and `watchResource` all take a
  // context name, and the tab is what carries it to them (#265).
  openTab("/overview", { clusterName: context.name });
}
