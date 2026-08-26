import { useEffect, useMemo, useRef, useState } from "react";
import {
  isApplePlatform,
  isTauri,
  listContexts,
  loadKubeconfigFiles,
  rehydrateForwards,
  type ClusterContext,
} from "@srelens/core";
import { Button, Checkbox, Drawer, LoadingState, TabStrip, TextInput, type ContextMenuItem, type StripTab } from "@srelens/ui-kit";
import { setContexts, setKubeconfigFiles, useContexts } from "../lib/clusters";
import { loadColumnPrefs } from "../lib/columnPrefs";
import { loadRecentLogSubjects } from "../lib/logRecents";
import { loadMarks } from "../lib/marks";
import { loadPeekWidth } from "../lib/peekWidth";
import { loadSectionFolds } from "../lib/sectionFolds";
import { loadNamespaces } from "../lib/workspace";
import { defaultState, reconcile } from "../lib/tabs";
import { flushSave, installFlushOnUnload, loadTabsState, scheduleSave } from "../lib/tabsPersist";
import {
  activateTab,
  closeAll,
  closeOthers,
  closeTab,
  closeToRight,
  createWorkspace,
  currentWorkspace,
  cycleTab,
  duplicateTab,
  getState,
  newTab,
  openTab,
  reopenClosed,
  selectIndex,
  setState,
  subscribe,
  switchWorkspace,
  togglePin,
  useActiveCluster,
  useTabs,
} from "../lib/tabsStore";
import { useConsole } from "../console";
import { getInfo, probeCluster } from "../lib/probe";
import { hint, matchWindowKey, type WindowAction } from "../lib/shortcuts";
import { Body } from "./Body";
import { Chrome, zoom } from "./Chrome";
import { Console } from "./Console";
import { Nav } from "./Nav";
import { Rail } from "./Rail";
import { Status } from "./Status";
import { TabSurface } from "./TabSurface";

export interface WindowProps {
  /** Display names of the screens that exist in the new design. */
  ported: string[];
  onOpenInClassic: (route: string, context?: string) => void;
  onOpenGallery?: () => void;
  onToggleTheme?: () => void;
  controls?: "macos" | "none";
  /**
   * False while the gallery is up: the chrome comes down and the accelerators
   * stop listening, but the tab bodies stay mounted so the session survives.
   */
  active?: boolean;
}

/**
 * The new design's window: the chrome composed around the tab strip over the
 * tab bodies.
 *
 * Boot reads the saved tabs and the cluster list together, then either
 * reconciles the one against the other or builds a Default workspace from the
 * clusters. Nothing renders until that resolves: a flash of last session's
 * tabs being replaced by this session's would read as the app losing work.
 *
 * The accelerator table is bound here, once, rather than in every component
 * that wants one — a chord bound twice is a chord that fires twice. It listens
 * only while `active`, which is what keeps ⌘T out of the gallery's own inputs.
 */
