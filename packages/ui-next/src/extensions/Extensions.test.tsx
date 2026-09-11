import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  isTauri: () => true,
  listExtensions: vi.fn(),
  configureExtensions: vi.fn(),
  readExtension: vi.fn(),
}));
import {
  listExtensions,
  configureExtensions,
  readExtension,
} from "@srelens/core";
import { ExtensionManager, ExtensionResults } from "./Extensions";
const plugin = {
  manifest: {
    id: "org.test.gitops",
    name: "GitOps",
    version: "0.1.0",
    permissions: ["k8s.listCustomResource"],
    capabilities: [
      { name: "list", arguments: { printerColumns: [{ name: "Ready" }] } },
    ],
    contributions: {
      pages: [{ id: "apps", title: "Applications", capability: "list" }],
      detailTabs: [],
      rowActions: [],
    },
  },
  enabled: true,
  revision: 1,
  settings: {},
} as any;
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    developerMode: false,
    nextRevision: 1,
    plugins: [],
  } as any);
  vi.mocked(configureExtensions).mockResolvedValue({} as any);
});
it("shows backend errors and retries instead of claiming no extensions", async () => {
  vi.mocked(listExtensions).mockRejectedValueOnce(new Error("disk unreadable"));
  render(<ExtensionManager />);
  expect((await screen.findByRole("alert")).textContent).toContain(
    "disk unreadable",
  );
  fireEvent.click(screen.getByText("Retry"));
  expect(await screen.findByText("No extensions installed.")).toBeTruthy();
});
it("persists developer mode only through the backend", async () => {
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByLabelText("Extension developer mode"));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "developerMode",
      enabled: true,
    }),
  );
});
it("reads the pinned context and distinguishes failed reads from empty results", async () => {
  vi.mocked(readExtension).mockRejectedValueOnce(new Error("Forbidden"));
  render(
    <ExtensionResults
      plugin={plugin}
      capability="list"
      context="staging"
      namespace="argo"
    />,
  );
  expect((await screen.findByRole("alert")).textContent).toContain("Forbidden");
  expect(readExtension).toHaveBeenCalledWith(
    plugin.manifest.id,
    1,
    "list",
    "staging",
    "argo",
  );
});

it("reviews the exact manifest and reports rejected installs without claiming success", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    developerMode: true,
    nextRevision: 1,
    plugins: [],
  });
  vi.mocked(configureExtensions).mockRejectedValueOnce(
    new Error("Unsupported API version"),
  );
  render(<ExtensionManager />);
  const source = JSON.stringify(plugin.manifest);
  fireEvent.change(
    await screen.findByLabelText("Local extension manifest (JSON)"),
    { target: { value: source } },
  );
  fireEvent.click(screen.getByText("Review manifest"));
  expect(configureExtensions).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("Install and grant permissions"));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Unsupported API version",
  );
  expect(configureExtensions).toHaveBeenCalledWith({
    action: "install",
    manifest: source,
    grants: plugin.manifest.permissions,
  });
});
it("persists settings, disable and remove through the backend", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    developerMode: true,
    nextRevision: 2,
    plugins: [plugin],
  });
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  fireEvent.change(screen.getByLabelText("Extension settings (JSON object)"), {
    target: { value: '{"team":"platform"}' },
  });
  fireEvent.click(screen.getByText("Save settings"));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "settings",
      id: plugin.manifest.id,
      settings: { team: "platform" },
    }),
  );
  await waitFor(() =>
    expect(
      (screen.getByLabelText("Enable GitOps") as HTMLInputElement).disabled,
    ).toBe(false),
  );
  fireEvent.click(screen.getByLabelText("Enable GitOps"));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "enable",
      id: plugin.manifest.id,
      enabled: false,
    }),
  );
  await waitFor(() => {
    const remove = screen.getByRole("button", { name: "Remove" }) as HTMLButtonElement;
    expect(remove.disabled).toBe(false);
    // Reload can replace the inventory between two separate async lookups.
    fireEvent.click(remove);
  });
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "remove",
      id: plugin.manifest.id,
    }),
  );
});
it("keeps reads idle until a cluster is chosen and shows successful empty results", async () => {
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
  const view = render(
    <ExtensionResults plugin={plugin} capability="list" context="" />,
  );
  expect(readExtension).not.toHaveBeenCalled();
  view.rerender(
    <ExtensionResults plugin={plugin} capability="list" context="prod" />,
  );
  expect(
    await screen.findByText("No resources returned by this extension."),
  ).toBeTruthy();
  expect(readExtension).toHaveBeenCalledWith(
    plugin.manifest.id,
    1,
    "list",
    "prod",
    "",
  );
});
it("renders printer-column values as text", async () => {
  vi.mocked(readExtension).mockResolvedValue({
    items: [
      {
        name: "app",
        namespace: "argo",
        age: "2d",
        columns: ["<script>bad()</script>"],
      },
    ],
  });
  render(<ExtensionResults plugin={plugin} capability="list" context="prod" />);
  expect(await screen.findByText("<script>bad()</script>")).toBeTruthy();
  expect(document.querySelector("script")).toBeNull();
  expect(screen.getByRole("columnheader", { name: "Ready" })).toBeTruthy();
});

