import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import React from "react";

// Capture the Tauri event handler App registers for the macOS Cmd+W menu item,
// and a stub window so we can assert tab-close vs. window-close behavior.
const tauri = vi.hoisted(() => {
  const handlers = new Map<string, (e: { payload: unknown }) => void>();
  const windowClose = vi.fn();
  const windowDestroy = vi.fn();
  return {
    handlers,
    windowClose,
    windowDestroy,
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
vi.mock("./lib/updater", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./lib/updater")>()),
  checkForUpdate: checkForUpdateMock,
}));
vi.mock("./lib/notify", () => ({
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
    activeCluster,
  }: {
    onSelect: (c: string, k: string) => void;
    activeCluster: string;
  }) => <button onClick={() => onSelect(activeCluster, "services")}>nav-services</button>,
}));
vi.mock("./components/ClusterOverview", () => ({
  ClusterOverview: ({ context }: { context: string }) => (
    <div data-testid="overview">{context}</div>
  ),
}));
vi.mock("./components/ResourceBrowser", () => ({
  RESOURCE_LABELS: {
    overview: "Overview",
    pods: "Pods",
    services: "Services",
    settings: "Settings",
    assistant: "Assistant",
  },
  K8S_KIND: { overview: "", pods: "Pod", services: "Service", settings: "", assistant: "" },
  ResourceBrowser: ({
    context,
    kind,
    query,
    onViewChange,
    onOpenResource,
    onOpenEdit,
  }: {
    context: string;
    kind: string;
    query?: string;
    onViewChange?: (patch: { query?: string }) => void;
    onOpenResource?: (target: { kind: string; namespace: string | null; name: string }) => void;
    onOpenEdit?: (kind: string, namespace: string | null, name: string) => void;
  }) => (
    <div data-testid="browser">
      {context}:{kind}
      <span data-testid="browser-query">{query ?? ""}</span>
      <button onClick={() => onViewChange?.({ query: "nginx" })}>set-query</button>
      <button
        onClick={() => onOpenResource?.({ kind: "Pod", namespace: "default", name: "web-1" })}
      >
        linked-pod
      </button>
      <button onClick={() => onOpenEdit?.("Deployment", "default", "web")}>edit-web</button>
    </div>
  ),
}));
vi.mock("./components/SettingsView", () => ({
  SettingsView: () => <div data-testid="settings">workspace settings</div>,
}));
// The dock hosts xterm, which is dynamically imported and has no place in
// jsdom; these tests only care about whether it is mounted and with what.
vi.mock("./components/Dock", () => ({
  Dock: ({ sessions }: { sessions: Array<{ kind: string; context: string }> }) => (
    <div data-testid="dock">{sessions.map((s) => `${s.kind}:${s.context}`).join(",")}</div>
  ),
}));
// The host shell is desktop-only, and `isWeb` is decided once at import time,
// so it has to be replaced rather than set up per test. `isTauri` is left real:
// flipping it too would switch on every Tauri-only effect in these tests.
vi.mock("./transport/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./transport/platform")>()),
  isWeb: false,
}));
const { listContextsMock } = vi.hoisted(() => ({ listContextsMock: vi.fn() }));
vi.mock("./lib/clusters", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./lib/clusters")>()),
  listContexts: listContextsMock,
}));
vi.mock("./components/EditResourceTab", () => ({
  EditResourceTab: ({ kind, name }: { kind: string; name: string }) => (
    <div data-testid="edit-tab">
      {kind}/{name}
    </div>
  ),
}));

import { App } from "./App";

const context = (name: string) => ({
  name,
  stableId: `/k/config#${name}`,
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
    expect(screen.getByTestId("edit-tab").textContent).toBe("Deployment/web");
    expect(screen.getByRole("tab", { name: /edit: Deployment\/web/ })).toBeDefined();

    // Re-edit the same resource from the services tab → focuses, doesn't duplicate.
    fireEvent.click(screen.getByRole("tab", { name: /Services/ }));
    fireEvent.click(screen.getByText("edit-web"));
    expect(screen.getAllByRole("tab", { name: /edit: Deployment\/web/ })).toHaveLength(1);
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
});
