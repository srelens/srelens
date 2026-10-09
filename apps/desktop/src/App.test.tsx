import { describe, it, expect, vi, beforeEach, afterEach, onTestFinished } from "vitest";
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import React from "react";

// Capture the Tauri event handler App registers for the macOS Cmd+W menu item,
// and a stub window so we can assert tab-close vs. window-close behavior.
vi.mock("@srelens/ui-next/extensions", async original => ({
  ...await original<typeof import("@srelens/ui-next/extensions")>(),
  ExtensionWarning: () => <div data-testid="extension-warning">Unsigned app warning</div>,
}));
const tauri = vi.hoisted(() => {
  const handlers = new Map<string, (e: { payload: unknown }) => void>();
  // Promise-returning, like the real commands: the close path chains a
  // `.catch()` onto destroy(), which a bare vi.fn() would make explode.
  const windowClose = vi.fn(() => Promise.resolve());
  const windowDestroy = vi.fn(() => Promise.resolve());
  return {
    handlers,
    windowClose,
    windowDestroy,
    // Undefined unless a test names it: every test written before the deep-link
    // block ran with no label at all, and keeps doing so.
    windowLabel: undefined as string | undefined,
    closeRequestedHandler: null as null | ((event: { preventDefault: () => void }) => unknown),
    listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) => {
      handlers.set(name, cb);
      return Promise.resolve(() => handlers.delete(name));
    }),
  };
});
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: tauri.windowLabel,
    close: tauri.windowClose,
    destroy: tauri.windowDestroy,
    // Capture the handler so a test can drive the close-request path.
    onCloseRequested: (handler: (event: { preventDefault: () => void }) => unknown) => {
      tauri.closeRequestedHandler = handler;
      return Promise.resolve(() => {
        tauri.closeRequestedHandler = null;
      });
    },
  }),
}));

const { checkForUpdateMock, notifyUpdateAvailableMock } = vi.hoisted(() => ({
  checkForUpdateMock: vi.fn(),
  notifyUpdateAvailableMock: vi.fn(),
}));
vi.mock("@srelens/core/lib/updater", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/lib/updater")>()),
  checkForUpdate: checkForUpdateMock,
}));
// What the backend's deep-link queue holds, handed out once per drain the way
// `take_pending_deep_links` drains it. Every other command goes to the real
// transport, as it did before this mock existed.
const deepLinks = vi.hoisted(() => ({ queue: [] as string[] }));
vi.mock("@srelens/core/transport", async (importOriginal) => {
  const real = await importOriginal<typeof import("@srelens/core/transport")>();
  return {
    ...real,
    invokeCommand: async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
      if (command !== "take_pending_deep_links") return real.invokeCommand<T>(command, args);
      const drained = deepLinks.queue;
      deepLinks.queue = [];
      return drained as T;
    },
  };
});
const hostNotices = vi.hoisted(() => ({ listen: vi.fn(() => () => {}) }));
vi.mock("@srelens/core/lib/hostNotices", () => ({ listenForHostNotices: hostNotices.listen }));
vi.mock("@srelens/core/lib/notify", () => ({
  notify: { success: vi.fn(), error: vi.fn(), info: vi.fn(), updateAvailable: notifyUpdateAvailableMock },
}));

