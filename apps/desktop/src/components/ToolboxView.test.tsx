import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";

const toolbox = vi.hoisted(() => ({
  toolboxStatus: vi.fn(),
  diagnoseContext: vi.fn(),
  searchPlugins: vi.fn(),
  startToolInstall: vi.fn(),
  installKubectl: vi.fn(),
  installPlugin: vi.fn(),
  removePlugin: vi.fn(),
  upgradePlugin: vi.fn(),
}));
vi.mock("../lib/toolbox", () => toolbox);
vi.mock("../lib/clusters", () => ({
  listContexts: vi.fn().mockResolvedValue({ contexts: [{ name: "dev" }] }),
}));

import { ToolboxView } from "./ToolboxView";

beforeEach(() => {
  Object.values(toolbox).forEach((m) => m.mockReset());
  toolbox.toolboxStatus.mockResolvedValue({
    data: [
      { name: "kubectl", installed: true, version: "v1.30.2", source: "system", path: "/usr/bin/kubectl" },
      { name: "krew", installed: false },
      { name: "helm", installed: true, version: "v3.16.2", source: "managed", path: "/h/helm" },
    ],
  });
  toolbox.searchPlugins.mockResolvedValue({ data: [] });
  toolbox.diagnoseContext.mockResolvedValue({ data: { context: "dev", items: [] } });
});

describe("ToolboxView", () => {
  it("lists managed tools with version and source, and installs a missing one with progress", async () => {
    toolbox.startToolInstall.mockImplementation(async (_tool: string, onProgress?: (p: number | null) => void) => {
      onProgress?.(42);
      return { data: { tool: "krew", version: "v0.5.0", path: "/k" } };
    });
    render(<ToolboxView />);

    expect(await screen.findByText("v1.30.2")).toBeDefined();
    expect(screen.getByText("v3.16.2")).toBeDefined();
    // krew is missing → its Install button, which streams via start_tool_install
    fireEvent.click(screen.getByRole("button", { name: "Install krew" }));
    await waitFor(() => expect(toolbox.startToolInstall).toHaveBeenCalledWith("krew", expect.any(Function)));
    // status is refreshed after install
    await waitFor(() => expect(toolbox.toolboxStatus).toHaveBeenCalledTimes(2));
  });

  it("searches the krew index and installs a plugin after confirmation", async () => {
    toolbox.searchPlugins.mockResolvedValue({
      data: [{ name: "oidc-login", description: "OIDC login", installed: false }],
    });
    toolbox.installPlugin.mockResolvedValue({ data: { plugin: "oidc-login", output: "ok" } });
    render(<ToolboxView />);
    await screen.findByText("v1.30.2");

    fireEvent.change(screen.getByLabelText("Search krew plugins"), { target: { value: "oidc" } });
    fireEvent.click(screen.getByRole("button", { name: /Search/ }));
    expect(await screen.findByText("oidc-login")).toBeDefined();
    expect(toolbox.searchPlugins).toHaveBeenCalledWith("oidc");

    // Install → confirm dialog (krew third-party caveat) → confirm
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(/community-maintained/)).toBeDefined();
    fireEvent.click(within(dialog).getByRole("button", { name: "Install" }));
    await waitFor(() => expect(toolbox.installPlugin).toHaveBeenCalledWith("oidc-login"));
  });

  it("diagnoses a context and offers a one-click fix for a missing krew plugin", async () => {
    toolbox.diagnoseContext.mockResolvedValue({
      data: {
        context: "dev",
        items: [
          { binary: "kubectl", kind: "kubectl", installable: true, status: "found", path: "/usr/bin/kubectl" },
          { binary: "kubectl-oidc_login", kind: "krew-plugin", plugin: "oidc-login", installable: true, status: "missing" },
        ],
      },
    });
    toolbox.installPlugin.mockResolvedValue({ data: { plugin: "oidc-login", output: "ok" } });
    render(<ToolboxView initialContext="dev" />);

    // Auto-diagnosed via initialContext deep-link.
    expect(await screen.findByText("kubectl-oidc_login")).toBeDefined();
    expect(screen.getByText("Missing")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Install" }));
    await waitFor(() => expect(toolbox.installPlugin).toHaveBeenCalledWith("oidc-login"));
  });
});
