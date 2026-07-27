import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsView } from "./SettingsView";
import { DEFAULT_WORKSPACE_LAYOUT } from "../lib/settings";

// These tests exercise desktop-only UI (native kubeconfig picker, in-app
// updater, relaunch) that Task 5 gates behind `isTauri()`. Give the suite a
// Tauri context so those blocks render as before; web-mode rendering of the
// same sections is covered separately (WebKubeconfigSection.test.tsx).
beforeEach(() => {
  (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
});
afterEach(() => {
  delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
});

const fileMocks = vi.hoisted(() => ({
  pickKubeconfigFiles: vi.fn(),
  savePastedKubeconfig: vi.fn(),
}));

vi.mock("../lib/files", () => fileMocks);

const updaterMocks = vi.hoisted(() => ({
  checkForUpdate: vi.fn(),
  installUpdate: vi.fn(),
}));
vi.mock("../lib/updater", () => updaterMocks);

const transportMocks = vi.hoisted(() => ({
  appVersion: vi.fn(async () => "0.1.0"),
  relaunchApp: vi.fn(async () => {}),
}));
vi.mock("../transport/transport", () => transportMocks);

vi.mock("../lib/clusters", () => ({
  listContexts: () =>
    Promise.resolve({
      contexts: [
        { name: "prod-eu", cluster: "production", server: "https://prod.example", isCurrent: true },
        { name: "staging", cluster: "staging", server: "https://staging.example", isCurrent: false },
      ],
    }),
}));

describe("SettingsView", () => {
  beforeEach(() => localStorage.clear());

  it("separates settings and edits context identity", async () => {
    const onContextProfilesChange = vi.fn();
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={onContextProfilesChange}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );

    expect(screen.getByText("Choose a palette and display mode. Changes apply immediately.")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: /Contexts/ }));
    await userEvent.click(screen.getByRole("tab", { name: "Appearance" }));
    const displayName = await screen.findByRole("textbox", { name: "Display name for prod-eu" });
    fireEvent.change(displayName, { target: { value: "Production Europe" } });
    expect(onContextProfilesChange).toHaveBeenCalledWith({
      "prod-eu": { displayName: "Production Europe" },
    });
  });

  it("accepts a custom logo URL", async () => {
    const onContextProfilesChange = vi.fn();
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{ "prod-eu": { logo: "custom" } }}
        onContextProfilesChange={onContextProfilesChange}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Contexts/ }));
    await userEvent.click(screen.getByRole("tab", { name: "Appearance" }));
    const url = await screen.findByRole("textbox", { name: "Custom logo URL for prod-eu" });
    fireEvent.change(url, { target: { value: "https://example.com/logo.png" } });
    expect(onContextProfilesChange).toHaveBeenCalledWith({
      "prod-eu": { logo: "custom", logoUrl: "https://example.com/logo.png" },
    });
  });

  it("moves contexts in the persisted order", async () => {
    const onContextOrderChange = vi.fn();
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={onContextOrderChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Contexts/ }));
    await userEvent.click(screen.getByRole("tab", { name: "Appearance" }));
    const moveDown = await screen.findByRole("button", { name: "Move prod-eu down" });
    fireEvent.click(moveDown);
    expect(onContextOrderChange).toHaveBeenCalledWith(["staging", "prod-eu"]);
  });

  it("reorders contexts using pointer dragging on the grip", async () => {
    const onContextOrderChange = vi.fn();
    const { container } = render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={onContextOrderChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Contexts/ }));
    await userEvent.click(screen.getByRole("tab", { name: "Appearance" }));
    await screen.findByText("Context identity");
    const rows = container.querySelectorAll<HTMLButtonElement>(".fl-context-manager__list > div > button");
    const grip = rows[0].querySelector<HTMLElement>(".fl-context-manager__grip")!;
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => rows[1]),
    });
    fireEvent.pointerDown(grip, { pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerMove(grip, { pointerId: 1, clientX: 10, clientY: 50 });
    fireEvent.pointerUp(grip, { pointerId: 1, clientX: 10, clientY: 50 });
    expect(onContextOrderChange).toHaveBeenCalledWith(["staging", "prod-eu"]);
    Reflect.deleteProperty(document, "elementFromPoint");
  });

  it("saves and adds a pasted kubeconfig", async () => {
    fileMocks.savePastedKubeconfig.mockResolvedValue("/app/kubeconfigs/team.yaml");
    const onKubeconfigFilesChange = vi.fn();
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={onKubeconfigFilesChange}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Contexts/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Paste" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Pasted kubeconfig name" }), {
      target: { value: "Team" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Kubeconfig YAML" }), {
      target: { value: "apiVersion: v1\nkind: Config\ncontexts: []" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add kubeconfig" }));
    expect(fileMocks.savePastedKubeconfig).toHaveBeenCalledWith(
      "apiVersion: v1\nkind: Config\ncontexts: []",
      "Team",
    );
    await waitFor(() =>
      expect(onKubeconfigFilesChange).toHaveBeenCalledWith(["/app/kubeconfigs/team.yaml"]),
    );
  });

  it("checks for updates and reports up to date", async () => {
    updaterMocks.checkForUpdate.mockResolvedValue(null);
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Updates/ }));
    expect(await screen.findByText("0.1.0")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(/up to date/i)).toBeDefined();
    expect(updaterMocks.checkForUpdate).toHaveBeenCalledWith("stable");
  });

  it("opens directly on the section named by initialSection", async () => {
    updaterMocks.checkForUpdate.mockResolvedValue(null);
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
        initialSection="updates"
      />,
    );
    // The Updates pane is shown without clicking the nav first.
    expect(await screen.findByRole("button", { name: "Check for updates" })).toBeDefined();
  });

  it("checks the dev channel when selected and persists the choice", async () => {
    updaterMocks.checkForUpdate.mockResolvedValue(null);
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Updates/ }));
    fireEvent.click(await screen.findByRole("button", { name: /^Dev\b/ }));
    fireEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    await waitFor(() => expect(updaterMocks.checkForUpdate).toHaveBeenCalledWith("dev"));
    expect(localStorage.getItem("srelens.updateChannel")).toBe("dev");
  });

  it("downloads an available update and offers a restart", async () => {
    updaterMocks.checkForUpdate.mockResolvedValue({
      version: "0.2.0",
      currentVersion: "0.1.0",
      notes: "New things",
    });
    updaterMocks.installUpdate.mockImplementation(
      async (_channel: string, onProgress?: (pct: number | null) => void) => {
        onProgress?.(100);
      },
    );
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Updates/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(/0\.2\.0/)).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: /Download & install/ }));
    await waitFor(() =>
      expect(updaterMocks.installUpdate).toHaveBeenCalledWith("stable", expect.any(Function)),
    );
    fireEvent.click(await screen.findByRole("button", { name: /Restart srelens/ }));
    await waitFor(() => expect(transportMocks.relaunchApp).toHaveBeenCalledTimes(1));
  });

  it("offers package-manager guidance instead of install for external installs", async () => {
    updaterMocks.checkForUpdate.mockResolvedValue({
      version: "0.2.0",
      currentVersion: "0.1.0",
      notes: "New things",
      external: true,
    });
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Updates/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(/0\.2\.0/)).toBeDefined();
    expect(screen.getByText(/system package manager/)).toBeDefined();
    expect(screen.queryByRole("button", { name: /Download & install/ })).toBeNull();
  });

  it("surfaces update check failures", async () => {
    updaterMocks.checkForUpdate.mockRejectedValue(new Error("endpoint unreachable"));
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Updates/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText(/endpoint unreachable/)).toBeDefined();
  });

  it("shows a delete context button and triggers onDeleteContext on click", async () => {
    const onDeleteContext = vi.fn().mockResolvedValue(undefined);

    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
        onDeleteContext={onDeleteContext}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Contexts/ }));
    await userEvent.click(screen.getByRole("tab", { name: "Appearance" }));
    // Wait for the context details panel to render for the default selection (prod-eu)
    const removeButton = await screen.findByRole("button", { name: "Remove context" });
    fireEvent.click(removeButton);

    const confirmButton = await screen.findByRole("button", { name: "Remove" });
    fireEvent.click(confirmButton);

    await waitFor(() => expect(onDeleteContext).toHaveBeenCalledWith("prod-eu"));
  });

  it("opens a right-click menu on a context row and confirms removal from there", async () => {
    const onDeleteContext = vi.fn().mockResolvedValue(undefined);

    const { container } = render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
        onDeleteContext={onDeleteContext}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Contexts/ }));
    await userEvent.click(screen.getByRole("tab", { name: "Appearance" }));
    await screen.findByText("Context identity");

    const row = container.querySelector<HTMLButtonElement>('button[data-context-name="staging"]')!;
    fireEvent.contextMenu(row);

    const menuRemove = await screen.findByText("Remove context", { selector: '[data-slot="context-menu-item"]' });
    fireEvent.click(menuRemove);

    const confirmButton = await screen.findByRole("button", { name: "Remove" });
    fireEvent.click(confirmButton);

    await waitFor(() => expect(onDeleteContext).toHaveBeenCalledWith("staging"));
  });

  it("hides the Updates/MCP nav items and the request-timeout slider on the web", async () => {
    delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
    render(
      <SettingsView
        theme={{ name: "slate", mode: "dark" }}
        onThemeNameChange={() => {}}
        onThemeModeChange={() => {}}
        defaultNamespace=""
        onDefaultNamespaceChange={() => {}}
        layout={DEFAULT_WORKSPACE_LAYOUT}
        onLayoutChange={() => {}}
        contextProfiles={{}}
        onContextProfilesChange={() => {}}
        kubeconfigFiles={[]}
        onKubeconfigFilesChange={() => {}}
        contextOrder={[]}
        onContextOrderChange={() => {}}
      />,
    );

    // The desktop-only nav entries are gone entirely (not just empty panes).
    expect(screen.queryByRole("button", { name: /Updates/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /^MCP/ })).toBeNull();

    // The request-timeout slider is a no-op on the web (set_request_timeout
    // isn't a web command), so it's hidden rather than shown-but-broken.
    fireEvent.click(screen.getByRole("button", { name: /Kubernetes/ }));
    expect(await screen.findByLabelText("Default namespace")).toBeDefined();
    expect(screen.queryByLabelText("Cluster request timeout in seconds")).toBeNull();
  });
});
