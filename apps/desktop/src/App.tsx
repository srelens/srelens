import React, { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ClusterHotbar } from "./components/ClusterHotbar";
import { ResourceTabs, type TabDescriptor } from "./components/ResourceTabs";
import { Sidebar } from "./components/Sidebar";
import { ResourceBrowser } from "./components/ResourceBrowser";
import { K8S_KIND, RESOURCE_LABELS, type ResourceKind } from "@srelens/core";
import { CustomResourceBrowser } from "./components/CustomResourceBrowser";
import { ClusterOverview } from "./components/ClusterOverview";
import { PortForwardsView } from "./components/PortForwardsView";
import { HelmReleasesView } from "./components/HelmReleasesView";
import { NewResourceEditor } from "./components/NewResourceEditor";
import { EditResourceTab } from "./components/EditResourceTab";
import { SettingsView } from "./components/SettingsView";
import { ToolboxView } from "./components/ToolboxView";
import { AssistantTab } from "./components/AssistantTab";
import { CommandPalette } from "./components/CommandPalette";
import { ShortcutCheatSheet } from "./components/ShortcutCheatSheet";
import { isTypingTarget, matchesShortcut } from "@srelens/core";
import { McpConfirmDialog } from "./components/McpConfirmDialog";
import { VaultGate } from "./components/VaultGate";
import { Toaster } from "./components/ui/sonner";
import { Dock, type DockSession, type DockKind } from "./components/Dock";
import { StatusBar } from "./components/StatusBar";
import { LandingPage } from "./components/LandingPage";
import { getInitialTheme, applyTheme, type Theme, type ThemeMode, type ThemeName } from "./ui";
import { listCrds, type CrdRef } from "@srelens/core";
import {
  isClusterScopedKind,
  isNavigableResourceKind,
  targetNamespace,
  type ResourceTarget,
} from "@srelens/core";
import {
  loadClusterNamespaces,
  saveClusterNamespaces,
  getDefaultNamespace,
  setDefaultNamespace,
  loadWorkspaceLayout,
  saveWorkspaceLayout,
  type WorkspaceLayoutSettings,
  loadContextProfiles,
  saveContextProfiles,
  contextDisplayName,
  type ContextProfiles,
  loadKubeconfigFiles,
  saveKubeconfigFiles,
  loadContextOrder,
  saveContextOrder,
  orderContexts,
  loadUpdateChannel,
  loadMcpSettings,
} from "@srelens/core";
import { applyUiScale, getUiScale, setUiScale, stepUiScale, uiScaleShortcut } from "@srelens/core";
import { dedupeDeepLinkTargets, parseDeepLink, type DeepLinkTarget } from "@srelens/core";
import { applyViewPatch, type TabViewState } from "@srelens/core";
import {
  remapTabsToContexts,
  mergeFromNames,
  mergeOrderFromNames,
  migrateOrder,
  migrateRecordKeys,
  projectOrderToNames,
  projectToNames,
} from "@srelens/core";
import { invokeCommand } from "@srelens/core/transport";
import {
  loadOpenTabs,
  scheduleSaveOpenTabs,
  flushSaveOpenTabs,
  nextTabId,
  pruneMissingContexts,
  reconcileActiveTab,
  reconcileCrdTabs,
} from "@srelens/core";
import { flushSettingsWrites } from "@srelens/core";
import { startMcpHttp } from "@srelens/core";
import { checkForUpdateAndNotify } from "@srelens/core";
import { notify } from "@srelens/core";
import { isTauri, isWeb } from "@srelens/core/platform";
import type { SettingsSection } from "./components/SettingsView";
import { listContexts, deleteContext, type ClusterContext } from "@srelens/core";
import { deletePod } from "@srelens/core";
import { clearAccessCache } from "@srelens/core/react";
import type { ViewTab } from "@srelens/core";

/** How long a closing window waits for the settings write to land. */
const CLOSE_WRITE_TIMEOUT_MS = 2000;


