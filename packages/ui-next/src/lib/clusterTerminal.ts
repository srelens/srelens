import { isTauri, type ClusterContext } from "@srelens/core";
import { getSessions, startLocalSession } from "./sessions";
import { closeTerminalPanel, setTerminalPanelMinimized, showTerminalPanel, terminalPanel } from "./terminalPanel";
import { currentWorkspace, isClusterPaused, openTab, tabNamespaces } from "./tabsStore";

/**
 * The namespace a terminal opened from a tab should start in: the tab's, when
 * it is looking at exactly one. Several, all, or no choice made yet is not one
 * namespace, and picking one of them for the reader would be a guess that
 * `kubectl get pods` then answers as though it were theirs.
 */
export function terminalNamespace(selection: readonly string[] | undefined): string | undefined {
  return selection?.length === 1 ? selection[0] : undefined;
}

/**
 * Whether a terminal can be opened for `cluster` from where the reader is.
 *
 * The shell runs on the reader's own machine, which only the desktop app has;
 * and a paused cluster is one the reader has asked this window to leave alone.
 */
export function canOpenClusterTerminal(cluster: ClusterContext | undefined): cluster is ClusterContext {
  return cluster !== undefined && isTauri() && !isClusterPaused(cluster.stableId);
}

/** Whether a route is the Terminals tab, which shows sessions itself. */
function isTerminalsRoute(route: string | undefined): boolean {
  return route === "/terminals" || route?.startsWith("/terminals?") === true;
}

/**
 * Open a shell on this machine that is bound to `cluster`, and show it (#846)
 * — under the page the reader is on, in that tab's terminal panel (#942).
 *
 * One act from anywhere in the window — the status bar, a chord — rather than
 * a screen change and a dialog. The shell is confined to this one cluster by
 * the host: its kubeconfig holds only this context, and `kubectl` and `helm`
 * in it refuse to be pointed at another.
 *
 * **A toggle, per tab.** With the panel open, the same act closes it, leaving
 * the shell running; minimized, it brings it back up. With it closed, it opens again on the shell it last
 * showed, if that is still on the same cluster and has not ended — so the
 * reader gets their shell back rather than a second one. Otherwise a new shell
 * is started. On the Terminals tab itself there is no panel: the new session
 * is shown there, as before.
 *
 * `cluster` is the caller's, read when the reader acted. Nothing here re-reads
 * the active cluster after the await, for the reason `NewSessionMenu` pins its
 * own: a shell opened on whatever the rail says by then is a shell on a cluster
 * the reader did not ask for. The tab is the one the reader acted on, too, so
 * the panel opens under the page that asked for it.
 *
 * Started, then shown, as the node actions do: a shell that could not be opened
 * is a `closed` session saying why, which is worth landing on.
 */
export async function openClusterTerminal(cluster: ClusterContext): Promise<void> {
  const workspace = currentWorkspace();
  const tab = workspace.activeId;
  const route = workspace.tabs.find((t) => t.id === tab)?.route;
  const inPanel = !isTerminalsRoute(route);
  if (inPanel) {
    const panel = terminalPanel(tab);
    const kept = panel && getSessions().find((s) => s.id === panel.shellId);
    // Only a live shell on the cluster asked for is the reader's to toggle or
    // bring back. One on another cluster, or one that has ended, is replaced
    // by a new shell on this one — the act asked for a terminal on `cluster`.
    const reusable = kept && kept.state !== "closed" && kept.context === cluster.name;
    if (panel?.open && reusable) {
      // Minimized is still open, but not what the reader is asking to close:
      // the act brings it back up.
      if (panel.minimized) setTerminalPanelMinimized(tab, false);
      else closeTerminalPanel(tab);
      return;
    }
    if (reusable) {
      showTerminalPanel(tab, kept.id);
      return;
    }
  }
  const id = await startLocalSession({
    context: cluster.name,
    namespace: terminalNamespace(tabNamespaces(tab, cluster.stableId)),
  });
  if (inPanel) showTerminalPanel(tab, id);
  else openTab("/terminals", { clusterName: cluster.name });
}