vi.mock("./components/ClusterHotbar", () => ({
  ClusterHotbar: ({
    onOpenContext,
    onOpenSettings,
    onOpenAssistant,
  }: {
    onOpenContext: (c: string) => void;
    onOpenSettings: () => void;
    onOpenAssistant?: () => void;
  }) => (
    <div>
      <button onClick={() => onOpenContext("kind-dev")}>open-kind-dev</button>
      <button onClick={() => onOpenContext("prod")}>open-prod</button>
      <button onClick={onOpenSettings}>open-settings</button>
      <button onClick={onOpenAssistant}>open-assistant</button>
    </div>
  ),
}));
vi.mock("./components/AssistantTab", () => ({
  AssistantTab: ({ cluster, namespace }: { cluster: string | null; namespace?: string }) => (
    <div data-testid="assistant-tab">
      {cluster ?? "none"}:{namespace ?? ""}
    </div>
  ),
}));
vi.mock("./components/Sidebar", () => ({
  Sidebar: ({
    onSelect,
    onOpenApp,
    activeKind,
    activeCluster,
  }: {
    onSelect: (c: string, k: string) => void;
    onOpenApp: (c:string,id:string,page:string) => void;
    activeKind?:string;
    activeCluster: string;
  }) => <><span data-testid="sidebar-active-kind">{activeKind}</span><button onClick={() => onSelect(activeCluster, "services")}>nav-services</button><button onClick={()=>onOpenApp(activeCluster,"org.srelens.flux","kustomizations")}>nav-app</button></>,
}));
vi.mock("./components/Extensions",()=>({ClassicAppPage:({context,id,page,namespace,onNamespace}:{context:string;id:string;page:string;namespace:string;onNamespace?(namespace:string):void})=><><div data-testid="app-page">{context}:{id}:{page}</div><div data-testid="app-namespace">{namespace}</div><button onClick={()=>onNamespace?.("flux-system")}>app-namespace-change</button></>}));
vi.mock("./components/ClusterOverview", () => ({
  ClusterOverview: ({ context }: { context: string }) => (
    <div data-testid="overview">{context}</div>
  ),
}));
// The kind tables moved to lib/kinds; the reduced set stays mocked here so the
// sidebar renders five entries rather than the real forty.
vi.mock("@srelens/core/lib/kinds", () => ({
  RESOURCE_LABELS: {
    overview: "Overview",
    pods: "Pods",
    services: "Services",
    settings: "Settings",
    assistant: "Assistant",
    newresource: "New Resource",
    editresource: "Edit Resource",
  },
  K8S_KIND: {
    overview: "",
    pods: "Pod",
    services: "Service",
    settings: "",
    assistant: "",
    newresource: "",
    editresource: "",
  },
}));
vi.mock("./components/ResourceBrowser", () => ({
  ResourceBrowser: ({
    context,
    kind,
    query,
    onViewChange,
    onOpenResource,
    onOpenEdit,
    onOpenNew,
    onNamespaceChange,
    onOpenTerminal,
    initialNamespace,
  }: {
    context: string;
    kind: string;
    query?: string;
    onViewChange?: (patch: { query?: string }) => void;
    onOpenResource?: (target: { kind: string; namespace: string | null; name: string }) => void;
    onOpenEdit?: (kind: string, namespace: string | null, name: string) => void;
    onOpenNew?: (initialKind?: string) => void;
    onNamespaceChange?: (namespace: string) => void;
    onOpenTerminal?: (s: {
      context: string;
      namespace: string;
      pod: string;
      deleteOnClose?: { context: string; namespace: string; pod: string };
    }) => void;
    initialNamespace?: string;
  }) => (
    <div data-testid="browser">
      {context}:{kind}
      <span data-testid="browser-namespace">{initialNamespace ?? ""}</span>
      <span data-testid="browser-query">{query ?? ""}</span>
      <button onClick={() => onViewChange?.({ query: "nginx" })}>set-query</button>
      <button
        onClick={() => onOpenResource?.({ kind: "Pod", namespace: "default", name: "web-1" })}
      >
        linked-pod
      </button>
      <button onClick={() => onOpenEdit?.("Deployment", "default", "web")}>edit-web</button>
      <button onClick={() => onOpenNew?.("Secret")}>new-secret</button>
      <button onClick={() => onNamespaceChange?.("team-a")}>use-team-a</button>
      <button onClick={() => onOpenNew?.("ConfigMap")}>new-config-map</button>
      <button
        onClick={() =>
          onOpenTerminal?.({
            context,
            namespace: "default",
            pod: "srelens-node-debug-x1",
            deleteOnClose: { context, namespace: "default", pod: "srelens-node-debug-x1" },
          })
        }
      >
        open-node-shell
      </button>
    </div>
  ),
}));
vi.mock("./components/SettingsView", () => ({
  SettingsView: () => <div data-testid="settings">workspace settings</div>,
}));
// The dock hosts xterm, which is dynamically imported and has no place in
// jsdom; these tests only care about whether it is mounted and with what.
vi.mock("./components/Dock", () => ({
  Dock: ({
    sessions,
    onCloseTab,
  }: {
    sessions: Array<{ id: number; kind: string; context: string }>;
    onCloseTab?: (id: number) => void;
  }) => (
    <>
      <div data-testid="dock">{sessions.map((s) => `${s.kind}:${s.context}`).join(",")}</div>
      {sessions.map((s) => (
        <button key={s.id} onClick={() => onCloseTab?.(s.id)}>
          close-dock-{s.id}
        </button>
      ))}
    </>
  ),
}));
const { deletePodMock } = vi.hoisted(() => ({ deletePodMock: vi.fn(async () => ({})) }));
vi.mock("@srelens/core/lib/workloads", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/lib/workloads")>()),
  deletePod: deletePodMock,
}));
// The host shell is desktop-only, and `isWeb` is decided once at import time,
// so it has to be replaced rather than set up per test. `isTauri` is left real:
// flipping it too would switch on every Tauri-only effect in these tests.
vi.mock("@srelens/core/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/platform")>()),
  isWeb: false,
}));
const { listContextsMock } = vi.hoisted(() => ({ listContextsMock: vi.fn() }));
vi.mock("@srelens/core/lib/clusters", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/lib/clusters")>()),
  listContexts: listContextsMock,
}));
vi.mock("./components/EditResourceTab", () => ({
  EditResourceTab: ({
    kind,
    name,
    draft,
    onDraftChange,
    onEdited,
  }: {
    kind: string;
    name: string;
    draft: string | null;
    onDraftChange: (yaml: string) => void;
    onEdited?: () => void;
  }) => (
    <div data-testid="edit-tab">
      {kind}/{name}
      <textarea
        aria-label="mock edit draft"
        value={draft ?? "kind: Deployment\nmetadata:\n  name: web\n"}
        onChange={(event) => onDraftChange(event.target.value)}
      />
      <button onClick={onEdited}>mock apply succeeds</button>
    </div>
  ),
}));
vi.mock("./components/NewResourceEditor", () => ({
  NewResourceEditor: ({
    initialKind,
    namespace,
    draft,
    onDraftChange,
    onCreated,
  }: {
    initialKind?: string;
    namespace?: string;
    draft?: { template: string; yaml: string };
    onDraftChange: (draft: { template: string; yaml: string }) => void;
    onCreated?: () => void;
  }) => {
    const current = draft ?? {
      template: initialKind ?? "Deployment",
      yaml: `kind: ${initialKind ?? "Deployment"}\nmetadata:\n  name: stock\n`,
    };
    return (
      <div data-testid="new-resource-tab">
        <span>{current.template}</span>
        <span data-testid="new-resource-namespace">{namespace}</span>
        <textarea
          aria-label="mock new draft"
          value={current.yaml}
          onChange={(event) => onDraftChange({ ...current, yaml: event.target.value })}
        />
        <button onClick={onCreated}>mock create succeeds</button>
      </div>
    );
  },
}));

import { App } from "./App";
import { HANDOFF_KEY } from "./design";
import { flushSaveOpenTabs } from "@srelens/core";
import { notify } from "@srelens/core/lib/notify";

const context = (name: string) => ({
  name,
  stableId: `/k/config#${name}`, key: `/k/config#${name}`,
  cluster: name,
  server: "https://example",
  isCurrent: false,
});

