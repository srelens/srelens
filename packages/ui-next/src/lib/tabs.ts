import { parseExtensionRoute, type ClusterContext } from "@srelens/core";
import type { TableSort } from "@srelens/ui-kit";
import { describe, isClusterScopedRoute, type TabKind } from "./routes";
import { parseDetailRoute, parseEditRoute, parseNewRoute } from "./detailRoute";

export interface Tab {
  id: string;
  route: string;
  title: string;
  sub?: string;
  kind: TabKind;
  /** A peek, replaced by the next peek — the way an editor previews a file. */
  preview?: boolean;
  /** Cannot be closed. */
  pinned?: boolean;
  /**
   * A resource list's sort, filter string and active filter column. Belongs
   * to the tab, not the screen, so it survives a restart the way the rest of
   * the tab does. Absent until the user sorts or filters — see `setTabView`.
   * A Logs tab keeps its search box here, as `query`.
   */
  view?: { sort?: TableSort | null; filter?: string; filterKey?: string | null; regex?: boolean; query?: string };
  /**
   * The namespace selection this tab is narrowed to, per cluster, keyed by
   * `ClusterContext.stableId`. On the tab, not the cluster: narrowing one tab
   * must never narrow another tab looking at the same cluster. Keyed by
   * cluster because a tab used to follow the rail and show whichever cluster
   * was active. A tab now belongs to the cluster it was opened on (#908, see
   * {@link ClusterStrip}), so it normally holds one entry; documents saved
   * before that hold several, and they are harmless. An empty array is an
   * explicit "all namespaces"; a cluster with no entry follows the
   * default-namespace preference.
   */
  namespaces?: Record<string, string[]>;
  /**
   * When the tab was closed, on a tab in a recently closed list only. It is
   * what orders that list when a cluster's own closed tabs are brought back
   * beside the app-wide ones that stayed (#908): without it one kind had to
   * go first, and the cap then dropped the other however recent it was.
   */
  closedAt?: number;
}

/**
 * One cluster's tabs while another cluster is in focus (#908).
 *
 * A workspace used to have one strip that followed the rail: every tab showed
 * whichever cluster was in focus, so Pods, ConfigMaps and Services opened
 * while working on one cluster were still there, relabelled, on the next —
 * where nobody had opened them. Each cluster now keeps its own. The strip on
 * screen (`Workspace.tabs`) is the one for the cluster in focus, and the rest
 * wait here, keyed by `ClusterContext.stableId`, until their cluster is in
 * focus again. See {@link focusStrip}.
 */
export interface ClusterStrip {
  /** The cluster's own tabs, in strip order. */
  tabs: Tab[];
  /**
   * The id of every tab that was in the strip, in order — the app-wide ones
   * too, which stay on screen rather than being put away. It is what lets the
   * strip come back in the order it was left in, instead of with every
   * app-wide tab moved to one end.
   */
  order: string[];
  activeId: string;
  /** The cluster's own recently closed tabs, most recent first. */
  closed: Tab[];
}

export interface Workspace {
  id: string;
  name: string;
  /**
   * A {@link PALETTE} value (`lib/markSymbols.ts`), drawn beside the name on
   * the title-bar chip and the switcher. Absent means no colour. On the
   * workspace rather than in a setting keyed by its id, so it is saved,
   * restored and deleted with it.
   */
  color?: string;
  /** `ClusterContext.stableId`s. Never display names — see #265. */
  clusters: string[];
  /** Clusters the reader has paused without removing from this workspace. */
  pausedClusters?: string[];
  /** The cluster the sidebar and status bar are about. A `stableId` in `clusters`. */
  activeCluster?: string;
  /**
   * The strip on screen: the tabs of the cluster in focus, with the app-wide
   * tabs and those that name a cluster of their own.
   */
  tabs: Tab[];
  activeId: string;
  /** Recently closed, most recent first, for reopen-closed. */
  closed: Tab[];
  /** The tabs of the clusters that are not in focus, by `stableId`. */
  shelved?: Record<string, ClusterStrip>;
}

