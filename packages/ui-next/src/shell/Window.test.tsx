import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { fireEvent } from "@testing-library/react";

// `vi.hoisted` because `vi.mock` is hoisted above every declaration in the
// file, and the tabsPersist factory reads these the moment `./Window` imports
// the module — a plain `const` is still in its temporal dead zone by then.
const {
  listContexts,
  loadTabsState,
  scheduleSave,
  installFlushOnUnload,
  flushSave,
  connectCluster,
  listCrds,
  getForwards,
  rehydrateForwards,
  subscribeForwards,
  isApplePlatform,
  isTauri,
  loadKubeconfigFiles,
  vaultStatus,
  vaultLock,
  vaultUnlockPassword,
  zoomSpy,
  createWorkspaceSpy,
  switchWorkspaceSpy,
} = vi.hoisted(() => ({
  listContexts: vi.fn(),
  loadTabsState: vi.fn(),
  scheduleSave: vi.fn(),
  installFlushOnUnload: vi.fn(() => () => {}),
  flushSave: vi.fn(),
  connectCluster: vi.fn(),
  listCrds: vi.fn(),
  getForwards: vi.fn(() => []),
  rehydrateForwards: vi.fn(async () => {}),
  subscribeForwards: vi.fn(() => () => {}),
  isApplePlatform: vi.fn(() => true),
  isTauri: vi.fn(() => true),
  loadKubeconfigFiles: vi.fn((): string[] => []),
  vaultStatus: vi.fn(),
  vaultLock: vi.fn(),
  vaultUnlockPassword: vi.fn(),
  zoomSpy: vi.fn(),
  createWorkspaceSpy: vi.fn(),
  switchWorkspaceSpy: vi.fn(),
}));

vi.mock("@srelens/core", async (importOriginal) => {
  const real = await importOriginal<typeof import("@srelens/core")>();
  return {
    ...real,
    listContexts: (...a: unknown[]) => listContexts(...a),
    connectCluster: (...a: unknown[]) => connectCluster(...a),
    listCrds: (...a: unknown[]) => listCrds(...a),
    getForwards: () => getForwards(),
    rehydrateForwards: () => rehydrateForwards(),
    subscribeForwards: (...a: Parameters<typeof subscribeForwards>) => subscribeForwards(...a),
    isApplePlatform: () => isApplePlatform(),
    isTauri: () => isTauri(),
    loadKubeconfigFiles: () => loadKubeconfigFiles(),
    vaultStatus: () => vaultStatus(),
    vaultLock: () => vaultLock(),
    vaultUnlockPassword: (...a: unknown[]) => vaultUnlockPassword(...a),
  };
});

vi.mock("../lib/tabsPersist", () => ({ loadTabsState, scheduleSave, installFlushOnUnload, flushSave }));

// The zoom helper lives in Chrome (shared with its buttons); spied rather than
// replaced outright so Chrome itself still renders for real.
vi.mock("./Chrome", async (importOriginal) => {
  const real = await importOriginal<typeof import("./Chrome")>();
  return { ...real, zoom: (...a: Parameters<typeof real.zoom>) => zoomSpy(...a) };
});

// createWorkspace/switchWorkspace spied the same way — a pass-through so the
// real store still drives every other test in this file.
vi.mock("../lib/tabsStore", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/tabsStore")>();
  return {
    ...real,
    createWorkspace: (...a: Parameters<typeof real.createWorkspace>) => {
      createWorkspaceSpy(...a);
      return real.createWorkspace(...a);
    },
    switchWorkspace: (...a: Parameters<typeof real.switchWorkspace>) => {
      switchWorkspaceSpy(...a);
      return real.switchWorkspace(...a);
    },
  };
});

/**
 * One extra route, and nothing else changed.
 *
 * `/settings` has no entry in the real `SCREENS` table yet (Task 10 adds it),
 * so the only screen a reader can reach through this window is the
 * Placeholder — and the Placeholder is handed no `onLocked`. Diverting a route
 * that does not otherwise exist is what lets this file prove the whole
 * injection path Task 8 specified (`Window` -> `Body` -> the screen's
 * `onLocked`) without replacing `Body`, `screenFor` or the routes table for
 * the other forty tests here, every one of which still resolves its routes for
 * real.
 */