beforeEach(() => {
  checkForUpdateMock.mockReset();
  checkForUpdateMock.mockResolvedValue(null); // up to date unless a test says otherwise
  notifyUpdateAvailableMock.mockReset();
  listContextsMock.mockReset();
  listContextsMock.mockResolvedValue({ contexts: [context("kind-dev"), context("prod")] });
});

describe("App", () => {
  it("checks for updates on startup and toasts, linking to the Updates section", async () => {
    // The update-check poll is desktop-only (Task 5 gates it behind
    // `isTauri()`) — give this test a Tauri context so the effect runs.
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    checkForUpdateMock.mockResolvedValue({ version: "0.3.0", currentVersion: "0.2.0", notes: "" });
    render(<App />);
    await waitFor(() => expect(notifyUpdateAvailableMock).toHaveBeenCalledWith("0.3.0", expect.any(Function)));
    // The toast's action opens the Settings tab (deep-linked to Updates).
    const onView = notifyUpdateAvailableMock.mock.calls[0][1] as () => void;
    onView();
    expect(await screen.findByTestId("settings")).toBeDefined();
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("says the classic design is deprecated, with the way to the new one", () => {
    render(<App />);
    expect(screen.getByText(/classic design is deprecated/i)).toBeDefined();
    expect(screen.getByRole("button", { name: "Switch to the new design" })).toBeDefined();
  });

  it("tells the deprecation banner about unsaved editor drafts before it switches", async () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("new-secret"));
    fireEvent.change(screen.getByLabelText("mock new draft"), {
      target: { value: "kind: Secret\nmetadata:\n  name: unsaved\n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Switch to the new design" }));
    expect(await screen.findByText(/discards 1 unsaved editor draft/i)).toBeDefined();
  });

  it("shows the welcome state until a cluster is opened", () => {
    render(<App />);
    expect(screen.getByText(/pure-Rust Kubernetes UI/)).toBeDefined();
    expect(screen.queryByTestId("overview")).toBeNull();
  });

  it("opens a terminal for a chosen context with no tabs open at all", async () => {
    // The dock used to mount only alongside an open tab, so a shell started
    // from the landing page went nowhere — the session existed and nothing
    // rendered it (#257).
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Open kubectl terminal" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "prod" }));
    expect((await screen.findByTestId("dock")).textContent).toBe("shell:prod");
  });

  it("opening a cluster lands on its Overview tab", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    expect(screen.getByTestId("overview").textContent).toBe("kind-dev");
    expect(screen.getByRole("tab", { name: /Overview · kind-dev/ })).toBeDefined();
  });

  it("selecting a resource opens a separate (cluster, kind) tab", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services")); // sidebar → Services

    expect(screen.getByTestId("browser").textContent).toContain("kind-dev:services");
    expect(screen.getByRole("tab", { name: /Overview · kind-dev/ })).toBeDefined();
    expect(screen.getByRole("tab", { name: /Services · kind-dev/ })).toBeDefined();

    fireEvent.click(screen.getByRole("tab", { name: /Overview · kind-dev/ }));
    expect(screen.getByTestId("overview").textContent).toBe("kind-dev");
  });

  it("opens linked Kubernetes resources in their product view", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("linked-pod"));
    expect(screen.getByTestId("browser").textContent).toContain("kind-dev:pods");
  });

  it("clears only the target tab's search when focusing a resource in it (#254)", () => {
    // The detail opens from the UNFILTERED rows, so a leftover search would
    // leave the user on a list that doesn't contain what they navigated to
    // once the drawer closes.
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("linked-pod")); // creates the pods tab
    expect(screen.getByTestId("browser").textContent).toContain("kind-dev:pods");

    fireEvent.click(screen.getByText("set-query"));
    expect(screen.getByTestId("browser-query").textContent).toBe("nginx");

    // Navigate to a pod again from Services: the existing pods tab is reused
    // and its search must be cleared so the focused row is actually listed.
    fireEvent.click(screen.getByRole("tab", { name: /Services/ }));
    fireEvent.click(screen.getByText("set-query")); // Services keeps its own
    fireEvent.click(screen.getByText("linked-pod"));
    expect(screen.getByTestId("browser").textContent).toContain("kind-dev:pods");
    expect(screen.getByTestId("browser-query").textContent).toBe("");

    // The Services tab's own search survived — only the target was cleared.
    fireEvent.click(screen.getByRole("tab", { name: /Services/ }));
    expect(screen.getByTestId("browser-query").textContent).toBe("nginx");
  });

  it("opens an edit tab from a resource and de-dupes re-edits", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("edit-web"));
    expect(screen.getByTestId("edit-tab").textContent).toContain("Deployment/web");
    expect(screen.getByRole("tab", { name: /edit: Deployment\/web/ })).toBeDefined();

    // Re-edit the same resource from the services tab → focuses, doesn't duplicate.
    fireEvent.click(screen.getByRole("tab", { name: /Services/ }));
    fireEvent.click(screen.getByText("edit-web"));
    expect(screen.getAllByRole("tab", { name: /edit: Deployment\/web/ })).toHaveLength(1);
  });

  it("scopes a new-resource editor to the namespace selected in its source tab (#404)", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("use-team-a"));
    fireEvent.click(screen.getByText("new-config-map"));
    expect(screen.getByTestId("new-resource-namespace").textContent).toBe("team-a");
  });

  it("keeps a namespace change in its own tab — another tab on the same cluster does not follow", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    expect(screen.getByTestId("browser-namespace").textContent).toBe("");
    // A second resource-list tab on the same cluster: following a linked pod
    // opens the Pods list, scoped to that pod's namespace.
    fireEvent.click(screen.getByText("linked-pod"));
    expect(screen.getByTestId("browser").textContent).toContain("kind-dev:pods");
    fireEvent.click(screen.getByText("use-team-a"));
    expect(screen.getByTestId("browser-namespace").textContent).toBe("team-a");

    fireEvent.click(screen.getByRole("tab", { name: /Services · kind-dev/ }));
    expect(screen.getByTestId("browser").textContent).toContain("kind-dev:services");
    expect(screen.getByTestId("browser-namespace").textContent).toBe("");

    fireEvent.click(screen.getByRole("tab", { name: /Pods · kind-dev/ }));
    expect(screen.getByTestId("browser-namespace").textContent).toBe("team-a");
  });

  it("keeps new-resource YAML in its tab while another tab is active (#403)", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("new-secret"));

    const editor = screen.getByLabelText("mock new draft") as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: "kind: Secret\nmetadata:\n  name: unsaved\n" } });
    fireEvent.click(screen.getByRole("tab", { name: /Services · kind-dev/ }));
    fireEvent.click(screen.getByRole("tab", { name: /New Resource · kind-dev/ }));
    expect((screen.getByLabelText("mock new draft") as HTMLTextAreaElement).value).toContain(
      "name: unsaved",
    );
  });

  it("keeps edit-resource YAML in its tab and does not replace it on return (#403)", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("edit-web"));

    const editor = screen.getByLabelText("mock edit draft") as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: "kind: Deployment\nmetadata:\n  name: unsaved\n" } });
    fireEvent.click(screen.getByRole("tab", { name: /Services · kind-dev/ }));
    fireEvent.click(screen.getByRole("tab", { name: /edit: Deployment\/web/ }));
    expect((screen.getByLabelText("mock edit draft") as HTMLTextAreaElement).value).toContain(
      "name: unsaved",
    );
  });

  it("does not persist the unchanged restorable workspace for each editor keystroke", () => {
    // A draft lives on the transient tab, but that tab is excluded from
    // session restore because it may contain Secret data. Updating only that
    // draft must not keep scheduling identical settings.json writes.
    flushSaveOpenTabs();
    localStorage.clear();
    vi.useFakeTimers();
    const setItem = vi.spyOn(Storage.prototype, "setItem");

    try {
      render(<App />);
      fireEvent.click(screen.getByText("open-kind-dev"));
      fireEvent.click(screen.getByText("nav-services"));
      fireEvent.click(screen.getByText("new-secret"));

      act(() => vi.advanceTimersByTime(400));
      const writesAfterOpening = setItem.mock.calls.filter(
        ([key]) => key === "srelens.openTabs",
      ).length;

      fireEvent.change(screen.getByLabelText("mock new draft"), {
        target: { value: "kind: Secret\nstringData:\n  token: first\n" },
      });
      act(() => vi.advanceTimersByTime(400));
      fireEvent.change(screen.getByLabelText("mock new draft"), {
        target: { value: "kind: Secret\nstringData:\n  token: second\n" },
      });
      act(() => vi.advanceTimersByTime(400));

      expect(
        setItem.mock.calls.filter(([key]) => key === "srelens.openTabs"),
      ).toHaveLength(writesAfterOpening);
    } finally {
      flushSaveOpenTabs();
      setItem.mockRestore();
      vi.useRealTimers();
    }
  });

  it("clears each in-memory draft after its apply succeeds", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));

    fireEvent.click(screen.getByText("new-secret"));
    fireEvent.change(screen.getByLabelText("mock new draft"), {
      target: { value: "kind: Secret\nmetadata:\n  name: created\n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "mock create succeeds" }));
    expect((screen.getByLabelText("mock new draft") as HTMLTextAreaElement).value).toContain(
      "name: stock",
    );

    fireEvent.click(screen.getByRole("tab", { name: /Services · kind-dev/ }));
    fireEvent.click(screen.getByText("edit-web"));
    fireEvent.change(screen.getByLabelText("mock edit draft"), {
      target: { value: "kind: Deployment\nmetadata:\n  name: edited\n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "mock apply succeeds" }));
    expect((screen.getByLabelText("mock edit draft") as HTMLTextAreaElement).value).toContain(
      "name: web",
    );
  });

  it("opens views across multiple clusters and closes tabs", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("open-prod"));

    expect(screen.getByTestId("overview").textContent).toBe("prod");
    expect(screen.getByRole("tab", { name: /Overview · prod/ })).toBeDefined();

    fireEvent.click(screen.getByLabelText("Close Overview · prod"));
    expect(screen.queryByRole("tab", { name: /Overview · prod/ })).toBeNull();
    expect(screen.getByTestId("overview").textContent).toBe("kind-dev");
  });

  it("focuses an existing tab instead of duplicating it", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("nav-services")); // again → no duplicate

    expect(screen.getAllByRole("tab", { name: /Services · kind-dev/ })).toHaveLength(1);
  });

  it("opens settings as a global workspace tab", () => {
    render(<App />);
    fireEvent.click(screen.getByText("open-settings"));

    expect(screen.getByTestId("settings").textContent).toBe("workspace settings");
    expect(screen.getByRole("tab", { name: /^Settings$/ })).toBeDefined();
    expect(screen.queryByText("nav-services")).toBeNull();
  });

  it("opens the assistant as a global workspace tab", () => {
    // The assistant drives Tauri-only backend commands, so its entry point is
    // gated behind `isTauri()` — give this test a desktop context.
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    render(<App />);
    fireEvent.click(screen.getByText("open-assistant"));

    expect(screen.getByTestId("assistant-tab").textContent).toBe("none:");
    expect(screen.getByRole("tab", { name: /^Assistant$/ })).toBeDefined();

    // Re-triggering focuses the same tab instead of duplicating it.
    fireEvent.click(screen.getByText("open-settings"));
    fireEvent.click(screen.getByText("open-assistant"));
    expect(screen.getAllByRole("tab", { name: /^Assistant$/ })).toHaveLength(1);
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("hides the assistant entry point in a web build", () => {
    // No Tauri context: the hotbar must not offer to open the assistant, since
    // the web server has no agent/chat commands to back it.
    render(<App />);
    fireEvent.click(screen.getByText("open-assistant"));
    expect(screen.queryByTestId("assistant-tab")).toBeNull();
    expect(screen.queryByRole("tab", { name: /^Assistant$/ })).toBeNull();
  });

  it("waits for the settings write before letting the window close (#254)", async () => {
    // The durable write is an async IPC round trip; an unload handler returns
    // immediately and the WebView is torn down mid-write, losing the last
    // sort/search. The close is intercepted and resumed instead.
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    tauri.windowDestroy.mockClear();
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));

    expect(tauri.closeRequestedHandler).toBeTypeOf("function");
    const preventDefault = vi.fn();
    await tauri.closeRequestedHandler!({ preventDefault });

    // The default close is cancelled, then re-issued as destroy() once the
    // write has drained — close() would re-enter this handler and loop.
    expect(preventDefault).toHaveBeenCalled();
    expect(tauri.windowDestroy).toHaveBeenCalled();
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("still closes when destroy() is refused, rather than wedging the window (#425)", async () => {
    // `core:window:allow-destroy` was never granted, so the destroy that
    // re-issues the cancelled close was rejected by the ACL and nothing closed
    // the window: the macOS red traffic light did nothing, and only Cmd+Q —
    // which quits without reaching this handler — could shut the app down.
    // The grant is the fix; this is the belt, because any rejection here has
    // the same cost, and losing a flush is cheaper than losing the quit.
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    tauri.windowDestroy.mockClear().mockRejectedValueOnce(new Error("window.destroy not allowed"));
    tauri.windowClose.mockClear();
    render(<App />);

    expect(tauri.closeRequestedHandler).toBeTypeOf("function");
    await tauri.closeRequestedHandler!({ preventDefault: vi.fn() });
    expect(tauri.windowClose).toHaveBeenCalled();

    // close() re-emits this event, so the second pass has to let it through —
    // cancelling the close it just asked for is how a fallback becomes a loop.
    const second = vi.fn();
    await tauri.closeRequestedHandler!({ preventDefault: second });
    expect(second).not.toHaveBeenCalled();
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("`?` opens the shortcut cheat sheet", () => {
    render(<App />);
    fireEvent.keyDown(window, { key: "?", shiftKey: true });
    expect(screen.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeDefined();
  });

  it("`?` typed into a field stays a question mark", () => {
    // The sheet's key carries no modifier, so it has to yield to typing —
    // otherwise searching for "why?" opens a help overlay mid-word.
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    const field = document.createElement("input");
    document.body.appendChild(field);
    field.focus();
    fireEvent.keyDown(field, { key: "?", shiftKey: true, bubbles: true });
    expect(screen.queryByRole("dialog", { name: "Keyboard shortcuts" })).toBeNull();
    field.remove();
  });

  it("close-active-tab (Cmd+W) closes the active tab, not the window", () => {
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    tauri.windowClose.mockClear();
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("open-prod"));
    expect(screen.getByTestId("overview").textContent).toBe("prod");

    const handler = tauri.handlers.get("close-active-tab");
    expect(handler).toBeDefined();
    act(() => handler!({ payload: undefined }));

    expect(screen.queryByRole("tab", { name: /Overview · prod/ })).toBeNull();
    expect(screen.getByTestId("overview").textContent).toBe("kind-dev");
    expect(tauri.windowClose).not.toHaveBeenCalled();
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("persists the post-close session when Cmd+W closes the last tab (#254)", async () => {
    // closeView only SCHEDULES the state change, and the window close fires in
    // the same callback — so without queueing the post-close snapshot first,
    // the flush writes the pre-close one and the closed tab returns on the
    // next launch.
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    localStorage.clear();
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));

    tauri.handlers.get("close-active-tab")?.({ payload: null });
    // Drive the close the way Tauri would, so the flush runs.
    await tauri.closeRequestedHandler?.({ preventDefault: vi.fn() });

    // Nothing to restore: the user closed their last tab deliberately.
    expect(localStorage.getItem("srelens.openTabs")).toBeNull();
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("close-active-tab (Cmd+W) closes the window when the last tab is closed", () => {
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    tauri.windowClose.mockClear();
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));

    const handler = tauri.handlers.get("close-active-tab");
    expect(handler).toBeDefined();
    act(() => handler!({ payload: undefined }));

    expect(tauri.windowClose).toHaveBeenCalledTimes(1);
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });

  it("reopens the view the new design handed over, once the contexts are known", async () => {
    // A design switch reloads the document, so the handoff rides in
    // sessionStorage — and is consumed exactly once, here. Routed only after
    // `contexts` resolves, mirroring the deep-link gate: a cold-start handoff
    // judged against an empty list would be rejected as unknown.
    sessionStorage.setItem(HANDOFF_KEY, JSON.stringify({ context: "prod", kind: "pods" }));
    render(<App />);
    // ResourceBrowser renders `{context}:{kind}` first; the rest is the
    // mock's own buttons.
    expect((await screen.findByTestId("browser")).textContent).toContain("prod:pods");
  });

  it("opens nothing for a handoff naming an unknown context, and still clears it", async () => {
    // Keeping an unreadable handoff would reopen a view on every later launch;
    // dropping it silently on an unknown name would hide that it was dropped.
    sessionStorage.setItem(HANDOFF_KEY, JSON.stringify({ context: "ghost", kind: "pods" }));
    render(<App />);
    await waitFor(() => expect(sessionStorage.getItem(HANDOFF_KEY)).toBeNull());
    expect(screen.queryByTestId("browser")).toBeNull();
    expect(screen.queryByTestId("overview")).toBeNull();
  });

  // A context window carries the cluster it was opened for in `?context=`, and
  // that identifier is the context's `key` — from both designs. Not the
  // `stableId`: a kubeconfig `a` declaring `b#c` and a kubeconfig `a#b`
  // declaring `c` produce the same stable id, so a window keyed on one could
  // open on the other (#623). Not a display name either. Classic tabs are
  // keyed by display name, so the query has to be resolved against the listed
  // contexts before it becomes a tab — seeded raw, a key names no cluster, and
  // the first `refreshContexts` prunes the window's only tab because
  // `resolveStoredKey` matches names and never ids.
  const withContextQuery = (value: string) => {
    localStorage.clear();
    vi.mocked(notify.error).mockClear();
    window.history.replaceState({}, "", `/?context=${encodeURIComponent(value)}`);
  };

  afterEach(() => {
    window.history.replaceState({}, "", "/");
    localStorage.clear();
  });

  it("opens the overview for the context a window's key query names", async () => {
    // `/k/config#prod` is `context("prod")`'s key — what both Rail and
    // classic Sidebar put in the query when they open a window for prod.
    withContextQuery("/k/config#prod");
    render(<App />);
    expect((await screen.findByTestId("overview")).textContent).toBe("prod");
  });

  /**
   * Two contexts can share a `stableId`; none share a `key`. Resolving the
   * query by the stable id opened this window on whichever of the pair was
   * listed first, and left the other unopenable (#623).
   */
  it("resolves the query by key, so two clusters sharing a stable id do not swap", async () => {
    listContextsMock.mockResolvedValue({
      contexts: [
        { name: "left", stableId: "a#b#c", key: "a#b%23c", cluster: "left", server: "", isCurrent: false },
        { name: "right", stableId: "a#b#c", key: "a%23b#c", cluster: "right", server: "", isCurrent: false },
      ],
    });
    withContextQuery("a%23b#c");
    render(<App />);
    expect((await screen.findByTestId("overview")).textContent).toBe("right");
  });

  it("does not treat a display name as a window identity", async () => {
    // A name that equals another cluster's key must not open that other
    // cluster. The query is an id only; a bare display name is "not listed".
    withContextQuery("prod");
    render(<App />);
    await waitFor(() =>
      expect(vi.mocked(notify.error)).toHaveBeenCalledWith(
        "Couldn't open that cluster",
        "It is not among the listed kube contexts.",
      ),
    );
    expect(screen.queryByTestId("overview")).toBeNull();
  });

  it("says the cluster could not be opened when the query names no listed context", async () => {
    // An empty window is not an answer: the reader asked for a cluster, and
    // which of the two facts this is — the list failed, or the list answered
    // and it is not there — decides whether retrying could help.
    withContextQuery("ghost");
    render(<App />);
    await waitFor(() =>
      expect(vi.mocked(notify.error)).toHaveBeenCalledWith(
        "Couldn't open that cluster",
        "It is not among the listed kube contexts.",
      ),
    );
    expect(screen.queryByTestId("overview")).toBeNull();
  });

  it("reports a failed context list as a failed list, not as a missing cluster", async () => {
    listContextsMock.mockResolvedValue({ contexts: [], error: "kubeconfig unreadable" });
    withContextQuery("/k/config#prod");
    render(<App />);
    await waitFor(() =>
      expect(vi.mocked(notify.error)).toHaveBeenCalledWith(
        "Couldn't open that cluster",
        "kubeconfig unreadable",
      ),
    );
  });

  it("opens a readable context even when another kubeconfig failed to list", async () => {
    listContextsMock.mockResolvedValue({
      contexts: [
        { name: "prod", stableId: "/k/config#prod", key: "/k/config#prod", cluster: "prod", server: "", isCurrent: false },
      ],
      error: "other kubeconfig unreadable",
    });
    withContextQuery("/k/config#prod");
    render(<App />);
    expect((await screen.findByTestId("overview")).textContent).toBe("prod");
    expect(vi.mocked(notify.error)).not.toHaveBeenCalled();
  });

  it("clears a restored session when the window's ?context= target is gone", async () => {
    // ClusterHotbar can leave other clusters' tabs in a context window's save.
    // When the requested target is cleanly absent, those must not stay active.
    listContextsMock.mockResolvedValue({ contexts: [context("prod")] });
    withContextQuery("/k/config#gone");
    localStorage.setItem(
      "srelens.openTabs",
      JSON.stringify({
        tabs: [{ id: 1, cluster: "prod", kind: "overview", namespace: "default" }],
        activeTabId: 1,
      }),
    );
    render(<App />);
    await waitFor(() =>
      expect(vi.mocked(notify.error)).toHaveBeenCalledWith(
        "Couldn't open that cluster",
        "It is not among the listed kube contexts.",
      ),
    );
    expect(screen.queryByTestId("overview")).toBeNull();
  });

  it("opens the overview once a failed context list later succeeds", async () => {
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    listContextsMock.mockResolvedValue({ contexts: [], error: "kubeconfig unreadable" });
    withContextQuery("/k/config#prod");
    render(<App />);
    await waitFor(() => expect(vi.mocked(notify.error)).toHaveBeenCalled());
    expect(screen.queryByTestId("overview")).toBeNull();

    listContextsMock.mockResolvedValue({
      contexts: [
        { name: "prod", stableId: "/k/config#prod", key: "/k/config#prod", cluster: "prod", server: "", isCurrent: false },
      ],
    });
    await waitFor(() => expect(tauri.handlers.has("kubeconfig-changed")).toBe(true));
    act(() => {
      tauri.handlers.get("kubeconfig-changed")?.({ payload: null });
    });
    expect((await screen.findByTestId("overview")).textContent).toBe("prod");
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  });
});

it("does not show an app developer banner on the landing screen or settings", () => {
  render(<App />);
  expect(screen.queryByTestId("extension-warning")).toBeNull();
  fireEvent.click(screen.getByText("open-settings"));
  expect(screen.queryByTestId("extension-warning")).toBeNull();
});

it("opens classic app pages in distinct cluster-bound tabs without replacing Overview",()=>{
 render(<App/>);fireEvent.click(screen.getByText("open-kind-dev"));fireEvent.click(screen.getByText("nav-app"));
 expect(screen.getByTestId("app-page").textContent).toBe("kind-dev:org.srelens.flux:kustomizations");
 expect(screen.getByTestId("sidebar-active-kind").textContent).toBe("");
 expect(screen.getByText("kustomizations")).toBeTruthy();
 fireEvent.click(screen.getByText("open-prod"));fireEvent.click(screen.getByText("nav-app"));
 expect(screen.getByTestId("app-page").textContent).toBe("prod:org.srelens.flux:kustomizations");
 fireEvent.click(screen.getByRole("tab",{name:/kustomizations · kind-dev/}));
 expect(screen.getByTestId("app-page").textContent).toContain("kind-dev:");
 fireEvent.click(screen.getByRole("tab",{name:/Overview · kind-dev/}));
 expect(screen.getByTestId("overview").textContent).toBe("kind-dev");
});

it("retains each classic app tab namespace across tab switches",()=>{
 render(<App/>);
 fireEvent.click(screen.getByText("open-kind-dev"));
 fireEvent.click(screen.getByText("nav-app"));
 fireEvent.click(screen.getByText("app-namespace-change"));
 fireEvent.click(screen.getByText("open-prod"));
 fireEvent.click(screen.getByText("nav-app"));
 expect(screen.getByTestId("app-namespace").textContent).toBe("");
 fireEvent.click(screen.getByRole("tab",{name:/kustomizations · kind-dev/}));
 expect(screen.getByTestId("app-namespace").textContent).toBe("flux-system");
});

// #735: a helm operation outlives the window that started it, and the desktop
// host broadcasts how it ended to every window. Classic listens for that from
// the moment it mounts, and lets go when it unmounts.
it("shows what the desktop host reports, and lets it go on unmount", () => {
  const release = vi.fn();
  hostNotices.listen.mockReset().mockReturnValue(release);
  (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
  try {
    const { unmount } = render(<App />);
    expect(hostNotices.listen).toHaveBeenCalledTimes(1);
    unmount();
    expect(release).toHaveBeenCalledTimes(1);
  } finally {
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
  }
});

// #734: a node shell's debug pod is privileged, and on desktop the host now
// deletes it — when its shell ends, its window closes or reloads, or srelens
// quits — so the page must not delete it as well. The web host has no such
// cleanup, so there the page still does.
describe("closing a node shell's dock tab", () => {
  beforeEach(() => deletePodMock.mockClear());

  function openAndCloseANodeShell() {
    render(<App />);
    fireEvent.click(screen.getByText("open-kind-dev"));
    fireEvent.click(screen.getByText("nav-services"));
    fireEvent.click(screen.getByText("open-node-shell"));
    fireEvent.click(screen.getByText(/^close-dock-/));
  }

  it("leaves the debug pod to the desktop host", () => {
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    try {
      openAndCloseANodeShell();
      expect(deletePodMock).not.toHaveBeenCalled();
    } finally {
      delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
    }
  });

  it("still deletes it on the web, where no host does", () => {
    openAndCloseANodeShell();
    expect(deletePodMock).toHaveBeenCalledWith("kind-dev", "default", "srelens-node-debug-x1");
  });
});

// Classic's half of #36/#370: drained on the main desktop window, judged
// against the listed contexts, then routed — or refused with a reason. Both
// designs share the rule set (`checkDeepLink` in core), so these pin what
// classic does with each outcome rather than the rules themselves.
describe("srelens:// deep links", () => {
  beforeEach(() => {
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    tauri.windowLabel = "main";
    deepLinks.queue = [];
    vi.mocked(notify.error).mockClear();
  });
  afterEach(() => {
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
    tauri.windowLabel = undefined;
    deepLinks.queue = [];
  });

  it("opens a cluster link on that cluster's Overview", async () => {
    deepLinks.queue = ["srelens://cluster/prod"];
    render(<App />);
    expect((await screen.findByTestId("overview")).textContent).toBe("prod");
    expect(screen.getByRole("tab", { name: /Overview · prod/ })).toBeDefined();
  });

  it("opens a resource link in its kind's view, on the link's own cluster", async () => {
    deepLinks.queue = ["srelens://resource/prod/default/Pod/web-1"];
    render(<App />);
    expect((await screen.findByTestId("browser")).textContent).toContain("prod:pods");
    expect(screen.getByRole("tab", { name: /Pods · prod/ })).toBeDefined();
  });

  it("opens a link the backend announces after the window is up", async () => {
    render(<App />);
    await waitFor(() => expect(tauri.handlers.has("deep-link-pending")).toBe(true));
    deepLinks.queue = ["srelens://cluster/kind-dev"];
    act(() => tauri.handlers.get("deep-link-pending")?.({ payload: null }));
    expect((await screen.findByTestId("overview")).textContent).toBe("kind-dev");
  });

  it("reports each link it refuses, with the reason, and opens none of them", async () => {
    deepLinks.queue = [
      "srelens://evil/prod",
      "srelens://cluster/staging",
      "srelens://resource/prod/default/Event/web.17f",
      "srelens://resource/prod/-/Pod/web-1",
    ];
    render(<App />);
    await waitFor(() => expect(notify.error).toHaveBeenCalledTimes(4));
    expect(vi.mocked(notify.error).mock.calls).toEqual([
      ["Couldn't open that link", "It isn't a link srelens understands."],
      ["Couldn't open that link", 'No kube context named "staging".'],
      ["Couldn't open that link", "srelens can't open a Event directly."],
      ["Couldn't open that link", "Pod is namespaced, so the link needs a namespace."],
    ]);
    expect(screen.queryByTestId("overview")).toBeNull();
    expect(screen.queryByTestId("browser")).toBeNull();
  });

  /**
   * #855: a listing that failed has not said a context is missing. The links
   * naming one it did not return wait, one notice says why, and the re-list
   * that follows a fixed kubeconfig opens them.
   */
  it("holds links while the context listing has failed, says why once, and opens them when it recovers", async () => {
    // Every listing until the fix, not just the next one: the landing page
    // lists too, and its effect runs before App's.
    listContextsMock.mockResolvedValue({
      contexts: [context("kind-dev")],
      error: "open /home/dana/.kube/prod: permission denied",
    });
    // kind-dev is in the partial list and opens now; prod's two links wait.
    deepLinks.queue = [
      "srelens://cluster/prod",
      "srelens://cluster/kind-dev",
      "srelens://resource/prod/default/Pod/web-1",
    ];
    render(<App />);
    expect((await screen.findByTestId("overview")).textContent).toBe("kind-dev");
    expect(vi.mocked(notify.error).mock.calls).toEqual([
      [
        "That link will be checked once the contexts load",
        "The kube contexts could not be listed. open /home/dana/.kube/prod: permission denied",
      ],
    ]);
    expect(screen.queryByRole("tab", { name: /· prod/ })).toBeNull();

    // Another link while the listing is still failing joins the wait without a
    // second notice: the reader has already been told why.
    deepLinks.queue = ["srelens://resource/prod/default/Service/web"];
    await act(async () => tauri.handlers.get("deep-link-pending")?.({ payload: null }));
    await waitFor(() => expect(deepLinks.queue).toEqual([]));
    await act(async () => {});
    expect(notify.error).toHaveBeenCalledTimes(1);

    // The kubeconfig is readable again: the backend's watcher says so, and the
    // re-list answers cleanly with both contexts.
    listContextsMock.mockResolvedValue({ contexts: [context("kind-dev"), context("prod")] });
    await waitFor(() => expect(tauri.handlers.has("kubeconfig-changed")).toBe(true));
    act(() => tauri.handlers.get("kubeconfig-changed")?.({ payload: null }));
    expect(await screen.findByRole("tab", { name: /Overview · prod/ })).toBeDefined();
    expect(screen.getByRole("tab", { name: /Pods · prod/ })).toBeDefined();
    expect(screen.getByRole("tab", { name: /Services · prod/ })).toBeDefined();
    expect(notify.error).toHaveBeenCalledTimes(1);

    // A later failure is a new one, and is said again.
    listContextsMock.mockResolvedValue({
      contexts: [context("kind-dev")],
      error: "open /home/dana/.kube/prod: permission denied",
    });
    const listings = listContextsMock.mock.calls.length;
    act(() => tauri.handlers.get("kubeconfig-changed")?.({ payload: null }));
    await waitFor(() => expect(listContextsMock.mock.calls.length).toBeGreaterThan(listings));
    await act(async () => {});
    // Nor has it said prod is gone, so prod's tabs stay open (#855 review).
    expect(screen.getByRole("tab", { name: /Overview · prod/ })).toBeDefined();
    deepLinks.queue = ["srelens://cluster/prod"];
    await act(async () => tauri.handlers.get("deep-link-pending")?.({ payload: null }));
    await waitFor(() => expect(notify.error).toHaveBeenCalledTimes(2));
    expect(vi.mocked(notify.error).mock.calls[1][0]).toBe("That link will be checked once the contexts load");
  });

  it("closes a restored tab only once a listing that answered lacks its context, and says so then", async () => {
    vi.mocked(notify.info).mockClear();
    onTestFinished(() => localStorage.removeItem("srelens.openTabs"));
    localStorage.setItem(
      "srelens.openTabs",
      JSON.stringify({ tabs: [{ id: 1, cluster: "prod", kind: "overview", namespace: "default" }], activeTabId: 1 }),
    );
    listContextsMock.mockResolvedValue({
      contexts: [context("kind-dev")],
      error: "open /home/dana/.kube/prod: permission denied",
    });
    render(<App />);
    await waitFor(() => expect(tauri.handlers.has("kubeconfig-changed")).toBe(true));
    await act(async () => {});
    expect(screen.getByRole("tab", { name: /Overview · prod/ })).toBeDefined();
    expect(notify.info).not.toHaveBeenCalled();

    listContextsMock.mockResolvedValue({ contexts: [context("kind-dev")] });
    act(() => tauri.handlers.get("kubeconfig-changed")?.({ payload: null }));
    await waitFor(() => expect(screen.queryByRole("tab", { name: /Overview · prod/ })).toBeNull());
    expect(notify.info).toHaveBeenCalledWith("Closed 1 restored tab", "Their cluster context is no longer available.");
  });

  it("still refuses at once, while the listing has failed, what no listing can change", async () => {
    listContextsMock.mockResolvedValue({
      contexts: [context("kind-dev")],
      error: "open /home/dana/.kube/prod: permission denied",
    });
    deepLinks.queue = [
      "srelens://evil/prod",
      "srelens://resource/prod/default/Event/web.17f",
      "srelens://resource/prod/-/Pod/web-1",
    ];
    render(<App />);
    await waitFor(() => expect(notify.error).toHaveBeenCalledTimes(3));
    expect(vi.mocked(notify.error).mock.calls).toEqual([
      ["Couldn't open that link", "It isn't a link srelens understands."],
      ["Couldn't open that link", "srelens can't open a Event directly."],
      ["Couldn't open that link", "Pod is namespaced, so the link needs a namespace."],
    ]);
  });
});