export function Window({
  ported,
  onOpenInClassic,
  onOpenGallery,
  onToggleTheme = () => {},
  controls = "none",
  active = true,
}: WindowProps) {
  const [booted, setBooted] = useState(false);
  // Kept from boot because half the chrome wants it: the rail resolves ids to
  // names, the sidebar looks up CRDs by name, and the new-tab action turns the
  // active cluster's id into the name a tab carries. Held in the shared store
  // rather than local state, so a screen that receives only `{ route }` can
  // resolve a workspace's cluster ids the same way `Window` does.
  const contexts = useContexts();
  // Kept rather than dropped once read: a failed list still preserves the
  // saved cluster ids (see below), and the rail has to say why it cannot draw
  // them rather than leaving the user to guess.
  const [contextsError, setContextsError] = useState<string | undefined>(undefined);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const apple = useMemo(() => isApplePlatform(), []);
  // Desktop only: `zoom` asks the webview to scale itself, which only exists
  // under Tauri. In a browser the native zoom already does this (see core's
  // uiScale doc), so a zoom chord here has to fall through to it untouched.
  const desktop = useMemo(() => isTauri(), []);
  const { setOpen } = useConsole();
  const { tabs, activeId, workspace } = useTabs();
  const activeIdCluster = useActiveCluster();
  const activeCtx = contexts.find((c) => c.stableId === activeIdCluster) ?? null;

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      // Whatever happens in here, boot has to finish: an exception escaping
      // this IIFE left `booted` false forever, so the window was a spinner —
      // no tabs, no Placeholder, and no way back to classic. A user whose
      // storage refuses reads gets a fresh workspace, not a dead window.
      // The contexts are read first so that `found` is already filled if the
      // saved state is what fails: the fallback is then a Default workspace
      // over the user's real clusters rather than an empty rail.
      // Loaded before anything else in boot: it try/catches internally and
      // cannot throw, so there is no reason to make the tabs/contexts work
      // wait on it. Without this call the module never reads what is on disk
      // — every mark starts at the default, and the first `setMark` then
      // spreads over that empty record and persists it, erasing every other
      // cluster's stored appearance.
      loadMarks();
      // Same reason as `loadMarks` above: without this, the first column
      // toggle spreads over an empty record and erases every other kind's.
      loadColumnPrefs();
      // And the detail peek's width, for the same reason both of those are
      // here: unread, the pane opens at its default however wide the reader
      // last dragged it.
      loadPeekWidth();
      // And which blocks of a resource detail the reader has opened. Unread,
      // every detail opens fully collapsed however many blocks they last
      // unfolded — and the first unfold then spreads over an empty record and
      // erases every other kind's, exactly as `loadMarks` above describes.
      loadSectionFolds();
      // And the namespace selection each cluster was narrowed to — unlike
      // `links`/`expanded` on the same store, this one is persisted, and
      // unread it costs the reader their picker choice on every launch.
      loadNamespaces();
      // And the subjects a bare `/logs` offers as a way in. Unread, that
      // screen has nothing to offer on the first visit of every launch — and
      // the first subject followed then spreads over an empty list and erases
      // every earlier one, exactly as `loadMarks` above describes.
      loadRecentLogSubjects();
      // And what the backend is still forwarding. The forwards store is
      // module-level JavaScript: a browser reload empties it while the server
      // keeps the tunnels up, so without this the status bar reads
      // `0 port-forwards` on every route but `/forwards` — which is the one
      // route that rehydrates itself, and the one a reader is least likely to
      // reload onto. It never rejects, so it needs no guard of its own.
      void rehydrateForwards();
      // Classic threads these through every call for a reason: a context that
      // came from an additional kubeconfig file cannot have a client built for
      // it until the backend has been told the file's path, and the list races
      // the first use otherwise.
      const files = isTauri() ? loadKubeconfigFiles() : [];
      setKubeconfigFiles(files);
      let found: ClusterContext[] = [];
      try {
        const outcome = await listContexts(files);
        if (cancelled) return;
        found = outcome.contexts ?? [];
        if (outcome.error) setContextsError(outcome.error);
        const saved = loadTabsState();
        if (saved && outcome.error) {
          // The list failed, not the clusters: reconciling against nothing would
          // strip every workspace's cluster ids and the next change would persist
          // that. Trust the disk until the backend answers — unless there is
          // nothing to trust: `parseStoredState` can legitimately return zero
          // workspaces (every stored one failed to parse), and installing that
          // raw skips `reconcile`, the only thing that restores a default
          // workspace. `currentWorkspace()` on an empty list is `undefined`,
          // and `useTabs` dereferencing it is a render-time crash — the one
          // thing "boot must always reach `setBooted(true)`" exists to prevent.
          setState(saved.workspaces.length > 0 ? saved : defaultState(found));
        } else {
          setState(saved ? reconcile(saved, found) : defaultState(found));
        }
      } catch (error) {
        if (cancelled) return;
        console.error("could not restore the workspaces", error);
        setState(defaultState(found));
      }
      setContexts(found);
      setBooted(true);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Persist only once booted, or the empty pre-boot state would be written
  // over the real one on the way in.
  useEffect(() => {
    if (!booted) return;
    const off = subscribe(() => scheduleSave(getState()));
    const offUnload = installFlushOnUnload();
    return () => {
      off();
      offUnload();
      // Unmounting is the other way this window ends, and `beforeunload` does
      // not fire for it: a design switch, or the gallery going up, would
      // otherwise throw away up to a debounce interval of changes.
      flushSave();
    };
  }, [booted]);

  // Every cluster you are looking at gets probed once, so the rail shows link
  // state and the status bar shows a version without waiting to be asked. The
  // effect runs per workspace rather than per render — switching away and back
  // re-runs it, and the probe store's memory is what keeps it to once each.
  const workspaceId = workspace.id;
  useEffect(() => {
    if (!booted) return;
    if (!active) return;
    const byId = new Map(contexts.map((c) => [c.stableId, c]));
    for (const id of currentWorkspace().clusters) {
      if (getInfo(id)) continue;
      const ctx = byId.get(id);
      if (ctx) void probeCluster(ctx);
    }
    // `contexts` rides along because a kubeconfig change replaces them; the
    // workspace id is the trigger for the switch case.
  }, [booted, contexts, workspaceId, active]);

  // Read at call time rather than closed over: an effect installed once must
  // act on whatever the strip shows now, not whatever it showed at mount.
  function run(action: WindowAction) {
    const w = currentWorkspace();
    switch (action.type) {
      case "close-tab":
        return closeTab(w.activeId);
      case "new-tab":
        return newTab("/", activeCtx?.name);
      case "reopen-tab":
        return reopenClosed();
      case "prev-tab":
        return cycleTab(-1);
      case "next-tab":
        return cycleTab(1);
      case "select-tab":
        return selectIndex(action.index);
      case "console":
        return setOpen(true);
      case "zoom-in":
        return zoom("in");
      case "zoom-out":
        return zoom("out");
      case "zoom-reset":
        return zoom("reset");
    }
  }

  // Read at call time rather than closed over: an effect installed once must
  // act on whatever the strip shows now, not whatever it showed at mount.
  const runRef = useRef(run);
  useEffect(() => {
    runRef.current = run;
  });

  useEffect(() => {
    if (!active) return;
    function onKey(e: KeyboardEvent) {
      const action = matchWindowKey(e, apple);
      if (!action) return;
      // In web mode these chords are the browser's own — neither dispatched
      // nor preventDefault-ed, so they fall through exactly as they would with
      // no listener here at all. Zoom is the browser's native zoom (core's
      // uiScale doc); ⌘W/⌘T/⌘1-9 are tab chords a page cannot cancel, so
      // acting on them here would desync this workspace's tabs from the one
      // the browser just closed/opened/switched to. `reopen-tab`,
      // `prev-tab`/`next-tab` and `console` collide with nothing the browser
      // owns, so they still fire.
      const browserOwned =
        action.type === "zoom-in" ||
        action.type === "zoom-out" ||
        action.type === "zoom-reset" ||
        action.type === "close-tab" ||
        action.type === "new-tab" ||
        action.type === "select-tab";
      if (browserOwned && !desktop) return;
      e.preventDefault();
      runRef.current(action);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, apple, desktop]);

  function menuFor(tab: StripTab): ContextMenuItem[] {
    return [
      { label: "Duplicate", onPick: () => duplicateTab(tab.id) },
      { label: tab.pinned ? "Unpin" : "Pin", onPick: () => togglePin(tab.id) },
      { kind: "sep" },
      { label: "Close", hint: hint("close-tab", apple), danger: true, onPick: () => closeTab(tab.id) },
      { label: "Close others", danger: true, onPick: () => closeOthers(tab.id) },
      { label: "Close to the right", danger: true, onPick: () => closeToRight(tab.id) },
      { label: "Close all", danger: true, onPick: () => closeAll() },
      { kind: "sep" },
      { label: "Reopen closed", hint: hint("reopen-tab", apple), onPick: () => reopenClosed() },
    ];
  }

  function openNewWorkspace() {
    setName("");
    setPicked(new Set(contexts.map((c) => c.stableId)));
    setCreating(true);
  }

  function create() {
    const ids = contexts.filter((c) => picked.has(c.stableId)).map((c) => c.stableId);
    // An unnamed workspace still gets one — the strip and the switcher have to
    // call it something, and an empty chip reads as a bug.
    const id = createWorkspace(name.trim() || "New workspace", ids);
    switchWorkspace(id);
    setCreating(false);
  }

  if (!booted) return <LoadingState label="Loading" />;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {active && (
        <Chrome
          controls={controls}
          clusterName={activeCtx?.name}
          onToggleTheme={onToggleTheme}
          onNewWorkspace={openNewWorkspace}
        />
      )}
      <div className="flex min-h-0 flex-1">
        {active && <Rail contexts={contexts} error={contextsError} onConnect={() => openTab("/connect")} />}
        {active && <Nav contexts={contexts} />}
        {/* `min-w-0` as well as `min-h-0`. This column holds the tab strip
            and the screen, and a flex item's implicit `min-width: auto`
            refuses to shrink below its content — so a wide screen widens the
            column, and `TabStrip`'s `overflow-x-auto` never engages because
            the box it would scroll inside has grown to fit. The strip then
            pushes its own new-tab and overflow controls off the window, which
            is where the user meets it. */}
        <div data-slot="screen-column" className="flex min-h-0 min-w-0 flex-1 flex-col">
          {active && (
            <TabStrip
              tabs={tabs}
              activeId={activeId}
              onSelect={activateTab}
              onClose={closeTab}
              menuFor={menuFor}
              onNew={() => run({ type: "new-tab" })}
              newHint={hint("new-tab", apple)}
              label="Open tabs"
            />
          )}
          <div className="relative min-h-0 flex-1">
            {tabs.map((tab) => (
              <TabSurface key={tab.id} visible={tab.id === activeId}>
                {/* A placeholder tab without a cluster of its own still leaves
                    via the cluster this window is looking at — that is the
                    context classic reopens onto. */}
                <Body
                  route={tab.route}
                  clusterName={tab.sub ?? activeCtx?.name}
                  ported={ported}
                  onOpenInClassic={onOpenInClassic}
                  onOpenGallery={onOpenGallery}
                />
              </TabSurface>
            ))}
          </div>
          {active && <Console apple={apple} />}
        </div>
        <Drawer open={creating} title="New workspace" onClose={() => setCreating(false)}>
          <div className="flex flex-col gap-3 px-3 py-3">
            <TextInput value={name} onValueChange={setName} placeholder="Workspace name" aria-label="Workspace name" />
            {contexts.map((c) => (
              <Checkbox
                key={c.stableId}
                checked={picked.has(c.stableId)}
                onChange={(checked) =>
                  setPicked((prev) => {
                    const next = new Set(prev);
                    if (checked) next.add(c.stableId);
                    else next.delete(c.stableId);
                    return next;
                  })
                }
                label={c.name}
              />
            ))}
            <div className="flex justify-end">
              <Button variant="primary" size="sm" onClick={() => create()}>
                Create
              </Button>
            </div>
          </div>
        </Drawer>
      </div>
      {active && <Status contexts={contexts} />}
    </div>
  );
}