vi.mock("../lib/routes", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/routes")>();
  const LockProbe = ({ onLocked }: import("../lib/routes").RoutedScreenProps) => (
    <button type="button" onClick={onLocked}>
      seal the workspace
    </button>
  );
  return {
    ...real,
    screenFor: (route: string) => (route === "/lock-probe" ? LockProbe : real.screenFor(route)),
  };
});

// jsdom has no ResizeObserver; TabStrip's overflow Popover wants one.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import { ConsoleProvider } from "../console";
import { Window } from "./Window";
import * as store from "../lib/tabsStore";
import { resetProbes } from "../lib/probe";
import { resetView } from "../lib/workspace";
import { defaultState, makeTab } from "../lib/tabs";
import { defaultMark, getMark, setMark, MARKS_KEY } from "../lib/marks";
import { contextFor, getContextsError, getContextsStatus, resetContexts } from "../lib/clusters";
import { resetLock } from "./LockGate";

/** An open vault: the state every test in this file but the lock ones needs. */
const VAULT_OPEN = {
  mode: "unlocked" as const,
  keySource: "password" as const,
  biometricAvailable: false,
  biometricEnrolled: false,
};

/** A sealed one. `biometricEnrolled` is false so no Touch ID sheet is raised. */
const VAULT_SEALED = {
  mode: "locked" as const,
  keySource: "password-locked" as const,
  biometricAvailable: false,
  biometricEnrolled: false,
};

const ctx = (stableId: string, name = stableId) => ({
  name, stableId, cluster: name, server: "", isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
});

beforeEach(() => {
  listContexts.mockReset().mockResolvedValue({ contexts: [ctx("prod")] });
  loadTabsState.mockReset().mockReturnValue(null);
  scheduleSave.mockReset();
  connectCluster.mockReset().mockImplementation(async (name: string) => ({ context: name, reachable: true, version: "1.30" }));
  listCrds.mockReset().mockResolvedValue({ crds: [] });
  getForwards.mockReset().mockReturnValue([]);
  subscribeForwards.mockReset().mockReturnValue(() => {});
  isApplePlatform.mockReset().mockReturnValue(true);
  isTauri.mockReset().mockReturnValue(true);
  loadKubeconfigFiles.mockReset().mockReturnValue(["/home/u/.kube/config", "/home/u/.kube/other"]);
  // Every test in this file but the lock ones runs with an OPEN vault: the
  // gate mounted above the tab strip covers the whole middle band while the
  // vault is sealed, so a file-wide default of `locked` would leave no strip
  // for any of them to find.
  vaultStatus.mockReset().mockResolvedValue(VAULT_OPEN);
  vaultLock.mockReset().mockResolvedValue(undefined);
  vaultUnlockPassword.mockReset().mockResolvedValue(undefined);
  resetLock();
  zoomSpy.mockReset();
  createWorkspaceSpy.mockReset();
  switchWorkspaceSpy.mockReset();
  // Cleared so "flushes on unload" can actually fail: a spy that is never
  // cleared stays called from the first test in the file onwards.
  installFlushOnUnload.mockClear();
  flushSave.mockClear();
  store.setState(defaultState([]));
  resetProbes();
  resetView();
  resetContexts();
});

async function booted() {
  render(
    <ConsoleProvider>
      <Window ported={[]} onOpenInClassic={() => {}} />
    </ConsoleProvider>,
  );
  await waitFor(() => expect(screen.getByRole("tablist")).toBeDefined());
}