export interface TabsState {
  workspaces: Workspace[];
  currentId: string;
}

export const CLOSED_CAP = 12;

let seq = 0;

/**
 * Random rather than a counter. The mock used `t${++uid}`, and a counter
 * restarts at zero on reload while the restored tabs keep the ids the last
 * session gave them — so the first tab opened after a restart collided with
 * one already on screen.
 *
 * `randomUUID` is guarded because it is `[SecureContext]`-only: on a plain
 * http origin — web mode on a LAN address — it is simply absent, and the
 * unguarded call threw while this module was still evaluating, so importing
 * `@srelens/ui-next` rejected and the window came up blank with no way back.
 * Core guards the same call in `transport/webTransport.ts`. Uniqueness is all
 * a tab id needs, so the fallback is time, a counter and randomness: unique
 * within a session by the counter, and across sessions by the other two.
 */
export function newId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") return crypto.randomUUID();
  return `t-${Date.now().toString(36)}-${(seq++).toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function makeTab(route: string, opts: { preview?: boolean; clusterName?: string } = {}): Tab {
  const info = describe(route, opts.clusterName);
  const tab: Tab = { id: newId(), route, title: info.title, kind: info.kind };
  if (info.sub) tab.sub = info.sub;
  if (info.pinned) tab.pinned = true;
  if (opts.preview) tab.preview = true;
  return tab;
}

/**
 * The tab a caller asked for, labelled for the cluster they named.
 *
 * **Why this exists at all.** `openTab` dedupes by ROUTE — deliberately, and
 * right for most of its callers: a detail route, a list route and `/overview`
 * are each meant to be one tab in the strip. But `makeTab` spends
 * `clusterName` on the `sub` at CREATION, so a tab reused for a second cluster
 * kept the first one's label: the overview rendered cluster B while the strip
 * read cluster A, which is the one place a reader looks to know which cluster
 * they are in.
 *
 * **Through `describe`, never `sub = clusterName`.** `describe` drops `sub`
 * entirely for an app-scoped route (`{ route, ...app }` spreads no `sub`),
 * because `/connections` and `/connect` are not about a cluster — assigning the
 * caller's string straight onto the tab would put a cluster name under the
 * strip's `Connections` label. The route table stays the one place that decides
 * whether a route carries a cluster, and the title comes back through the same
 * call so a route that ever spends `clusterName` on its title follows too.
 *
 * No `clusterName` means the caller said nothing about a cluster — a keyboard
 * shortcut, a reopened tab — and the tab keeps the label it has rather than
 * being blanked. Returns the input untouched when nothing changed, which is
 * what lets `openTab` decide by identity whether to emit: one subscriber of
 * that store writes a file.
 */
export function relabel(tab: Tab, clusterName?: string): Tab {
  if (!clusterName || parseExtensionRoute(tab.route)) return tab;
  const info = describe(tab.route, clusterName);
  if (info.title === tab.title && info.sub === tab.sub) return tab;
  const next: Tab = { ...tab, title: info.title };
  if (info.sub) next.sub = info.sub;
  else delete next.sub;
  return next;
}

function homeTab(): Tab {
  return makeTab("/");
}

/**
 * Whether a tab belongs to the cluster in focus, and so is put away when
 * another cluster takes it.
 *
 * Not an app-wide tab: Home, Settings and Connections are the same screen
 * whichever cluster is in focus, and stay where they are. And not a tab whose
 * route names a cluster of its own — the editor, "New resource", and a detail
 * tab (#834) — for two reasons. It already shows that cluster whatever the
 * rail says (see "The route is the identity" in `AGENTS.md`), under that
 * cluster's name, so it is never the mislabelled tab #908 is about; a detail
 * tab is pinned precisely so one cluster's object can stay open beside
 * another's. And putting a tab away unmounts its screen, which for an editor
 * is somebody's unsaved changes.
 */
export function followsFocus(tab: Tab): boolean {
  if (parseEditRoute(tab.route)?.cluster || parseNewRoute(tab.route)?.cluster) return false;
  if (parseDetailRoute(tab.route)?.cluster) return false;
  return isClusterScopedRoute(tab.route);
}

/**
 * Move a workspace's focus to cluster `to` (`undefined` for none): put the
 * outgoing cluster's tabs away, and bring back the ones `to` last had.
 *
 * - **Outgoing.** Its tabs, the order of the whole strip, the active tab and
 *   its own recently closed tabs are shelved under its id.
 * - **Incoming.** Its shelved tabs come back in the order they were left in,
 *   around the app-wide tabs that never went away, with the tab that was
 *   active there active again. Relabelled for `clusterName`, because a context
 *   can be renamed while its tabs are away.
 * - **Nothing remembered.** A cluster seen for the first time gets the
 *   app-wide tabs and nothing else — not a copy of another cluster's tabs.
 *   Opening its overview is the caller's to do, and only a caller acting on
 *   the reader's own gesture does ({@link withOverview}): a listing that
 *   moves the focus because a cluster vanished must not throw the reader from
 *   the screen they are on.
 * - **No cluster was in focus.** Cluster tabs open with none in focus were
 *   showing nothing; they become the incoming cluster's rather than being
 *   lost, since there is no cluster to shelve them under.
 *
 * The same workspace back when the focus does not move, so a caller can
 * decide by identity whether anything changed.
 */
export function focusStrip(w: Workspace, to: string | undefined, clusterName?: string): Workspace {
  const from = w.activeCluster;
  if (from === to) return w;

  const shelved = { ...w.shelved };
  const staying = from === undefined ? w.tabs : w.tabs.filter((t) => !followsFocus(t));
  // The recently closed list is divided the same way: an app-wide tab that
  // was closed can be brought back whichever cluster is in focus.
  const closedStaying = from === undefined ? w.closed : w.closed.filter((t) => !followsFocus(t));
  if (from !== undefined) {
    const mine = w.tabs.filter(followsFocus);
    const closed = w.closed.filter(followsFocus);
    if (mine.length > 0 || closed.length > 0) {
      shelved[from] = { tabs: mine, order: w.tabs.map((t) => t.id), activeId: w.activeId, closed };
    } else {
      delete shelved[from];
    }
  }

  const incoming = to === undefined ? undefined : shelved[to];
  if (to !== undefined) delete shelved[to];

  let tabs = staying;
  if (incoming) {
    const back = new Map(incoming.tabs.map((t) => [t.id, relabel(t, clusterName)]));
    const here = new Map(staying.map((t) => [t.id, t]));
    const placed = new Set<string>();
    tabs = [];
    for (const id of incoming.order) {
      const tab = back.get(id) ?? here.get(id);
      if (!tab || placed.has(id)) continue;
      placed.add(id);
      tabs.push(tab);
    }
    // Opened since: app-wide tabs this cluster's strip never saw, and any of
    // its own the saved order did not name.
    for (const tab of [...staying, ...back.values()]) if (!placed.has(tab.id)) tabs.push(tab);
  } else if (from === undefined && clusterName) {
    const relabelled = tabs.map((t) => (followsFocus(t) ? relabel(t, clusterName) : t));
    if (relabelled.some((t, i) => t !== tabs[i])) tabs = relabelled;
  }

  let activeId = incoming && tabs.some((t) => t.id === incoming.activeId) ? incoming.activeId : w.activeId;
  if (tabs.length === 0) tabs = [homeTab()];
  if (!tabs.some((t) => t.id === activeId)) activeId = tabs[0].id;

  const next: Workspace = {
    ...w,
    tabs,
    activeId,
    closed: incoming?.closed.length ? mostRecentlyClosed(incoming.closed, closedStaying) : closedStaying,
  };
  if (to === undefined) delete next.activeCluster;
  else next.activeCluster = to;
  if (Object.keys(shelved).length > 0) next.shelved = shelved;
  else delete next.shelved;
  return next;
}

/**
 * Two recently closed lists as one, most recent first.
 *
 * By when each tab was closed, so that the next close — which is what applies
 * the cap — drops the oldest of either list and never a tab closed a moment
 * ago because of which list it was in. A tab closed before that was recorded
 * counts as oldest and keeps its place among its own: the sort is stable, and
 * each list is already most recent first.
 *
 * **Not capped here.** Each list was capped when it was written, and bringing
 * a cluster back must not cost it a closed tab: Home offers every cluster's,
 * and reopens one by focusing its cluster and then asking for it by id. A cap
 * applied in between dropped the very tab the reader had just picked whenever
 * the shared list was the newer and full.
 */
function mostRecentlyClosed(own: Tab[], shared: Tab[]): Tab[] {
  return [...own, ...shared].sort((a, b) => (b.closedAt ?? 0) - (a.closedAt ?? 0));
}

/**
 * The workspace with its cluster's overview open and active, if the strip has
 * nothing of that cluster's — which is what a cluster looked at for the first
 * time has. The same workspace back otherwise.
 */
export function withOverview(w: Workspace, clusterName?: string): Workspace {
  if (w.activeCluster === undefined || w.tabs.some(followsFocus)) return w;
  const overview = makeTab("/overview", { clusterName });
  return { ...w, tabs: [...w.tabs, overview], activeId: overview.id };
}

/**
 * Every tab open in a workspace: the strip on screen and the strips put away
 * under its other clusters, which close with it too. A plain sum, because a
 * strip is put away holding only its own cluster's tabs (see
 * {@link focusStrip}): the app-wide ones stay on screen, so none is in two.
 */
export function workspaceTabCount(w: Workspace): number {
  return Object.values(w.shelved ?? {}).reduce((n, strip) => n + strip.tabs.length, w.tabs.length);
}

/** A workspace's shelved strips, less those of clusters it no longer has. */
function shelvedFor(w: Workspace, clusters: readonly string[]): Workspace {
  if (!w.shelved) return w;
  const kept = Object.entries(w.shelved).filter(([id]) => clusters.includes(id));
  if (kept.length === Object.keys(w.shelved).length) return w;
  const next = { ...w };
  if (kept.length > 0) next.shelved = Object.fromEntries(kept);
  else delete next.shelved;
  return next;
}

/**
 * Give a workspace a new cluster list, moving the focus if the cluster that
 * had it is gone.
 *
 * A cluster that is gone takes its tabs with it — the ones on screen if it
 * was in focus, the shelved ones otherwise. They were about that cluster and
 * no other, and leaving them up would show the next cluster's objects in
 * tabs nobody opened there, which is what #908 is about.
 */
export function withClusters(w: Workspace, clusters: string[], nameOf?: (id: string) => string | undefined): Workspace {
  const active = w.activeCluster && clusters.includes(w.activeCluster) ? w.activeCluster : clusters[0];
  let next = w;
  if (active !== w.activeCluster) {
    const gone = w.activeCluster;
    // Emptied first, so its tabs are dropped rather than shelved under an id
    // the workspace is about to forget.
    const emptied =
      gone === undefined
        ? w
        : { ...w, tabs: w.tabs.filter((t) => !followsFocus(t)), closed: w.closed.filter((t) => !followsFocus(t)) };
    next = focusStrip(emptied, active, active === undefined ? undefined : nameOf?.(active));
  }
  next = shelvedFor(next, clusters);
  if (next.tabs.length === 0) {
    const home = homeTab();
    next = { ...next, tabs: [home], activeId: home.id };
  }
  return next;
}

/**
 * What a first launch gets: one workspace holding every cluster the user has,
 * so the rail is full rather than empty, and one pinned home tab so the strip
 * is never blank.
 */
export function defaultState(contexts: ClusterContext[]): TabsState {
  const home = homeTab();
  const ids = contexts.map((c) => c.stableId);
  const w: Workspace = {
    id: newId(),
    name: "Default",
    clusters: ids,
    pausedClusters: [],
    tabs: [home],
    activeId: home.id,
    closed: [],
  };
  if (ids[0]) w.activeCluster = ids[0];
  return { workspaces: [w], currentId: w.id };
}

/**
 * Make a state consistent with the world: every cluster id names a context
 * that exists, every workspace has tabs and an active one among them, the
 * closed list is capped, and there is a current workspace. Returns the input
 * untouched when nothing needed changing, so a caller can compare by identity.
 */
export function reconcile(state: TabsState, contexts: ClusterContext[]): TabsState {
  const known = new Set(contexts.map((c) => c.stableId));
  const contextNames = new Map(contexts.map((c) => [c.stableId, c.name]));
  let changed = false;

  let workspaces = state.workspaces.map((w) => {
    let next = w;
    const clusters = w.clusters.filter((id) => known.has(id));
    if (clusters.length !== w.clusters.length) next = { ...next, clusters };
    const pausedClusters = (w.pausedClusters ?? []).filter((id) => clusters.includes(id));
    if (pausedClusters.length !== (w.pausedClusters ?? []).length) next = { ...next, pausedClusters };

    // The active cluster follows its list: it survives if it is still there,
    // and otherwise the first that remains takes over — including for a
    // workspace stored before the field existed, which has none at all. Its
    // own tabs come with it: see `withClusters`.
    next = withClusters(next, clusters, (id) => contextNames.get(id));
    const active = next.activeCluster;

    let tabs = next.tabs;
    if (tabs.length === 0) tabs = [homeTab()];
    if (active) {
      const clusterName = contextNames.get(active);
      if (clusterName) {
        // Only the tabs that are this cluster's: an editor names its own.
        const relabelled = tabs.map((tab) => (followsFocus(tab) ? relabel(tab, clusterName) : tab));
        if (relabelled.some((tab, index) => tab !== tabs[index])) tabs = relabelled;
      }
    }
    if (tabs !== next.tabs) next = { ...next, tabs };

    if (!tabs.some((t) => t.id === next.activeId)) next = { ...next, activeId: tabs[0].id };

    if (next.closed.length > CLOSED_CAP) next = { ...next, closed: next.closed.slice(0, CLOSED_CAP) };
    // The strips put away are kept true as well: capped, and labelled for
    // their cluster's name as it is now. A context renamed while its tabs are
    // away would otherwise come back under the old name whenever the focus
    // reaches it without a name in hand — a cluster removed from the sidebar,
    // say — and the window finds a tab's context by that label.
    if (next.shelved) {
      let shelved = next.shelved;
      for (const [id, strip] of Object.entries(next.shelved)) {
        const name = contextNames.get(id);
        const label = (list: Tab[]) => {
          const out = list.map((tab) => relabel(tab, name));
          return out.some((tab, index) => tab !== list[index]) ? out : list;
        };
        const stripTabs = label(strip.tabs);
        let stripClosed = label(strip.closed);
        if (stripClosed.length > CLOSED_CAP) stripClosed = stripClosed.slice(0, CLOSED_CAP);
        if (stripTabs !== strip.tabs || stripClosed !== strip.closed) {
          shelved = { ...shelved, [id]: { ...strip, tabs: stripTabs, closed: stripClosed } };
        }
      }
      if (shelved !== next.shelved) next = { ...next, shelved };
    }

    if (next !== w) changed = true;
    return next;
  });

  if (workspaces.length === 0) {
    workspaces = defaultState(contexts).workspaces;
    changed = true;
  }

  let currentId = state.currentId;
  if (!workspaces.some((w) => w.id === currentId)) {
    currentId = workspaces[0].id;
    changed = true;
  }

  return changed ? { workspaces, currentId } : state;
}
