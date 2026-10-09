import { isTauri, type ClusterContext } from "@srelens/core";
import { startLocalSession } from "./sessions";
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

/**
 * Open a shell on this machine that is bound to `cluster`, and show it (#846).
 *
 * One act from anywhere in the window — the status bar, a chord — rather than
 * a screen change and a dialog. The shell is confined to this one cluster by
 * the host: its kubeconfig holds only this context, and `kubectl` and `helm`
 * in it refuse to be pointed at another.
 *
 * `cluster` is the caller's, read when the reader acted. Nothing here re-reads
 * the active cluster after the await, for the reason `NewSessionMenu` pins its
 * own: a shell opened on whatever the rail says by then is a shell on a cluster
 * the reader did not ask for.
 *
 * Started, then shown, as the node actions do: a shell that could not be opened
 * is a `closed` row on `/terminals` saying why, which is worth landing on.
 */
export async function openClusterTerminal(cluster: ClusterContext): Promise<void> {
  const tab = currentWorkspace().activeId;
  await startLocalSession({
    context: cluster.name,
    namespace: terminalNamespace(tabNamespaces(tab, cluster.stableId)),
  });
  openTab("/terminals", { clusterName: cluster.name });
}