describe("Window boot", () => {
  it("builds a Default workspace from the contexts when nothing was saved", async () => {
    await booted();
    expect(store.getState().workspaces[0].name).toBe("Default");
    expect(store.getState().workspaces[0].clusters).toEqual(["prod"]);
  });

  it("restores a saved state and reconciles it against the contexts", async () => {
    const saved = defaultState([ctx("prod"), ctx("gone")]);
    saved.workspaces[0].tabs.push(makeTab("/k/pods"));
    loadTabsState.mockReturnValue(saved);
    await booted();
    expect(store.getState().workspaces[0].clusters).toEqual(["prod"]);
    expect(screen.getByRole("tab", { name: /Pods/ })).toBeDefined();
  });

  it("still boots when listing contexts fails", async () => {
    listContexts.mockResolvedValue({ error: "kubeconfig unreadable" });
    await booted();
    expect(store.getState().workspaces).toHaveLength(1);
    expect(store.getState().workspaces[0].clusters).toEqual([]);
  });

  it("keeps the saved workspaces untouched when the cluster list errors", async () => {
    const saved = {
      workspaces: [{ id: "w1", name: "Team", clusters: ["prod"], tabs: [makeTab("/")], activeId: "", closed: [] }],
      currentId: "w1",
    };
    saved.workspaces[0].activeId = saved.workspaces[0].tabs[0].id;
    loadTabsState.mockReturnValue(saved);
    listContexts.mockResolvedValue({ error: "kubeconfig unreadable" });
    render(
      <ConsoleProvider>
        <Window ported={[]} onOpenInClassic={() => {}} />
      </ConsoleProvider>,
    );
    await screen.findByRole("tablist");
    // reconcile(saved, []) would have stripped "prod"; a transient failure must not.
    expect(store.currentWorkspace().clusters).toEqual(["prod"]);
  });

  it("shows a loading state rather than the wrong tabs before boot resolves", () => {
    let resolve!: (v: unknown) => void;
    listContexts.mockReturnValue(new Promise((r) => (resolve = r)));
    render(
      <ConsoleProvider>
        <Window ported={[]} onOpenInClassic={() => {}} />
      </ConsoleProvider>,
    );
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.getByText(/loading/i)).toBeDefined();
    act(() => resolve({ contexts: [] }));
  });

  it("still boots when reading the saved state throws", async () => {
    // `loadTabsState` guards its own storage, but the boot body is what has to
    // survive: an exception here rejected the IIFE, `setBooted(true)` never
    // ran, and the spinner stayed up forever with no Placeholder and no way
    // back to classic.
    loadTabsState.mockImplementation(() => {
      throw new DOMException("denied", "SecurityError");
    });
    await booted();
    expect(store.getState().workspaces[0].clusters).toEqual(["prod"]);
  });

  it("boots to a usable window when every saved workspace fails to parse and the cluster list also errors", async () => {
    // `parseStoredState` can legitimately return zero workspaces (every stored
    // one failed to parse). Before the fix, the `saved && outcome.error`
    // branch installed that raw — bypassing `reconcile`, the only thing that
    // restores a default workspace — and `currentWorkspace()` returning
    // `undefined` threw at render.
    loadTabsState.mockReturnValue({ workspaces: [], currentId: "gone" });
    listContexts.mockResolvedValue({ error: "kubeconfig unreadable" });
    await booted();
    expect(store.getState().workspaces.length).toBeGreaterThan(0);
    expect(screen.getByRole("tablist")).toBeDefined();
  });

  it("saves on every store change after boot, and flushes on unload", async () => {
    await booted();
    expect(installFlushOnUnload).toHaveBeenCalled();
    act(() => store.openTab("/k/pods"));
    expect(scheduleSave).toHaveBeenCalledWith(store.getState());
  });

  it("flushes the debounced save when it unmounts", async () => {
    // Unmounting mid-debounce — the gallery round trip, a design switch —
    // dropped up to 300ms of changes: the unload listener never fires, so
    // taking it off without writing threw the pending state away.
    const view = render(
      <ConsoleProvider>
        <Window ported={[]} onOpenInClassic={() => {}} />
      </ConsoleProvider>,
    );
    await waitFor(() => expect(screen.getByRole("tablist")).toBeDefined());
    act(() => store.openTab("/k/pods"));
    expect(flushSave).not.toHaveBeenCalled();
    view.unmount();
    expect(flushSave).toHaveBeenCalled();
  });
});

