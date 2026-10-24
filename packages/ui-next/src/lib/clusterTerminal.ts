import { isTauri, type ClusterContext } from "@srelens/core";
import { protectionFor } from "./protection";
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
 * Whether the host can make a local shell read-only here (#101). Its `kubectl`
 * and `helm` guards are POSIX `sh` scripts (`crates/streams/src/terminal_scope.rs`),
 * and a Windows desktop has none. Read at call time, the same test
 * `ResourceMenu`'s `runsInTerminal` makes.
 */
export function localShellCanBeReadOnly(): boolean {
  return !(typeof navigator !== "undefined" && navigator.userAgent.includes("Windows"));
}

/** A shell held for the reader's typed name. See {@link openClusterTerminal}. */
export interface PendingClusterTerminal {
  cluster: ClusterContext;
  /** Read when the reader acted, not when they answer. */
  namespace: string | undefined;
  /** The tab the reader acted on. */
  tab: string;
  /** Under that tab's page (#942), or on the Terminals tab. */
  inPanel: boolean;
}

let pending: PendingClusterTerminal | null = null;
const pendingListeners = new Set<() => void>();

function hold(next: PendingClusterTerminal | null): void {
  pending = next;
  for (const listener of [...pendingListeners]) listener();
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
 *
 * On a protected cluster a new shell opens read-only (#101). On Windows the
 * host cannot do that, so the shell is held instead: `ClusterTerminalConfirm`
 * asks for the cluster's name, and {@link answerClusterTerminal} opens it where
 * it was asked for. It is held rather than asked here because the status bar
 * and ⌘J reach this from outside any component, the shape `settingsRequest`
 * has. Toggling or bringing back a live shell asks nothing: that shell was
 * confirmed when it started.
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
  const namespace = terminalNamespace(tabNamespaces(tab, cluster.stableId));
  if (!localShellCanBeReadOnly() && protectionFor(cluster.name)) {
    hold({ cluster, namespace, tab, inPanel });
    return;
  }
  await start(cluster, namespace, tab, inPanel);
}

/** The shell waiting on the reader's typed name, or `null`. The same object
 *  until it changes, for `useSyncExternalStore`. */
export function pendingClusterTerminal(): PendingClusterTerminal | null {
  return pending;
}

/** Hear when a shell starts or stops waiting. Returns the release. */
export function onClusterTerminalPending(listener: () => void): () => void {
  pendingListeners.add(listener);
  return () => {
    pendingListeners.delete(listener);
  };
}

/**
 * The reader's answer for the held shell. Either way it stops waiting. A
 * cluster paused while the question was up is not opened: pausing is the
 * reader asking this window to leave it alone.
 */
export async function answerClusterTerminal(open: boolean): Promise<void> {
  const asked = pending;
  hold(null);
  if (open && asked && canOpenClusterTerminal(asked.cluster)) {
    await start(asked.cluster, asked.namespace, asked.tab, asked.inPanel);
  }
}

/** Start a shell, then show it under the tab's page or on the Terminals tab. */
async function start(cluster: ClusterContext, namespace: string | undefined, tab: string, inPanel: boolean): Promise<void> {
  const id = await startLocalSession({ context: cluster.name, namespace });
  if (inPanel) showTerminalPanel(tab, id);
  else openTab("/terminals", { clusterName: cluster.name });
}
