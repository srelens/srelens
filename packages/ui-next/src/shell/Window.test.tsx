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
import { contextFor, resetContexts } from "../lib/clusters";

const ctx = (stableId: string, name = stableId) => ({ name, stableId, cluster: name, server: "", isCurrent: false });

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
    // The row is the middle `flex min-h-0 flex-1` that holds Rail/Nav/the tab
    // column — an exact class match, since that string is unique to it.
    const row = document.querySelector('div[class="flex min-h-0 flex-1"]');
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