describe("Window marks", () => {
  it("loads stored cluster marks at boot, so a colour set before this launch is not lost", async () => {
    localStorage.setItem(
      MARKS_KEY,
      JSON.stringify({ prod: { name: "prod", short: "PR", color: "var(--ok)", mark: "text", withText: true } }),
    );
    await booted();
    expect(getMark("prod", "prod").color).toBe("var(--ok)");
  });

  it("setting one cluster's mark does not erase another cluster's stored entry", async () => {
    // Before the fix, the module started at `marks = {}` (nothing had ever
    // loaded it), so the first `setMark` spread over an empty record and
    // persisted that — every other cluster's stored mark vanished from disk.
    localStorage.setItem(
      MARKS_KEY,
      JSON.stringify({ prod: { name: "prod", short: "PR", color: "var(--ok)", mark: "text", withText: true } }),
    );
    listContexts.mockResolvedValue({ contexts: [ctx("prod"), ctx("dev")] });
    await booted();
    act(() => setMark("dev", { ...defaultMark("dev"), color: "var(--warn)" }));
    const stored = JSON.parse(localStorage.getItem(MARKS_KEY)!);
    expect(stored.prod.color).toBe("var(--ok)");
    expect(stored.dev.color).toBe("var(--warn)");
  });
});

describe("Window cluster list error", () => {
  it("surfaces a failed cluster list to the rail instead of just leaving it empty", async () => {
    listContexts.mockResolvedValue({ error: "kubeconfig unreadable" });
    await booted();
    expect(screen.getByRole("img", { name: "kubeconfig unreadable" })).toBeDefined();
  });

  it("hands the reason to the store, so the screens can say it too and not blame the reader", async () => {
    listContexts.mockResolvedValue({ error: "kubeconfig unreadable" });
    await booted();
    expect(getContextsStatus()).toBe("failed");
    expect(getContextsError()).toBe("kubeconfig unreadable");
  });

  it("treats a listing that rejects as a failed listing, not as a cluster-less kubeconfig", async () => {
    // The `outcome.error` path was the only one being reported. A rejection
    // left the store reading "listed, and there are none" — which is the
    // reader's-fault sentence again, for a failure they cannot act on.
    listContexts.mockRejectedValue(new Error("kubeconfig unreadable"));
    await booted();
    expect(getContextsStatus()).toBe("failed");
    expect(getContextsError()).toContain("kubeconfig unreadable");
    expect(screen.getByRole("img", { name: /kubeconfig unreadable/ })).toBeDefined();
  });

  it("keeps a listing that worked out of the failed state when the workspaces are what fail", async () => {
    loadTabsState.mockImplementation(() => {
      throw new Error("storage refuses reads");
    });
    await booted();
    expect(getContextsStatus()).toBe("loaded");
    expect(getContextsError()).toBe("");
  });
});

describe("Window strip", () => {
  it("renders every tab and shows only the active one's body", async () => {
    await booted();
    act(() => store.openTab("/k/pods", { clusterName: "prod" }));
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    // Both bodies are mounted; only the active is visible.
    const headings = screen.getAllByRole("heading", { level: 1, hidden: true });
    expect(headings.map((h) => h.textContent)).toEqual(["Control room", "Pods"]);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Pods");
  });

  it("selecting a tab switches the body", async () => {
    await booted();
    act(() => store.openTab("/k/pods"));
    await userEvent.click(screen.getByRole("tab", { name: /Control room/ }));
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Control room");
  });

  it("closing a tab goes through the store", async () => {
    await booted();
    act(() => store.openTab("/k/pods"));
    await userEvent.click(screen.getByRole("button", { name: /close pods/i }));
    expect(store.currentWorkspace().tabs).toHaveLength(1);
  });

  it("new tab opens the home route as a second tab", async () => {
    await booted();
    await userEvent.click(screen.getByRole("button", { name: /new tab/i }));
    expect(store.currentWorkspace().tabs.map((t) => t.route)).toEqual(["/", "/"]);
  });

  it("names the workspace's active cluster on a new tab", async () => {
    // `contexts[0]` would have been whichever context the kubeconfig lists
    // first — not the current one, and not necessarily even in this
    // workspace. The active cluster is what a new tab is about, so its name
    // is what the tab carries.
    listContexts.mockResolvedValue({ contexts: [ctx("prod", "prod-eu")] });
    await booted();
    await userEvent.click(screen.getByRole("button", { name: /new tab/i }));
    const opened = store.currentWorkspace().tabs.at(-1)!;
    expect(opened.sub).toBe("prod-eu");
  });

  it("hands the Placeholder the way back to classic, with the cluster", async () => {
    const onOpenInClassic = vi.fn();
    render(
      <ConsoleProvider>
        <Window ported={[]} onOpenInClassic={onOpenInClassic} />
      </ConsoleProvider>,
    );
    await waitFor(() => expect(screen.getByRole("tablist")).toBeDefined());
    await userEvent.click(screen.getByRole("button", { name: /open in classic/i }));
    expect(onOpenInClassic).toHaveBeenCalledWith("/", "prod");
  });
});