export function App() {
  // Each tab is a (cluster, resource-kind) view, like browser tabs. In web mode
  // the open tabs are restored from a prior session (a browser reload otherwise
  // wipes them); desktop starts empty. Computed once so tabs/activeTabId/the id
  // counter all agree on the same restored snapshot.
  const [restored] = useState(loadOpenTabs);
  const [tabs, setTabs] = useState<ViewTab[]>(() => restored?.tabs ?? []);
  const [activeTabId, setActiveTabId] = useState<number | null>(
    () => restored?.activeTabId ?? null,
  );
  const [layout, setLayout] = useState(loadWorkspaceLayout);
  const [sidebarWidth, setSidebarWidth] = useState(layout.leftSidebarWidth);
  // Stored by stable id, rendered by display name (#265). A context's name
  // changes the moment another kubeconfig declares the same one, so keying
  // durable state on it loses everything the user configured.
  const [contextProfilesById, setContextProfilesById] = useState(loadContextProfiles);
  const [kubeconfigFiles, setKubeconfigFiles] = useState(loadKubeconfigFiles);
  const [contextOrderById, setContextOrderById] = useState(loadContextOrder);
  const [theme, setTheme] = useState<Theme>(getInitialTheme);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [cheatSheetOpen, setCheatSheetOpen] = useState(false);
  // Bumped after a palette action mutates a resource, so the active
  // ResourceBrowser remounts and re-fetches (mirrors the settingsSectionNonce
  // remount-via-key pattern below).
  const [viewReloadNonce, setViewReloadNonce] = useState(0);
  // Last-used namespace per cluster (persisted across restarts).
  const [clusterNsById, setClusterNsById] = useState<Record<string, string>>(loadClusterNamespaces);
  // Global fallback namespace for clusters with no remembered selection.
  const [defaultNs, setDefaultNs] = useState(getDefaultNamespace);
  // Start the id counter past any restored tab so ids are never reused.
  const tabIdRef = useRef(restored ? nextTabId(restored.tabs) : 1);
  const focusNonce = useRef(0);
  // Mirror the active tab id into a ref so the (once-registered) Cmd+W menu
  // event listener always sees the latest value without re-subscribing.
  const activeTabIdRef = useRef<number | null>(null);
  activeTabIdRef.current = activeTabId;
  const tabsRef = useRef<ViewTab[]>([]);
  tabsRef.current = tabs;
  // Deep-link the Settings tab to a section (e.g. from the update toast). The
  // nonce bumps to remount SettingsView at the requested section when asked.
  const [settingsInitialSection, setSettingsInitialSection] = useState<SettingsSection>("appearance");
  const [settingsSectionNonce, setSettingsSectionNonce] = useState(0);
  // The context to pre-diagnose when the Toolbox opens via a "Diagnose" deep-link.
  const [toolboxContext, setToolboxContext] = useState<string | null>(null);

  const [contexts, setContexts] = useState<ClusterContext[] | null>(null);
  const [contextsError, setContextsError] = useState("");
  // The restored-session notice fires at most once, on the first contexts
  // resolution after launch.
  const sessionPruneReported = useRef(false);

  // Durable per-context state lives under stable ids; everything below this
  // line works in display names, which is what the UI shows (#265).
  const knownContexts = contexts ?? [];
  const stableIdOf = (cluster: string) =>
    knownContexts.find((context) => context.name === cluster)?.stableId;
  const contextProfiles = projectToNames(contextProfilesById, knownContexts);
  const contextOrder = projectOrderToNames(contextOrderById, knownContexts);
  const clusterNs = projectToNames(clusterNsById, knownContexts);

  /** Remember a cluster's namespace against its identity, not its name. */
  function rememberNamespace(cluster: string, ns: string) {
    const id = stableIdOf(cluster);
    if (!id) return;
    setClusterNsById((current) => {
      const next = { ...current, [id]: ns };
      saveClusterNamespaces(next);
      return next;
    });
  }

  // One-time rekey of settings written before stable ids existed (#265).
  // Runs on the first successful context resolution, when display names still
  // match what those settings were saved under.
  const keysMigrated = useRef(false);
  useEffect(() => {
    if (keysMigrated.current || !contexts || contexts.length === 0) return;
    keysMigrated.current = true;

    const profiles = migrateRecordKeys(contextProfilesById, contexts);
    if (profiles.changed) {
      setContextProfilesById(profiles.migrated);
      saveContextProfiles(profiles.migrated);
    }
    const namespaces = migrateRecordKeys(clusterNsById, contexts);
    if (namespaces.changed) {
      setClusterNsById(namespaces.migrated);
      saveClusterNamespaces(namespaces.migrated);
    }
    const order = migrateOrder(contextOrderById, contexts);
    if (order.changed) {
      setContextOrderById(order.migrated);
      saveContextOrder(order.migrated);
    }
  }, [contexts]);

  const refreshContexts = () => {
    // Web has no local kubeconfig files to merge in — the server resolves
    // contexts server-side from the caller's uploaded kubeconfigs instead.
    listContexts(isTauri() ? kubeconfigFiles : []).then((o) => {
      setContexts(o.contexts ?? []);
      setContextsError(o.error ?? "");

      // Auto close any tabs of clusters/contexts that no longer exist!
      if (o.contexts) {
        const names = o.contexts.map((c) => c.name);
        // Remap BEFORE pruning, and prune the remapped list — a separate
        // setTabs would be overwritten by the prune below, which reads
        // tabsRef, so the remap would be dead exactly when a rename happened.
        const remapped = remapTabsToContexts(tabsRef.current, o.contexts);
        // Computed from the ref rather than inside the updater: the active id
        // has to be reconciled alongside, and state updaters must stay free
        // of side effects (React re-invokes them in development).
        const { tabs: kept, dropped } = pruneMissingContexts(remapped, names);
        const renamed = remapped.some((tab, i) => tab !== tabsRef.current[i]);
        if (renamed && dropped === 0) setTabs(remapped);
        if (dropped > 0) {
          setTabs(kept);
          // Without this the workspace goes blank behind a populated tab
          // strip, and the native close command loses its no-tabs path.
          setActiveTabId((cur) => reconcileActiveTab(kept, cur));
          // Say something only for a RESTORED session (#159): a user who just
          // removed a kubeconfig already knows why those tabs closed, and
          // this runs on every kubeconfig change.
          if (!sessionPruneReported.current) {
            notify.info(
              dropped === 1 ? "Closed 1 restored tab" : `Closed ${dropped} restored tabs`,
              "Their cluster context is no longer available.",
            );
          }
        }
        sessionPruneReported.current = true;
      }
    });
  };

  useEffect(() => {
    refreshContexts();
  }, [kubeconfigFiles]);

  // Deep links (#36). The backend stashes the URL and only nudges us, so a
  // link that launched the app cold is not lost while the WebView boots; both
  // paths DRAIN the same slot, so a link is acted on exactly once.
  const [pendingLinks, setPendingLinks] = useState<string[]>([]);
  useEffect(() => {
    if (!isTauri()) return;
    const drain = () => {
      void invokeCommand<string[]>("take_pending_deep_links")
        .then((urls) => {
          if (urls.length > 0) setPendingLinks((queued) => [...queued, ...urls]);
        })
        .catch(() => {});
    };
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // Subscribe BEFORE the first drain. Draining first leaves a window
    // between taking the queue and the listener being attached, and a link
    // landing in it would be stored, nudged into the void, and never drained
    // again until some later link happened to fire another event.
    void listen("deep-link-pending", drain)
      .then((fn) => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
        drain();
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Routed only once the contexts are known: a link that arrives during a cold
  // start would otherwise be judged against an empty context list and
  // rejected as pointing at a cluster that "doesn't exist".
  useEffect(() => {
    if (pendingLinks.length === 0 || !contexts) return;
    // Drain the whole queue: several links can arrive while the contexts are
    // still loading, and routing only the newest would silently swallow the
    // rest. They open in order, so the last one ends up in front.
    const queued = pendingLinks;
    setPendingLinks([]);

    // Validate first, then route: a batch is applied against ONE render's
    // `tabs`, so links sharing a view have to be collapsed before any of them
    // appends a tab (see dedupeDeepLinkTargets).
    const valid: DeepLinkTarget[] = [];
    for (const url of queued) {
      const target = parseDeepLink(url);
      if (!target) {
        notify.error("Couldn't open that link", "It isn't a link srelens understands.");
        continue;
      }
      if (!contexts.some((c) => c.name === target.context)) {
        notify.error("Couldn't open that link", `No kube context named "${target.context}".`);
        continue;
      }
      if (target.route === "resource") {
        // K8S_KIND alone is too permissive: Events have a list view but no
        // detail, so such a link would quietly land on the list instead of
        // the object it named.
        if (!isNavigableResourceKind(target.kind)) {
          notify.error("Couldn't open that link", `srelens can't open a ${target.kind} directly.`);
          continue;
        }
        // "-" means cluster-scoped. Allowing it for a namespaced kind would
        // search every namespace and focus whichever matching name came back
        // first — a link that silently opens the wrong object.
        if (!isClusterScopedKind(target.kind) && target.namespace === null) {
          notify.error(
            "Couldn't open that link",
            `${target.kind} is namespaced, so the link needs a namespace.`,
          );
          continue;
        }
      }
      valid.push(target);
    }

    for (const target of dedupeDeepLinkTargets(valid)) {
      if (target.route === "cluster") {
        openView(target.context, "overview");
        continue;
      }
      const entry = Object.entries(K8S_KIND).find(([, k8sKind]) => k8sKind === target.kind);
      if (!entry) continue;
      openResourceIn(
        target.context,
        entry[0] as ResourceKind,
        targetNamespace(target.kind, target.namespace),
        target.name,
      );
    }
  }, [pendingLinks, contexts]);

  // Restored CRD tabs carry a CrdRef captured in a previous session (#159).
  // The CRD may since have been deleted, or may now serve a different version,
  // and CustomResourceBrowser would surface that as a raw API error. Validate
  // once per launch, per context that actually has a restored CRD tab.
  const crdTabsValidated = useRef(false);
  useEffect(() => {
    if (crdTabsValidated.current || !contexts || !restored) return;
    const pending = [...new Set(
      tabsRef.current.filter((t) => t.crd && t.cluster).map((t) => t.cluster as string),
    )].filter((name) => contexts.some((c) => c.name === name));
    if (pending.length === 0) return;

    let cancelled = false;
    void Promise.all(
      pending.map(async (context) => ({ context, result: await listCrds(context) })),
    ).then((results) => {
      // Superseded by a newer contexts value (a kubeconfig change mid-flight):
      // drop this result and leave the one-shot flag CLEAR, so the replacement
      // effect still validates. Setting it before the await would have retired
      // the check without it ever having run.
      if (cancelled) return;
      crdTabsValidated.current = true;
      let total = 0;
      let next = tabsRef.current;
      for (const { context, result } of results) {
        // Only a SUCCESSFUL discovery is evidence of absence. An unreachable
        // cluster must never silently delete the user's restored tabs.
        if (!result.crds) continue;
        const reconciled = reconcileCrdTabs(next, context, result.crds);
        next = reconciled.tabs;
        total += reconciled.dropped;
      }
      if (next !== tabsRef.current) {
        setTabs(next);
        setActiveTabId((cur) => reconcileActiveTab(next, cur));
      }
      if (total > 0) {
        notify.info(
          total === 1 ? "Closed 1 restored tab" : `Closed ${total} restored tabs`,
          "Their custom resource is no longer installed.",
        );
      }
    });
    return () => {
      cancelled = true;
    };
  }, [contexts, restored]);

  // Listen to external/internal kubeconfig changes
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const unlistenPromise = listen("kubeconfig-changed", () => {
      refreshContexts();
    }).catch(() => () => {});
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // The master-password gate (issue #208) mounts as a blocking overlay via
  // <VaultGate /> below — setup at first launch, unlock (password or the
  // enrolled biometric skip) on later ones.

  const handleDeleteContext = async (name: string) => {
    try {
      await deleteContext(name);

      // Clean up profiles
      const nextProfiles = { ...contextProfiles };
      delete nextProfiles[name];
      changeContextProfiles(nextProfiles);

      // Clean up order
      const nextOrder = contextOrder.filter((item) => item !== name);
      changeContextOrder(nextOrder);

      // Clean up namespace preference
      const removedId = stableIdOf(name);
      if (removedId) {
        setClusterNsById((current) => {
          const next = { ...current };
          delete next[removedId];
          saveClusterNamespaces(next);
          return next;
        });
      }

      // Close open tabs for this cluster
      setTabs((ts) => ts.filter((t) => t.cluster !== name));

      // Refresh list
      refreshContexts();
    } catch (e) {
      notify.error("Failed to remove context", String(e));
    }
  };

  // Persist per-cluster namespace whenever it changes.
  // No effect persisting `clusterNs`: it is a DERIVED, name-keyed projection.
  // Saving it wrote `{}` on the first paint (before contexts resolve) and
  // name-keyed data afterwards, undoing the id migration on disk. The id-keyed
  // setters — rememberNamespace, the migration, delete-context — save instead.

  // Persist the open tabs (web only) so a browser reload restores them.
  useEffect(() => scheduleSaveOpenTabs(tabs, activeTabId), [tabs, activeTabId]);

  // Web: localStorage writes are synchronous, so unload handlers suffice.
  useEffect(() => {
    if (isTauri()) return;
    const flush = () => flushSaveOpenTabs();
    window.addEventListener("beforeunload", flush);
    window.addEventListener("pagehide", flush);
    return () => {
      window.removeEventListener("beforeunload", flush);
      window.removeEventListener("pagehide", flush);
    };
    // Deliberately no flush on unmount: the real teardown is the window going
    // away, which fires the events above. Flushing here would also write on
    // every test/HMR unmount, persisting a session the next mount restores.
  }, []);

  // Desktop: the durable write is an async IPC round trip, which an unload
  // handler cannot wait for — it returns immediately and the WebView is torn
  // down mid-write. Intercept the close instead, drain the queue, then close.
  useEffect(() => {
    if (!isTauri()) return;
    const win = getCurrentWindow();
    // Older runtimes (and the test harness) may not expose this; losing the
    // close-time flush is far better than failing to mount the app.
    if (typeof win?.onCloseRequested !== "function") return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void win
      .onCloseRequested(async (event) => {
        event.preventDefault();
        flushSaveOpenTabs();
        // Bounded: a stuck or slow write must never leave the user unable to
        // quit, so the close proceeds either way.
        await Promise.race([
          flushSettingsWrites(),
          new Promise((resolve) => setTimeout(resolve, CLOSE_WRITE_TIMEOUT_MS)),
        ]);
        // destroy(), not close() — close() re-emits this event and would loop.
        await win.destroy();
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  /** The namespace a new tab in `cluster` should start on. */
  const namespaceFor = (cluster: string) => clusterNs[cluster] ?? defaultNs;

  /** Update a tab's namespace filter and remember it as the cluster's default. */
  function setTabNamespace(tabId: number, cluster: string, ns: string) {
    setTabs((ts) => ts.map((t) => (t.id === tabId ? { ...t, namespace: ns } : t)));
    rememberNamespace(cluster, ns);
  }

  /** Change the saved default namespace (settings). */
  function changeDefaultNamespace(ns: string) {
    setDefaultNs(ns);
    setDefaultNamespace(ns);
  }

  function changeWorkspaceLayout(next: WorkspaceLayoutSettings) {
    setLayout(next);
    setSidebarWidth(next.leftSidebarWidth);
    saveWorkspaceLayout(next);
  }

  function changeContextProfiles(next: ContextProfiles) {
    // Merged, not replaced: the name-keyed view only holds connected
    // contexts, so replacing would delete every offline cluster's identity.
    const merged = mergeFromNames(contextProfilesById, next, contexts ?? []);
    setContextProfilesById(merged);
    saveContextProfiles(merged);
  }

  function changeKubeconfigFiles(next: string[]) {
    setKubeconfigFiles(next);
    saveKubeconfigFiles(next);
  }

  function changeContextOrder(next: string[]) {
    const merged = mergeOrderFromNames(contextOrderById, next, contexts ?? []);
    setContextOrderById(merged);
    saveContextOrder(merged);
  }

  useEffect(() => {
    applyTheme(theme);
    if (theme.mode !== "system") return;

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const syncSystemTheme = () => applyTheme(theme);
    mediaQuery.addEventListener("change", syncSystemTheme);
    return () => mediaQuery.removeEventListener("change", syncSystemTheme);
  }, [theme]);

  function setThemeMode(mode: ThemeMode) {
    setTheme((current) => ({ ...current, mode }));
  }

  function setThemeName(name: ThemeName) {
    setTheme((current) => ({ ...current, name }));
  }

  function toggleThemeMode() {
    setTheme((current) => ({
      ...current,
      mode: current.mode === "dark" ? "light" : "dark",
    }));
  }

  // Global shortcuts, matched against the registry the cheat sheet reads from
  // (#160), so the two cannot drift apart.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (matchesShortcut("palette", e)) {
        e.preventDefault();
        setPaletteOpen((o) => !o);
        return;
      }
      // `?` carries no modifier, so it has to yield to whatever the user is
      // typing into — in a search box it is a question mark, not a request
      // for help.
      if (matchesShortcut("cheatsheet", e) && !isTypingTarget(e.target)) {
        e.preventDefault();
        setCheatSheetOpen((o) => !o);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Interface scale (#237): restore the persisted zoom and serve the
  // browser-zoom shortcuts (Cmd/Ctrl +, -, 0). Desktop only — a browser
  // already zooms on these keys, and preventDefault here would suppress it.
  useEffect(() => {
    if (!isTauri()) return;
    applyUiScale(getUiScale());
    function onKey(e: KeyboardEvent) {
      const action = uiScaleShortcut(e);
      if (!action) return;
      e.preventDefault();
      applyUiScale(setUiScale(stepUiScale(getUiScale(), action)));
      // Keep an open Settings slider in step with the shortcut.
      window.dispatchEvent(new Event("srelens:uiscale"));
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // On macOS the native menu routes Cmd+W to a custom "Close" item (see
  // src-tauri) which emits `close-active-tab`. Close the active tab here, and
  // only fall back to closing the window when no tabs remain — mirroring
  // browser-style tab behavior.
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const unlistenPromise = listen("close-active-tab", () => {
      const id = activeTabIdRef.current;
      if (id != null) {
        const closingLastTab = tabsRef.current.length === 1 && tabsRef.current[0]?.id === id;
        const remaining = tabsRef.current.filter((t) => t.id !== id);
        closeView(id);
        // `closeView` only SCHEDULES the state change; the close below fires
        // in this same callback, before React has re-rendered and run the
        // persistence effect. Without queueing the post-close snapshot here,
        // the close-request flush would write the pre-close one and the tab
        // the user just closed would come back on the next launch.
        scheduleSaveOpenTabs(remaining, remaining.at(-1)?.id ?? null);
        if (closingLastTab) void getCurrentWindow().close();
      } else {
        void getCurrentWindow().close();
      }
    }).catch(() => () => {});
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;
  const activeCluster = activeTab?.cluster ?? null;
  // Sort, search text and filtered column belong to the ACTIVE tab (#254).
  // Previously sort/filter lived in the unmounted-on-switch view components
  // and the search box was one App-level value shared by every tab, so the
  // first vanished on a switch and the second leaked across tabs.
  const activeView = activeTab?.view;
  const query = activeView?.query ?? "";
  const updateActiveView = (patch: Partial<TabViewState>) => {
    if (activeTabId == null) return;
    setTabs((ts) =>
      ts.map((t) => (t.id === activeTabId ? { ...t, view: applyViewPatch(t.view, patch) } : t)),
    );
  };
  const activeKind: ResourceKind = activeTab?.kind ?? "pods";
  const activeCrd = activeTab?.crd ?? null;
  const clusters = orderContexts(
    [...new Set(tabs.flatMap((t) => (t.cluster ? [t.cluster] : [])))].map((name) => ({ name })),
    contextOrder,
  ).map(({ name }) => name);

  // RBAC preflight results are cached per (context, check) — clear them on
  // context switch so a stale cache from a previous cluster (or an admin who
  // just changed the user's bindings) can't leave controls mis-gated.
  useEffect(() => {
    clearAccessCache();
  }, [activeCluster]);

  /** Open (or focus, if already open) a resource view for a cluster + kind. */
  function openView(cluster: string, kind: ResourceKind) {
    if (kind === "settings") {
      openSettings();
      return;
    }
    if (kind === "toolbox") {
      openToolbox();
      return;
    }
    if (kind === "assistant") {
      openAssistant();
      return;
    }
    const existing = tabs.find((t) => t.cluster === cluster && t.kind === kind && !t.crd);
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = tabIdRef.current++;
    setTabs((ts) => [...ts, { id, cluster, clusterId: stableIdOf(cluster), kind, namespace: namespaceFor(cluster) }]);
    setActiveTabId(id);
  }

  /** Open the single workspace-level Settings tab, optionally at a section. */
  function openSettings(section?: SettingsSection) {
    if (section) {
      setSettingsInitialSection(section);
      // Remount SettingsView so it opens at the requested section even if the
      // tab is already open on another section.
      setSettingsSectionNonce((n) => n + 1);
    }
    const existing = tabs.find((t) => t.kind === "settings" && !t.cluster);
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = tabIdRef.current++;
    setTabs((ts) => [...ts, { id, cluster: null, kind: "settings" }]);
    setActiveTabId(id);
  }

  /** Open (or focus) the single workspace-level Toolbox tab. When `context` is
   *  given (from a "Diagnose in Toolbox" deep-link), that context is
   *  pre-diagnosed in the health section. */
  function openToolbox(context?: string) {
    setToolboxContext(context ?? null);
    const existing = tabs.find((t) => t.kind === "toolbox" && !t.cluster);
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = tabIdRef.current++;
    setTabs((ts) => [...ts, { id, cluster: null, kind: "toolbox" }]);
    setActiveTabId(id);
  }

  /** Open (or focus) the single workspace-level Assistant tab: a full-tab,
   *  global chat with the configured coding agent, not scoped to any
   *  particular resource. */
  function openAssistant() {
    // The assistant drives agent CLIs through Tauri-only backend commands
    // (agent_list / chat_start / chat_send). In a web build those fall through
    // the server's command match to a 404, so the tab would mount with an empty
    // agent list and Send permanently disabled — don't open it there. This also
    // backstops the command-palette and search paths that funnel here.
    if (!isTauri()) return;
    const existing = tabs.find((t) => t.kind === "assistant" && !t.cluster);
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = tabIdRef.current++;
    setTabs((ts) => [...ts, { id, cluster: null, kind: "assistant", namespace: "" }]);
    setActiveTabId(id);
  }
  // Keep a stable handle to openSettings so the update-check effect's toast
  // action always uses the current tab state, not a stale closure.
  const openSettingsRef = useRef(openSettings);
  openSettingsRef.current = openSettings;

  // Automatically check for updates on startup and periodically, surfacing a
  // small toast (with a link to the Updates section) when one is available —
  // rather than only when the user opens Settings and clicks "check".
  const notifiedVersionRef = useRef<string | null>(null);
  useEffect(() => {
    if (!isTauri()) return;
    const channel = loadUpdateChannel();
    const run = () =>
      void checkForUpdateAndNotify(
        channel,
        (update) => {
          notifiedVersionRef.current = update.version;
          notify.updateAvailable(update.version, () => openSettingsRef.current("updates"));
        },
        { alreadyNotified: (v) => notifiedVersionRef.current === v },
      );
    run();
    const SIX_HOURS = 6 * 60 * 60 * 1000;
    const timer = setInterval(run, SIX_HOURS);
    return () => clearInterval(timer);
  }, []);

  // The MCP server needs the vault's token, so its auto-start must wait for
  // the VaultGate to report the vault usable (set up + unlocked). Flipped by
  // the gate's onReady exactly once per launch; starting while locked would
  // fail to persist a token and silently never retry.
  const [vaultReady, setVaultReady] = useState(false);

  // Start the in-app MCP HTTP server once the vault is ready if the user left
  // it enabled, so agents can connect without opening Settings first.
  useEffect(() => {
    if (!isTauri() || !vaultReady) return;
    const mcp = loadMcpSettings();
    if (mcp.enabled) void startMcpHttp(mcp.port).catch(() => {});
  }, [vaultReady]);

  /** Open a resource's kind view in a NAMED cluster and focus its detail.
   *  Takes the cluster explicitly because a deep link can target a context
   *  other than the one currently in front (#36). */
  function openResourceIn(
    cluster: string,
    kind: ResourceKind,
    namespace: string | null,
    name: string,
  ) {
    const focus = { name, namespace, nonce: ++focusNonce.current };
    // Filter the list to the resource's namespace so its row is present to focus.
    const ns = namespace ?? "";
    const existing = tabs.find((t) => t.cluster === cluster && t.kind === kind && !t.crd);
    if (existing) {
      setTabs((ts) =>
        ts.map((t) =>
          t.id === existing.id
            ? {
                ...t,
                focus,
                namespace: ns,
                // Clear THIS tab's search only. The detail opens from the
                // unfiltered rows, so a leftover query would leave the user
                // on a list that doesn't contain what they navigated to once
                // the drawer closes. Sort and filter column are unaffected,
                // and other tabs keep their own searches.
                view: applyViewPatch(t.view, { query: "" }),
              }
            : t,
        ),
      );
      setActiveTabId(existing.id);
    } else {
      const id = tabIdRef.current++;
      setTabs((ts) => [...ts, { id, cluster, clusterId: stableIdOf(cluster), kind, focus, namespace: ns }]);
      setActiveTabId(id);
    }
    rememberNamespace(cluster, ns);
  }

  /** Open a resource's kind view and deep-link to its detail (from search). */
  function openResource(kind: ResourceKind, namespace: string | null, name: string) {
    if (!activeCluster) return;
    openResourceIn(activeCluster, kind, namespace, name);
  }

  /** Resolve a canonical Kubernetes kind from a detail link to its product view. */
  function openLinkedResource(target: ResourceTarget) {
    const entry = Object.entries(K8S_KIND).find(([, k8sKind]) => k8sKind === target.kind);
    if (!entry) return;
    openResource(entry[0] as ResourceKind, target.namespace, target.name);
  }

  /** Open a fresh "new resource" editor tab, optionally seeded with a template. */
  function openNewResource(initialKind?: string) {
    if (!activeCluster) return;
    const id = tabIdRef.current++;
    setTabs((ts) => [...ts, { id, cluster: activeCluster, kind: "newresource", create: { initialKind } }]);
    setActiveTabId(id);
  }

  /** Open (or focus) a full-tab editor preloaded with a resource's manifest. */
  function openEditResource(kind: string, namespace: string | null, name: string) {
    if (!activeCluster) return;
    const existing = tabs.find(
      (t) =>
        t.kind === "editresource" &&
        t.cluster === activeCluster &&
        t.edit?.kind === kind &&
        (t.edit?.namespace ?? null) === (namespace ?? null) &&
        t.edit?.name === name,
    );
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = tabIdRef.current++;
    setTabs((ts) => [...ts, { id, cluster: activeCluster, kind: "editresource", edit: { kind, namespace, name } }]);
    setActiveTabId(id);
  }

  /** Open (or focus) a custom-resource view for a cluster + CRD. */
  function openCrdView(cluster: string, crd: CrdRef) {
    const existing = tabs.find((t) => t.cluster === cluster && t.crd?.name === crd.name);
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = tabIdRef.current++;
    setTabs((ts) => [...ts, { id, cluster, clusterId: stableIdOf(cluster), kind: "overview", crd }]);
    setActiveTabId(id);
  }
  function closeView(id: number) {
    setTabs((ts) => {
      const remaining = ts.filter((t) => t.id !== id);
      setActiveTabId((a) => (a === id ? (remaining.at(-1)?.id ?? null) : a));
      return remaining;
    });
  }
  /** Close every tab except `id`, then focus it. */
  function closeOtherViews(id: number) {
    setTabs((ts) => ts.filter((t) => t.id === id));
    setActiveTabId(id);
  }
  /** Close every tab to the right of `id`. */
  function closeViewsToRight(id: number) {
    setTabs((ts) => {
      const idx = ts.findIndex((t) => t.id === id);
      if (idx < 0) return ts;
      const remaining = ts.slice(0, idx + 1);
      setActiveTabId((a) => (remaining.some((t) => t.id === a) ? a : id));
      return remaining;
    });
  }
  function closeAllViews() {
    setTabs([]);
    setActiveTabId(null);
  }

  // Bottom dock state (terminals + logs as tabs).
  const [dockSessions, setDockSessions] = useState<DockSession[]>([]);
  const [activeDock, setActiveDock] = useState<number | null>(null);
  const [dockHeight, setDockHeight] = useState(300);
  const dockIdRef = useRef(1);

  function openDock(
    kind: DockKind,
    s: {
      context: string;
      namespace: string;
      pod?: string;
      container?: string;
      workload?: { kind: string; name: string };
      kubeconfigFiles?: string[];
      helm?: { args: string[]; title: string; values?: string; onComplete?: () => void };
      /** A pod to delete when this dock session closes (node debug shell). */
      deleteOnClose?: { context: string; namespace: string; pod: string };
      /** Override the terminal's exec command (node shell `nsenter …`). */
      execCommand?: string[];
    },
  ) {
    const id = dockIdRef.current++;
    setDockSessions((t) => [...t, { id, kind, ...s }]);
    setActiveDock(id);
  }
  /** Tear down any pod tied to a closing dock session (e.g. node debug shell). */
  function teardownDock(sessions: DockSession[]) {
    for (const s of sessions) {
      if (s.deleteOnClose) {
        void deletePod(s.deleteOnClose.context, s.deleteOnClose.namespace, s.deleteOnClose.pod);
      }
    }
  }
  function closeDockTab(id: number) {
    setDockSessions((t) => {
      teardownDock(t.filter((x) => x.id === id));
      const remaining = t.filter((x) => x.id !== id);
      setActiveDock((a) => (a === id ? (remaining.at(-1)?.id ?? null) : a));
      return remaining;
    });
  }
  function closeDock() {
    setDockSessions((t) => {
      teardownDock(t);
      return [];
    });
    setActiveDock(null);
  }

  // Contexts the status bar's terminal launcher can open a shell for, in the
  // hotbar's order so the two lists agree.
  const terminalContexts = useMemo(
    () =>
      orderContexts(contexts ?? [], contextOrder).map((c) => ({
        name: c.name,
        label: contextDisplayName(c.name, contextProfiles[c.name]),
      })),
    [contexts, contextOrder, contextProfiles],
  );

  const tabDescriptors: TabDescriptor[] = tabs.map((t) => ({
    id: t.id,
    label: t.edit
      ? `edit: ${t.edit.kind}/${t.edit.name}`
      : t.cluster
        ? `${t.crd ? t.crd.kind : RESOURCE_LABELS[t.kind]} · ${contextDisplayName(t.cluster, contextProfiles[t.cluster])}`
        : RESOURCE_LABELS[t.kind],
  }));

  return (
    <div
      className={`fl-app${activeCluster ? "" : " fl-app--no-cluster"}`}
      style={activeCluster ? { gridTemplateColumns: `75px ${sidebarWidth}px 1fr` } : undefined}
    >
      <ClusterHotbar
        openContext={activeCluster}
        onOpenContext={(ctx) => openView(ctx, "overview")}
        theme={theme}
        onToggleTheme={toggleThemeMode}
        onOpenSettings={openSettings}
        onOpenToolbox={openToolbox}
        onOpenAssistant={isTauri() ? openAssistant : undefined}
        contextProfiles={contextProfiles}
        kubeconfigFiles={kubeconfigFiles}
        contextOrder={contextOrder}
        contexts={contexts ?? []}
      />
      {activeCluster && (
        <Sidebar
          clusters={clusters}
          activeCluster={activeCluster}
          activeKind={activeKind}
          activeCrd={activeCrd}
          onSelect={(c, k) => openView(c, k)}
          onSelectCrd={(c, crd) => openCrdView(c, crd)}
          contextProfiles={contextProfiles}
          width={sidebarWidth}
          onResize={setSidebarWidth}
        />
      )}
      <div className="fl-main">
        {tabs.length > 0 ? (
          <>
            <ResourceTabs
              tabs={tabDescriptors}
              activeId={activeTabId}
              onActivate={setActiveTabId}
              onClose={closeView}
              onCloseOthers={closeOtherViews}
              onCloseToRight={closeViewsToRight}
              onCloseAll={closeAllViews}
            />
            {activeTab && (
              <>
                <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background">
                  {activeKind === "settings" ? (
                    <SettingsView
                      key={`${activeTab.id}:${settingsSectionNonce}`}
                      initialSection={settingsInitialSection}
                      theme={theme}
                      onThemeNameChange={setThemeName}
                      onThemeModeChange={setThemeMode}
                      defaultNamespace={defaultNs}
                      onDefaultNamespaceChange={changeDefaultNamespace}
                      layout={layout}
                      onLayoutChange={changeWorkspaceLayout}
                      contextProfiles={contextProfiles}
                      onContextProfilesChange={changeContextProfiles}
                      kubeconfigFiles={kubeconfigFiles}
                      onKubeconfigFilesChange={changeKubeconfigFiles}
                      contextOrder={contextOrder}
                      onContextOrderChange={changeContextOrder}
                      contexts={contexts}
                      contextsError={contextsError}
                      onDeleteContext={handleDeleteContext}
                    />
                  ) : activeKind === "toolbox" ? (
                    <ToolboxView key={activeTab.id} initialContext={toolboxContext} />
                  ) : activeKind === "assistant" ? (
                    <div className="min-h-0 flex-1 overflow-hidden">
                      <AssistantTab
                        cluster={activeCluster}
                        namespace={activeTab.namespace}
                        availableContexts={contexts?.map((c) => c.name) ?? []}
                      />
                    </div>
                  ) : activeTab.crd && activeCluster ? (
                    <CustomResourceBrowser
                      key={activeTab.id}
                      context={activeCluster}
                      crd={activeTab.crd}
                      query={query}
                      onQueryChange={(q) => updateActiveView({ query: q })}
                      view={activeView}
                      onViewChange={updateActiveView}
                      detailDrawerWidth={layout.rightSidebarWidth}
                    />
                  ) : activeCluster && activeKind === "overview" ? (
                    <div className="min-h-0 flex-1 overflow-auto p-3">
                      <ClusterOverview
                        key={activeTab.id}
                        context={activeCluster}
                        onOpenView={(kind) => openView(activeCluster, kind)}
                        onDiagnose={() => openToolbox(activeCluster)}
                      />
                    </div>
                  ) : activeCluster && activeKind === "portforwards" ? (
                    <PortForwardsView key={activeTab.id} context={activeCluster} />
                  ) : activeCluster && activeKind === "helmreleases" ? (
                    <HelmReleasesView
                      key={activeTab.id}
                      context={activeCluster}
                      detailDrawerWidth={layout.rightSidebarWidth}
                      openHelmDock={(s) =>
                        openDock("helm", { context: s.context, namespace: s.namespace, helm: s.helm, kubeconfigFiles })
                      }
                      initialNamespace={activeTab.namespace ?? ""}
                      onNamespaceChange={(ns) => setTabNamespace(activeTab.id, activeCluster, ns)}
                      kubeconfigFiles={kubeconfigFiles}
                      view={activeView}
                      onViewChange={updateActiveView}
                    />
                  ) : activeCluster && activeKind === "newresource" ? (
                    <NewResourceEditor
                      key={activeTab.id}
                      context={activeCluster}
                      initialKind={activeTab.create?.initialKind}
                    />
                  ) : activeCluster && activeKind === "editresource" && activeTab.edit ? (
                    <EditResourceTab
                      key={activeTab.id}
                      context={activeCluster}
                      kind={activeTab.edit.kind}
                      namespace={activeTab.edit.namespace}
                      name={activeTab.edit.name}
                    />
                  ) : activeCluster ? (
                    <ResourceBrowser
                      key={`${activeTab.id}:${viewReloadNonce}`}
                      context={activeCluster}
                      kind={activeKind}
                      query={query}
                      onQueryChange={(q) => updateActiveView({ query: q })}
                      view={activeView}
                      onViewChange={updateActiveView}
                      onOpenTerminal={(s) => openDock("terminal", s)}
                      onOpenLogs={(s) => openDock("logs", s)}
                      onOpenEdit={openEditResource}
                      onOpenWorkloadLogs={(s) =>
                        openDock("logs", {
                          context: s.context,
                          namespace: s.namespace,
                          workload: { kind: s.kind, name: s.name },
                        })
                      }
                      onOpenNew={openNewResource}
                      onOpenResource={openLinkedResource}
                      focus={activeTab.focus}
                      initialNamespace={activeTab.namespace ?? ""}
                      onNamespaceChange={(ns) => setTabNamespace(activeTab.id, activeCluster, ns)}
                      detailDrawerWidth={layout.rightSidebarWidth}
                      kubeconfigFiles={kubeconfigFiles}
                    />
                  ) : (
                    <LandingPage
                      onOpenContext={(ctx) => openView(ctx, "overview")}
                      onOpenSettings={openSettings}
                      contextProfiles={contextProfiles}
                      kubeconfigFiles={kubeconfigFiles}
                      contextOrder={contextOrder}
                      contexts={contexts}
                      contextsError={contextsError}
                    />
                  )}
                </div>
              </>
            )}
          </>
        ) : (
          // Wrapped so the landing page shrinks when the dock is open: it asks
          // for `min-height: 100%`, which unwrapped is 100% of the whole main
          // column and would push the dock off the bottom.
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
            <LandingPage
              onOpenContext={(ctx) => openView(ctx, "overview")}
              onOpenSettings={openSettings}
              contextProfiles={contextProfiles}
              kubeconfigFiles={kubeconfigFiles}
              contextOrder={contextOrder}
              contexts={contexts}
              contextsError={contextsError}
            />
          </div>
        )}
        {/* Outside the tab branch: the status bar can start a shell with no
            tabs open at all, and a dock that only mounts alongside a tab would
            swallow that session silently. */}
        {dockSessions.length > 0 && (
          <Dock
            sessions={dockSessions}
            activeId={activeDock}
            height={dockHeight}
            onActivate={setActiveDock}
            onCloseTab={closeDockTab}
            onClose={closeDock}
            onResize={setDockHeight}
            onNewTerminal={(() => {
              // "+" opens another host shell for the active shell's context.
              // The host shell is desktop-only — web users get in-pod exec
              // terminals, so no "+" there.
              if (isWeb) return undefined;
              const active = dockSessions.find((s) => s.id === activeDock);
              const ctx = active?.kind === "shell" ? active.context : activeCluster;
              const files =
                active?.kind === "shell" ? active.kubeconfigFiles ?? kubeconfigFiles : kubeconfigFiles;
              return ctx
                ? () => openDock("shell", { context: ctx, namespace: "", kubeconfigFiles: files })
                : undefined;
            })()}
          />
        )}
      </div>
      <StatusBar
        activeCluster={activeCluster}
        activeLabel={
          activeTab ? (activeTab.crd ? activeTab.crd.kind : RESOURCE_LABELS[activeKind]) : undefined
        }
        tabCount={tabs.length}
        // Every configured context, not just the active tab's: a shell for a
        // second cluster used to need a tab opened for it first, and the
        // launcher disappeared entirely on tabs with no cluster (#257).
        terminalContexts={terminalContexts}
        onOpenTerminal={
          // The host shell (`kubectl · <ctx>`) is desktop-only: on the shared
          // web server a container-host shell would break user isolation. Web
          // users reach an RBAC-scoped in-pod exec terminal from a pod instead.
          isWeb
            ? undefined
            : (context) => openDock("shell", { context, namespace: "", kubeconfigFiles })
        }
      />
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        context={activeCluster}
        onOpenView={(kind) =>
          kind === "settings"
            ? openSettings()
            : kind === "toolbox"
              ? openToolbox()
              : kind === "assistant"
                ? openAssistant()
                : activeCluster && openView(activeCluster, kind)
        }
        onOpenResource={openResource}
        onOpenCrd={(crd) => activeCluster && openCrdView(activeCluster, crd)}
        currentViewKind={activeTab?.kind}
        onAfterAction={() => setViewReloadNonce((n) => n + 1)}
      />
      <ShortcutCheatSheet open={cheatSheetOpen} onOpenChange={setCheatSheetOpen} desktop={!isWeb} />
      <Toaster position="top-right" richColors closeButton />
      <McpConfirmDialog />
      <VaultGate onReady={() => setVaultReady(true)} />
    </div>
  );
}