it("adds namespace detail views and actions, and removes them when disabled", async () => {
  const { ExtensionResourceSlot } = await import("./Extensions");
  const installed = structuredClone(plugin);
  installed.manifest.contributions.detailTabs = [
    {
      id: "detail",
      title: "GitOps apps",
      capability: "list",
      forKinds: ["/Namespace"],
    },
  ];
  installed.manifest.contributions.rowActions = [
    {
      id: "inspect",
      title: "Inspect apps",
      capability: "list",
      forKinds: ["/Namespace"],
    },
  ];
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    developerMode: true,
    nextRevision: 2,
    plugins: [installed],
  });
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
  render(
    <ExtensionResourceSlot
      context="staging"
      kind="Namespace"
      namespace={null}
      name="argo"
    />,
  );
  expect(await screen.findByRole("tab", { name: "GitOps apps" })).toBeTruthy();
  await waitFor(() =>
    expect(readExtension).toHaveBeenCalledWith(
      plugin.manifest.id,
      1,
      "list",
      "staging",
      "argo",
    ),
  );
  fireEvent.click(screen.getByText("Extension actions"));
  fireEvent.click(screen.getByText("Inspect apps"));
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    developerMode: true,
    nextRevision: 2,
    plugins: [{ ...installed, enabled: false }],
  });
  const { EXTENSIONS_CHANGED } = await import("@srelens/core");
  fireEvent(window, new Event(EXTENSIONS_CHANGED));
  await waitFor(() =>
    expect(screen.queryByRole("tab", { name: "GitOps apps" })).toBeNull(),
  );
});
it("does not attach a custom kind contribution to a built-in with the same name", async () => {
  const { ExtensionResourceSlot } = await import("./Extensions");
  const installed = structuredClone(plugin);
  installed.manifest.contributions.detailTabs = [
    {
      id: "detail",
      title: "Custom",
      capability: "list",
      forKinds: ["acme.io/Deployment"],
    },
  ];
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    developerMode: true,
    nextRevision: 2,
    plugins: [installed],
  });
  render(
    <ExtensionResourceSlot
      context="prod"
      kind="Deployment"
      namespace="default"
      name="web"
    />,
  );
  await waitFor(() => expect(listExtensions).toHaveBeenCalled());
  expect(screen.queryByRole("tab", { name: "Custom" })).toBeNull();
  expect(readExtension).not.toHaveBeenCalled();
});
it("registers cluster-pinned extension routes with a real screen", async () => {
  const { screenFor, describe, isClusterScopedRoute } = await import(
    "../lib/routes"
  );
  const { extensionRoute } = await import("@srelens/core");
  const route = extensionRoute("prod/a", plugin.manifest.id, "apps");
  expect(screenFor(route)?.name).toBe("ExtensionPage");
  expect(describe(route, "other").sub).toBe("prod/a");
  expect(isClusterScopedRoute(route)).toBe(true);
  expect(screenFor("/extensions/")).toBeNull();
});

it("explains a missing extension API and keeps the server error collapsed", async () => {
  const installed = structuredClone(plugin);
  Object.assign(installed.manifest.capabilities[0].arguments, {
    group: "argoproj.io",
    version: "v1alpha1",
    plural: "applications",
    kind: "Application",
  });
  const raw =
    'ApiError: 404 page not found : Failed to parse error data (Status { code: 404, message: "404 page not found" })';
  vi.mocked(readExtension)
    .mockRejectedValueOnce(new Error(raw))
    .mockResolvedValueOnce({ items: [] });
  render(
    <ExtensionResults plugin={installed} capability="list" context="M01" />,
  );
  expect(await screen.findByText("Application API unavailable")).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain(
    "argoproj.io/v1alpha1",
  );
  expect(screen.getByRole("alert").textContent).toContain(
    "does not install its Kubernetes APIs",
  );
  expect(screen.getByText(raw).closest("details")?.hasAttribute("open")).toBe(
    false,
  );
  expect(
    screen.queryByText("No resources returned by this extension."),
  ).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(
    await screen.findByText("No resources returned by this extension."),
  ).toBeTruthy();
});
it.each(["ApiError: Forbidden (code: 403)", "list custom resource timed out"])(
  "does not turn %s into an API absence",
  async (message) => {
    vi.mocked(readExtension).mockRejectedValueOnce(new Error(message));
    render(
      <ExtensionResults plugin={plugin} capability="list" context="M01" />,
    );
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText(/API unavailable/)).toBeNull();
    expect(
      screen.queryByText(/does not install its Kubernetes APIs/),
    ).toBeNull();
  },
);

it("renders readable conditions and short revisions while preserving the full value", async () => {
  const revision = "main@sha1:0123456789abcdef0123456789abcdef01234567";
  const displayPlugin = structuredClone(plugin);
  displayPlugin.manifest.capabilities[0].arguments.printerColumns = [
    {name:"Ready"}, {name:"Suspended"}, {name:"Revision"},
  ];
  vi.mocked(readExtension).mockResolvedValue({items:[{name:"apps",namespace:"flux-system",age:"1d",columns:["True","false",revision]}]});
  render(<ExtensionResults plugin={displayPlugin} capability="list" context="staging"/>);
  expect(await screen.findByText("Ready", {selector:"td span"})).toBeTruthy();
  expect(screen.getByText("No", {selector:"td span"})).toBeTruthy();
  expect(screen.getByText("main@01234567").getAttribute("title")).toBe(revision);
});