describe("Window accelerators", () => {
  it("binds ⌘T to a new tab carrying the active cluster's name", async () => {
    await booted();
    fireEvent.keyDown(window, { key: "t", metaKey: true });
    const tabs = store.currentWorkspace().tabs;
    expect(tabs).toHaveLength(2);
    expect(tabs.at(-1)!.sub).toBe("prod");
  });

  it("⌘W closes the active tab and ⌘⇧T reopens it", async () => {
    await booted();
    act(() => store.openTab("/k/pods"));
    fireEvent.keyDown(window, { key: "w", metaKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(1);
    expect(store.currentWorkspace().closed).toHaveLength(1);
    fireEvent.keyDown(window, { key: "T", metaKey: true, shiftKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(2);
  });

  it("does nothing on Ctrl+W while Apple, since that chord belongs to the terminal", async () => {
    await booted();
    act(() => store.openTab("/k/pods"));
    fireEvent.keyDown(window, { key: "w", ctrlKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(2);
  });

  it("binds nothing while inactive, and takes the chrome down but not the bodies", async () => {
    render(
      <ConsoleProvider>
        <Window ported={[]} onOpenInClassic={() => {}} active={false} />
      </ConsoleProvider>,
    );
    await screen.findByText(/not in the new design yet/);
    expect(screen.queryByRole("tablist")).toBeNull();
    fireEvent.keyDown(window, { key: "t", metaKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(1);
  });

  it("probes each cluster of the workspace once at boot", async () => {
    listContexts.mockResolvedValue({ contexts: [ctx("prod"), ctx("dev")] });
    render(
      <ConsoleProvider>
        <Window ported={[]} onOpenInClassic={() => {}} />
      </ConsoleProvider>,
    );
    await screen.findByRole("tablist");
    await waitFor(() => expect(connectCluster).toHaveBeenCalledTimes(2));
    expect(connectCluster.mock.calls.map((c) => c[0])).toEqual(["prod", "dev"]);
  });

  it("offers Close others on a tab's context menu", async () => {
    await booted();
    act(() => store.openTab("/k/pods", { clusterName: "prod" }));
    act(() => store.openTab("/events"));
    fireEvent.contextMenu(screen.getByRole("tab", { name: /Pods/ }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Close others" }));
    // The pinned home tab survives by design; /events is what "others" means.
    expect(store.currentWorkspace().tabs.map((t) => t.route)).toEqual(["/", "/k/pods"]);
  });

  it("marks every close item as destructive", async () => {
    await booted();
    act(() => store.openTab("/k/pods", { clusterName: "prod" }));
    fireEvent.contextMenu(screen.getByRole("tab", { name: /Pods/ }));
    for (const name of ["Close", "Close others", "Close to the right", "Close all"]) {
      const item = await screen.findByRole("menuitem", { name });
      expect(item.getAttribute("data-danger")).toBe("true");
    }
  });

  it("zooms via the shared helper under Tauri, and eats the keystroke", async () => {
    await booted();
    const notCancelled = fireEvent.keyDown(window, { key: "=", metaKey: true });
    expect(zoomSpy).toHaveBeenCalledWith("in");
    // `false` means preventDefault() was called on a cancelable event.
    expect(notCancelled).toBe(false);
  });

  it("leaves ⌘W alone in web mode too — the browser owns it, and closing the tab under it would look like data loss", async () => {
    // A browser delivers ⌘W without letting the page cancel it; if `closeTab`
    // still ran here, `installFlushOnUnload` would persist the close as the
    // page unloads and the tab would be missing on the next visit.
    isTauri.mockReturnValue(false);
    await booted();
    act(() => store.openTab("/k/pods"));
    const notCancelled = fireEvent.keyDown(window, { key: "w", metaKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(2);
    expect(notCancelled).toBe(true);
  });

  it("leaves the browser's own zoom alone in web mode", async () => {
    // Core's uiScale doc: in a browser the native zoom already does this, so
    // the accelerator must neither dispatch nor preventDefault — a suppressed
    // keystroke that does nothing is worse than one left alone.
    isTauri.mockReturnValue(false);
    await booted();
    const notCancelled = fireEvent.keyDown(window, { key: "=", metaKey: true });
    expect(zoomSpy).not.toHaveBeenCalled();
    expect(notCancelled).toBe(true);
  });
});

describe("Window new workspace", () => {
  it("pins the drawer inside the row, not as a sibling of the status bar", async () => {
    await booted();
    await userEvent.click(screen.getByRole("button", { name: /Default/ }));
    await userEvent.click(await screen.findByRole("button", { name: "New workspace" }));
    const drawer = await screen.findByRole("complementary", { name: "Details" });
    // The row is the middle band that holds Rail/Nav/the tab column — an exact
    // class match, since that string is unique to it. `relative` joined it when
    // §25's cover was mounted here: the cover is `absolute inset-0` inside this
    // band rather than `fixed`, because the titlebar and the status bar are not
    // part of what a lock replaces.
    const row = document.querySelector('div[class="relative flex min-h-0 flex-1"]');
    expect(row).not.toBeNull();
    expect(drawer.parentElement).toBe(row);
  });

  it("creates a workspace from the switcher with the name typed and the clusters picked", async () => {
    listContexts.mockResolvedValue({ contexts: [ctx("prod"), ctx("dev")] });
    await booted();
    await userEvent.click(screen.getByRole("button", { name: /Default/ }));
    await userEvent.click(await screen.findByRole("button", { name: "New workspace" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Workspace name" }), "Team");
    // Both clusters start picked; unticking "prod" leaves only "dev".
    await userEvent.click(screen.getByRole("checkbox", { name: "prod" }));
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(createWorkspaceSpy).toHaveBeenCalledWith("Team", ["dev"]);
    expect(store.currentWorkspace().name).toBe("Team");
    expect(store.currentWorkspace().clusters).toEqual(["dev"]);
  });
});

describe("Window contexts", () => {
  it("fills the contexts store at boot, so a screen can resolve the active cluster", async () => {
    listContexts.mockResolvedValue({ contexts: [ctx("prod"), ctx("dev")] });
    await booted();
    expect(contextFor("prod")?.name).toBe("prod");
  });

  it("passes the configured kubeconfig files to listContexts", async () => {
    await booted();
    expect(listContexts).toHaveBeenCalledWith(["/home/u/.kube/config", "/home/u/.kube/other"]);
  });
});

describe("Window — the shell's own widths", () => {
  it("lets the screen column shrink, so the tab strip scrolls instead of overflowing", async () => {
    // `TabStrip`'s tablist is `overflow-x-auto` and is meant to scroll under
    // the new-tab and overflow controls. It never got the chance: this column
    // had no `min-w-0`, so a wide screen widened it, the strip grew with it,
    // and the controls went off the right edge of the window. Photographed
    // with eight tabs open and a terminal on screen.
    //
    // jsdom lays nothing out, so this pins the mechanism. Sixth time this
    // exact property has bitten on this migration.
    await booted();
    const column = document.querySelector('[data-slot="screen-column"]');
    expect(column?.className ?? "").toContain("min-w-0");
  });
});

describe("Window — what boot has to ask for", () => {
  it("asks the backend what is still forwarding, whatever route it opens on", async () => {
    // The forwards store is module-level JavaScript and a browser reload
    // empties it while the server keeps forwarding. Rehydrating only when
    // `/forwards` mounts leaves the status bar reading `0 port-forwards`
    // — on every other route — while tunnels are live and the `/pf/<id>/`
    // proxies still answer. Boot is the only place that runs regardless.
    await booted();
    await waitFor(() => expect(rehydrateForwards).toHaveBeenCalled());
  });
});

/**
 * §25's cover, proved against the real chrome rather than against a tile.
 *
 * The tab strip and the cluster rail are what PR #365 deliberately made
 * reachable while a dialog is open, which is right for a dialog and exactly
 * wrong for a lock: a cover that left them live would be worse than no lock,
 * because the window would look sealed and every other tab would still be
 * running over a sealed vault. So these tests name both by the role and the
 * accessible name the shell actually gives them — `TabStrip`'s `tablist` and
 * `ClusterRail`'s `nav aria-label="Clusters"` — and the first test in the file
 * is its own positive control: the same two queries have to FIND them with the
 * vault open, or their absence below would prove nothing.
 */
describe("Window lock cover", () => {
  it("leaves the whole window alone while the vault is open", async () => {
    await booted();
    expect(screen.getByRole("tablist")).toBeTruthy();
    expect(screen.getByRole("navigation", { name: "Clusters" })).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Control room");
    expect(screen.queryByText("Workspace locked")).toBeNull();
  });

  it("covers the tab strip and the cluster rail when the vault is sealed", async () => {
    vaultStatus.mockResolvedValue(VAULT_SEALED);
    render(
      <ConsoleProvider>
        <Window ported={[]} onOpenInClassic={() => {}} />
      </ConsoleProvider>,
    );
    expect(await screen.findByText("Workspace locked")).toBeTruthy();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("tab")).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Clusters" })).toBeNull();
    // The screen under the strip is gone too — including from the hidden tab
    // surfaces, which stay mounted for every other reason.
    expect(screen.queryByRole("heading", { level: 1, hidden: true, name: "Control room" })).toBeNull();
  });

  it("seals and covers on the lock chord", async () => {
    await booted();
    act(() => store.openTab("/k/pods", { clusterName: "prod" }));
    fireEvent.keyDown(window, { key: "L", metaKey: true, shiftKey: true });
    expect(vaultLock).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("Workspace locked")).toBeTruthy();
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Clusters" })).toBeNull();
    // The tabs themselves survive: this covers the window, it does not close
    // the session the reader comes back to.
    expect(store.currentWorkspace().tabs).toHaveLength(2);
  });

  it("covers nothing when the chord's lock is refused", async () => {
    vaultLock.mockRejectedValue(new Error("there is no vault to lock"));
    await booted();
    fireEvent.keyDown(window, { key: "L", metaKey: true, shiftKey: true });
    await waitFor(() => expect(vaultLock).toHaveBeenCalled());
    expect(screen.queryByText("Workspace locked")).toBeNull();
    expect(screen.getByRole("tablist")).toBeTruthy();
  });

  it("leaves the lock chord to the browser in web mode, where there is no vault", async () => {
    isTauri.mockReturnValue(false);
    await booted();
    fireEvent.keyDown(window, { key: "L", metaKey: true, shiftKey: true });
    expect(vaultLock).not.toHaveBeenCalled();
    expect(screen.queryByText("Workspace locked")).toBeNull();
  });

  it("hands a screen the raise function, and it covers the window rather than the tab", async () => {
    await booted();
    act(() => store.openTab("/lock-probe", { clusterName: "prod" }));
    await userEvent.click(screen.getByRole("button", { name: "seal the workspace" }));
    expect(await screen.findByText("Workspace locked")).toBeTruthy();
    // The point of the whole seam: not this tab, the window.
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("navigation", { name: "Clusters" })).toBeNull();
    expect(screen.queryByRole("button", { name: "seal the workspace" })).toBeNull();
  });
});

/**
 * The half of decision 5 the cover alone does not deliver.
 *
 * `LockGate` unmounts the rail, the nav, the strip, every tab body, the drawer
 * and the console — but `Chrome` and `Status` are its SIBLINGS, outside the
 * band, and the window's keydown listener had no sealed guard at all. Behind a
 * raised cover, ⌘T twice took the tabs from 2 to 4 and ⌘W three times took
 * them to 1; the titlebar gear opened `/settings`; seven status-bar segments
 * opened tabs; and the workspace switcher's `onRemove` deleted a workspace
 * outright, with no dialog, whenever it held one tab.
 *
 * Decision 5's whole argument is that a cover leaving these live is worse than
 * no lock, because the window LOOKS sealed. Excluding the titlebar and the
 * status bar visually is defensible per §25; leaving them interactive is not.
 */
describe("Window — what the cover has to take with it", () => {
  /** Booted with the vault open, then sealed the way `Lock now` seals it. */
  async function sealed() {
    await booted();
    act(() => store.openTab("/k/pods", { clusterName: "prod" }));
    fireEvent.keyDown(window, { key: "L", metaKey: true, shiftKey: true });
    await screen.findByText("Workspace locked");
  }

  it("acts on no tab chord while the cover is up", async () => {
    await sealed();
    expect(store.currentWorkspace().tabs).toHaveLength(2);
    fireEvent.keyDown(window, { key: "t", metaKey: true });
    fireEvent.keyDown(window, { key: "t", metaKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(2);
    fireEvent.keyDown(window, { key: "w", metaKey: true });
    fireEvent.keyDown(window, { key: "w", metaKey: true });
    fireEvent.keyDown(window, { key: "w", metaKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(2);
    // Nor the ones that reorder or reopen what is behind it.
    const activeBefore = store.currentWorkspace().activeId;
    fireEvent.keyDown(window, { key: "]", metaKey: true, shiftKey: true });
    fireEvent.keyDown(window, { key: "1", metaKey: true });
    expect(store.currentWorkspace().activeId).toBe(activeBefore);
  });

  /**
   * Zoom is the exception, and the reason is not convenience. Its whole effect
   * is on the surface the reader is looking at — the lock screen — and a
   * reader who cannot read the passphrase field cannot unlock. Taking away the
   * ability to make this screen legible would be a lock-out, not a lock.
   */
  it("still zooms while the cover is up, because that is what makes the cover readable", async () => {
    await sealed();
    fireEvent.keyDown(window, { key: "=", metaKey: true });
    expect(zoomSpy).toHaveBeenCalledWith("in");
  });

  it("offers no way into Settings from the titlebar while the cover is up", async () => {
    await sealed();
    const gear = screen.getByRole("button", { name: "Appearance settings" }) as HTMLButtonElement;
    expect(gear.disabled).toBe(true);
    await userEvent.click(gear);
    expect(store.currentWorkspace().tabs.some((t) => t.route === "/settings")).toBe(false);
  });

  /**
   * The worst of them: `askRemove` deletes a workspace with no dialog when it
   * holds one tab or fewer, and a workspace is BORN with exactly one. Behind
   * the cover that was a destructive, undoable action with no credential.
   */
  it("puts the workspace switcher out of reach while the cover is up", async () => {
    await sealed();
    const before = store.getState().workspaces.length;
    expect(screen.queryByRole("button", { name: /Default/ })).toBeNull();
    // The name is still shown — it is not a secret, and blanking it would
    // imply the vault had sealed it — but there is nothing to press.
    expect(screen.getByText("Default")).toBeTruthy();
    expect(store.getState().workspaces).toHaveLength(before);
  });

  it("leaves the status bar as readouts while the cover is up", async () => {
    await sealed();
    const strip = screen.getByRole("group", { name: "Status" });
    expect(strip.querySelectorAll("button")).toHaveLength(0);
    // The readouts themselves stay: the cluster name and the counts come from
    // files and stores the vault never sealed.
    expect(strip.textContent ?? "").toContain("prod");
  });

  it("gives every one of those back when the vault opens again", async () => {
    await sealed();
    vaultStatus.mockResolvedValue(VAULT_OPEN);
    // Only an unlock lowers the cover, so this goes through the form.
    await userEvent.type(screen.getByLabelText("Master passphrase"), "aaaa1111aaaa");
    await userEvent.click(screen.getByRole("button", { name: "Unlock workspace" }));
    await screen.findByRole("tablist");
    expect(
      (screen.getByRole("button", { name: "Appearance settings" }) as HTMLButtonElement).disabled,
    ).toBe(false);
    expect(screen.getByRole("button", { name: /Default/ })).toBeTruthy();
    expect(
      screen.getByRole("group", { name: "Status" }).querySelectorAll("button").length,
    ).toBeGreaterThan(0);
    fireEvent.keyDown(window, { key: "t", metaKey: true });
    expect(store.currentWorkspace().tabs).toHaveLength(3);
  });
});
