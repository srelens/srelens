import { act, render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  isTauri: () => true,
  listExtensionCatalog: vi.fn(),
  reviewCatalogExtension: vi.fn(),
  listExtensions: vi.fn(),
  configureExtensions: vi.fn(),
  validateExtension: vi.fn(),
  readExtension: vi.fn(),
  inspectExtensionResource: vi.fn(),
  saveTextFile: vi.fn(),
  listContexts: vi.fn(),
}));
import {
  listExtensionCatalog,
  reviewCatalogExtension,
  listExtensions,
  configureExtensions,
  validateExtension,
  readExtension,
  saveTextFile,
  listContexts,
} from "@srelens/core";
import { ExtensionManager, ExtensionResults } from "./Extensions";

// jsdom has no ResizeObserver, and the cluster picker's popover watches its trigger with
// one while cmdk scrolls the highlighted row into view. The same stubs the kit's
// Radix-backed suites carry, kept here so the requirement stays visible.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const elementProto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
elementProto.scrollIntoView ??= () => {};
elementProto.hasPointerCapture ??= () => false;
elementProto.setPointerCapture ??= () => {};
elementProto.releasePointerCapture ??= () => {};
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
      detailLinks: [],
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
    nextRevision: 1,
    plugins: [],
  } as any);
  vi.mocked(configureExtensions).mockResolvedValue({} as any);
  vi.mocked(validateExtension).mockResolvedValue({ errors: [] });
  vi.mocked(listContexts).mockResolvedValue({ contexts: [] });
});
it("shows backend errors and retries instead of claiming no apps", async () => {
  vi.mocked(listExtensions).mockRejectedValueOnce(new Error("disk unreadable"));
  render(<ExtensionManager />);
  expect((await screen.findByRole("alert")).textContent).toContain(
    "disk unreadable",
  );
  fireEvent.click(screen.getByText("Retry"));
  expect(await screen.findByText("No apps installed.")).toBeTruthy();
});
it("says why an app was quarantined and does not offer to re-enable it", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [
      { ...plugin, enabled: false, quarantined: "App publisher signature is invalid" },
    ],
  } as any);
  render(<ExtensionManager />);
  expect(
    (await screen.findByText(/App publisher signature is invalid/)).textContent,
  ).toContain("Remove it or reinstall it from the Catalog");
  const toggle = screen.getByLabelText("Enable GitOps") as HTMLInputElement;
  expect(toggle.checked).toBe(false);
  expect(toggle.disabled).toBe(true);
  expect(screen.getByText("Remove")).toBeTruthy();
});
it("refreshes an open list only when an action on one of its own resources is accepted", async () => {
  const { EXTENSION_RESOURCE_CHANGED } = await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue({ items: [] } as any);
  render(
    <ExtensionResults plugin={plugin} capability="list" context="staging" namespace="argo" />,
  );
  await waitFor(() => expect(readExtension).toHaveBeenCalledTimes(1));
  const changed = (detail: object) =>
    window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED, { detail }));
  const resource = { id: plugin.manifest.id, revision: 1, capability: "list", context: "staging", namespace: "argo", name: "web" };
  changed({ ...resource, context: "prod" });
  changed({ ...resource, id: "org.other.app" });
  changed({ ...resource, capability: "other" });
  changed({ ...resource, namespace: "other" });
  expect(readExtension).toHaveBeenCalledTimes(1);
  changed(resource);
  await waitFor(() => expect(readExtension).toHaveBeenCalledTimes(2));
});
it("offers native installation without a developer-mode toggle", async () => {
  render(<ExtensionManager />);
  expect(await screen.findByText("Install a local manifest")).toBeTruthy();
  expect(screen.queryByLabelText("App developer mode")).toBeNull();
  expect(configureExtensions).not.toHaveBeenCalled();
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
    true,
  );
});

it("reviews the exact manifest and reports rejected installs without claiming success", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 1,
    plugins: [],
  });
  vi.mocked(validateExtension).mockResolvedValue({ errors: [] });
  vi.mocked(configureExtensions).mockRejectedValueOnce(
    new Error("Unsupported API version"),
  );
  render(<ExtensionManager />);
  const source = JSON.stringify(plugin.manifest);
  fireEvent.change(
    await screen.findByLabelText("Local app manifest (JSON)"),
    { target: { value: source } },
  );
  fireEvent.click(screen.getByText("Review manifest"));
  expect(configureExtensions).not.toHaveBeenCalled();
  fireEvent.click(await screen.findByText("Install and grant permissions"));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Unsupported API version",
  );
  expect(configureExtensions).toHaveBeenCalledWith({
    action: "install",
    manifest: source,
    grants: plugin.manifest.permissions,
  });
});
it("lists every manifest problem with its path and does not offer to install", async () => {
  const errors = [
    { code: "EXTENSION_INVALID_ID", path: "id", message: "App ID must be a reverse-domain identifier" },
    { code: "EXTENSION_UNRESOLVED_CAPABILITY", path: "contributions.pages[0].capability", message: 'Capability "applications.delete" is not declared' },
    { code: "EXTENSION_INVALID_KIND", path: "contributions.detailTabs[0].forKinds[0]", message: "Qualify the kind with its API group" },
  ];
  vi.mocked(validateExtension).mockResolvedValue({ errors });
  render(<ExtensionManager />);
  const source = JSON.stringify(plugin.manifest);
  fireEvent.change(
    await screen.findByLabelText("Local app manifest (JSON)"),
    { target: { value: source } },
  );
  fireEvent.click(screen.getByText("Review manifest"));
  const list = await screen.findByRole("list", { name: "Manifest problems" });
  const items = within(list).getAllByRole("listitem");
  expect(items).toHaveLength(3);
  errors.forEach((error, index) => {
    expect(items[index].textContent).toContain(error.path);
    expect(items[index].textContent).toContain(error.message);
    expect(items[index].textContent).toContain(error.code);
  });
  expect(validateExtension).toHaveBeenCalledWith(source, plugin.manifest.permissions, undefined);
  expect(screen.queryByText("Install and grant permissions")).toBeNull();
  expect(configureExtensions).not.toHaveBeenCalled();
});
it("says the manifest check failed, offers a retry and does not offer to install", async () => {
  vi.mocked(validateExtension).mockRejectedValueOnce(new Error("bridge timed out"));
  render(<ExtensionManager />);
  const source = JSON.stringify(plugin.manifest);
  fireEvent.change(
    await screen.findByLabelText("Local app manifest (JSON)"),
    { target: { value: source } },
  );
  fireEvent.click(screen.getByText("Review manifest"));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("bridge timed out");
  expect(within(alert).getByText("Could not check the manifest")).toBeTruthy();
  expect(screen.queryByText("Install and grant permissions")).toBeNull();
  fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
  await screen.findByText("Install and grant permissions");
  expect(validateExtension).toHaveBeenCalledTimes(2);
  expect(validateExtension).toHaveBeenLastCalledWith(source, plugin.manifest.permissions, undefined);
  expect(configureExtensions).not.toHaveBeenCalled();
});
it("applies a manifest check only to the review that asked for it", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 1, plugins: [] });
  const source = JSON.stringify(plugin.manifest);
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: source, signature: [1, 2, 3] });
  vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [{ id: plugin.manifest.id, name: "Catalog GitOps", description: "GitOps resources", repository: "https://github.com/example/gitops", license: "MIT", release: { version: "0.1.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: true } }] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] } as any);
  let finishSigned!: (report: { errors: [] }) => void;
  vi.mocked(validateExtension)
    .mockImplementationOnce(() => new Promise((resolve) => { finishSigned = resolve; }))
    .mockResolvedValueOnce({ errors: [{ code: "EXTENSION_RESERVED_ID", path: "id", message: "Reserved for signed releases" }] });
  render(<ExtensionManager />);
  // A signed catalog review is still being checked...
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  await waitFor(() => expect(validateExtension).toHaveBeenCalledTimes(1));
  // ...when the same bytes are reviewed unsigned, and that check finds a problem.
  fireEvent.click(screen.getByRole("tab", { name: "Apps" }));
  fireEvent.change(screen.getByLabelText("Local app manifest (JSON)"), { target: { value: source } });
  fireEvent.click(screen.getByText("Review manifest"));
  await screen.findByRole("list", { name: "Manifest problems" });
  // The signed check answering late must not clear the unsigned review's problem.
  await act(async () => { finishSigned({ errors: [] }); });
  expect(screen.getByRole("list", { name: "Manifest problems" })).toBeTruthy();
  expect(screen.queryByText("Install and grant permissions")).toBeNull();
});
/** A signed catalog update that added an event grant, with the version it replaced. */
const updated = () => ({
  ...plugin,
  manifest: { ...plugin.manifest, version: "0.2.0", permissions: ["k8s.listCustomResource", "k8s.listEvents"] },
  grants: ["k8s.listCustomResource", "k8s.listEvents"],
  revision: 4,
  settings: { team: "platform" },
  source: "catalog",
  installedAt: 1_700_000_000,
  signatureProof: { manifest: "{}", signature: [1] },
  history: [
    { manifest: { ...plugin.manifest, version: "0.1.0" }, grants: ["k8s.listCustomResource"], revision: 2, source: "local", installedAt: 1_690_000_000 },
  ],
});
async function openDetails(app: ReturnType<typeof updated>) {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 5, plugins: [app] } as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Details for GitOps" }));
  return screen.getByRole("region", { name: "GitOps details" });
}
it("inspects an installed app's source, grants and manifest, and exports or resets its settings", async () => {
  vi.mocked(saveTextFile).mockResolvedValue("/tmp/settings.json");
  const details = await openDetails(updated());
  expect(details.textContent).toContain("Signed by srelens");
  expect(details.textContent).toContain("from the Catalog");
  expect(details.textContent).toContain("revision 4");
  const grants = within(details).getByRole("list", { name: "Granted capabilities" });
  expect(within(grants).getByText("k8s.listEvents").closest("li")!.textContent).toContain("Read-only");
  const manifest = within(details).getByRole("textbox", { name: "GitOps manifest" });
  expect(manifest.getAttribute("contenteditable")).toBe("false");
  expect(manifest.textContent).toContain('"version": "0.2.0"');

  fireEvent.click(within(details).getByRole("button", { name: "Export settings" }));
  await waitFor(() =>
    expect(saveTextFile).toHaveBeenCalledWith(
      "org.test.gitops-settings.json",
      `${JSON.stringify({ team: "platform" }, null, 2)}\n`,
    ),
  );
  fireEvent.click(within(details).getByRole("button", { name: "Reset settings" }));
  expect(configureExtensions).not.toHaveBeenCalled();
  fireEvent.click(within(details).getByRole("button", { name: "Reset to defaults" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "settings", id: "org.test.gitops", settings: {} }),
  );
});
it("reviews the permissions of a rollback whose grants differ", async () => {
  const details = await openDetails(updated());
  const versions = within(details).getByRole("list", { name: "Previous versions" });
  fireEvent.click(within(versions).getByRole("button", { name: "Roll back to 0.1.0" }));
  const review = screen.getByRole("region", { name: "Review rollback" });
  expect(review.textContent).toContain("k8s.listCustomResource");
  expect(configureExtensions).not.toHaveBeenCalled();
  fireEvent.click(within(review).getByRole("button", { name: "Roll back and grant permissions" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "rollback", id: "org.test.gitops", revision: 2, grants: ["k8s.listCustomResource"],
    }),
  );
});
it("closes the reset confirmation with Escape without resetting", async () => {
  const details = await openDetails(updated());
  fireEvent.click(within(details).getByRole("button", { name: "Reset settings" }));
  const dialog = within(details).getByRole("alertdialog", { name: "Reset settings" });
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(within(details).queryByRole("alertdialog", { name: "Reset settings" })).toBeNull();
  expect(configureExtensions).not.toHaveBeenCalled();
});
it("limits an app to chosen clusters from its details", async () => {
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [{ name: "cluster/a", stableId: "a" }, { name: "cluster/b", stableId: "b" }],
  } as any);
  // Opening the picker makes the manifest editor measure text ranges, and jsdom has no
  // layout to measure. Stub them for this test only.
  const measuring = {
    getClientRects: Range.prototype.getClientRects,
    getBoundingClientRect: Range.prototype.getBoundingClientRect,
  };
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () =>
    ({ x: 0, y: 0, width: 0, height: 0, top: 0, right: 0, bottom: 0, left: 0, toJSON: () => ({}) }) as DOMRect;
  try {
    const details = await openDetails(updated());
    const clusters = within(details).getByRole("group", { name: "Clusters" });
    expect((within(clusters).getByLabelText("All clusters") as HTMLInputElement).checked).toBe(true);
    fireEvent.click(within(clusters).getByLabelText("Only these clusters"));
    await waitFor(() => expect(listContexts).toHaveBeenCalled());
    // Contexts come from the kubeconfig and can number in the hundreds, so they are searched.
    fireEvent.click(within(clusters).getByRole("combobox", { name: "Add a cluster" }));
    fireEvent.click(await screen.findByRole("option", { name: "cluster/b" }));
    expect(within(clusters).getByRole("button", { name: "Remove cluster/b" })).toBeTruthy();
    fireEvent.click(within(clusters).getByRole("button", { name: "Save clusters" }));
    await waitFor(() =>
      expect(configureExtensions).toHaveBeenCalledWith({ action: "clusters", id: "org.test.gitops", contexts: ["cluster/b"] }),
    );
  } finally {
    Object.assign(Range.prototype, measuring);
  }
});
it("allows every cluster again, and keeps listing a chosen cluster the kubeconfig no longer has", async () => {
  vi.mocked(listContexts).mockResolvedValue({ contexts: [{ name: "cluster/a", stableId: "a" }] } as any);
  const app = { ...updated(), contexts: ["cluster/a", "retired"] };
  const details = await openDetails(app);
  const clusters = within(details).getByRole("group", { name: "Clusters" });
  expect((within(clusters).getByLabelText("Only these clusters") as HTMLInputElement).checked).toBe(true);
  expect(within(clusters).getByRole("button", { name: "Remove cluster/a" })).toBeTruthy();
  fireEvent.click(within(clusters).getByRole("button", { name: "Remove retired" }));
  expect(within(clusters).queryByRole("button", { name: "Remove retired" })).toBeNull();
  fireEvent.click(within(clusters).getByLabelText("All clusters"));
  fireEvent.click(within(clusters).getByRole("button", { name: "Save clusters" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "clusters", id: "org.test.gitops", contexts: null }),
  );
});
it("says why the cluster list could not be loaded, and retries it", async () => {
  vi.mocked(listContexts)
    .mockResolvedValueOnce({ error: "kubeconfig unreadable" })
    .mockResolvedValue({ contexts: [{ name: "cluster/a", stableId: "a" }] } as any);
  const app = { ...updated(), contexts: ["cluster/b"] };
  const details = await openDetails(app);
  const clusters = within(details).getByRole("group", { name: "Clusters" });
  const alert = await within(clusters).findByRole("alert");
  expect(alert.textContent).toContain("kubeconfig unreadable");
  fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(within(clusters).queryByRole("alert")).toBeNull());
  expect(listContexts).toHaveBeenCalledTimes(2);
});
it("does not call a quarantined app's signature verified", async () => {
  const app = { ...updated(), enabled: false, quarantined: "App publisher signature is invalid" };
  const details = await openDetails(app);
  const source = within(details).getByText(/revision 4/);
  expect(source.textContent).toContain("Signature not verified");
  expect(source.textContent).not.toContain("Signed by srelens");
});
it("only confirms a rollback whose grants are unchanged", async () => {
  const app = updated();
  app.history[0].manifest = { ...app.history[0].manifest, permissions: app.grants };
  const details = await openDetails(app);
  fireEvent.click(within(details).getByRole("button", { name: "Roll back to 0.1.0" }));
  const review = screen.getByRole("region", { name: "Review rollback" });
  expect(within(review).queryByRole("button", { name: "Roll back and grant permissions" })).toBeNull();
  fireEvent.click(within(review).getByRole("button", { name: "Roll back" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "rollback", id: "org.test.gitops", revision: 2, grants: app.grants,
    }),
  );
});
it("persists settings, disable and remove through the backend", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [plugin],
  });
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  fireEvent.change(screen.getByLabelText("App settings (JSON object)"), {
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
  fireEvent.click(screen.getByRole("button", {name:"Remove app"}));
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
    await screen.findByText("No resources returned by this app."),
  ).toBeTruthy();
  expect(readExtension).toHaveBeenCalledWith(
    plugin.manifest.id,
    1,
    "list",
    "prod",
    "",
    true,
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

it("adds namespace detail views and links, and removes them when disabled", async () => {
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
  installed.manifest.contributions.detailLinks = [
    {
      id: "inspect",
      title: "Inspect apps",
      capability: "list",
      forKinds: ["/Namespace"],
    },
  ];
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
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
      true,
    ),
  );
  fireEvent.click(screen.getByText("App links"));
  fireEvent.click(screen.getByText("Inspect apps"));
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
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
it("registers cluster-pinned app routes with a real screen", async () => {
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

it("explains a missing app API and keeps the server error collapsed", async () => {
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
    screen.queryByText("No resources returned by this app."),
  ).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(
    await screen.findByText("No resources returned by this app."),
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

it("requires confirmation and permits cancelling removal of stored settings", async () => {
  vi.mocked(listExtensions).mockResolvedValue({plugins:[plugin]} as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", {name:"Remove"}));
  expect(configureExtensions).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", {name:"Cancel"}));
  expect(configureExtensions).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", {name:"Remove"}));
  fireEvent.click(screen.getByRole("button", {name:"Remove app"}));
  await waitFor(()=>expect(configureExtensions).toHaveBeenCalledWith({action:"remove",id:plugin.manifest.id}));
});
it("refreshes external lifecycle changes without unmounting enabled content", async () => {
  const { useExtensions } = await import("./Extensions");
  const Consumer = () => <>{useExtensions().data?.plugins.map(p => <span key={p.manifest.id}>Installed plugin</span>)}</>;
  vi.mocked(listExtensions).mockResolvedValue({plugins:[plugin]} as any);
  const {act}=await import("@testing-library/react");
  vi.useFakeTimers();
  let view: ReturnType<typeof render>;
  try {
    await act(async()=>{view=render(<Consumer />);});
    expect(screen.getByText("Installed plugin")).toBeTruthy();
    vi.mocked(listExtensions).mockResolvedValue({plugins:[]} as any);
    await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
    expect(screen.queryByText("Installed plugin")).toBeNull();
  } finally {view!.unmount();vi.useRealTimers();}
});
it("distinguishes filtered rows from an empty resource response", async () => {
  vi.mocked(readExtension).mockResolvedValue({items:[{name:"apps",namespace:"team",age:"1d",columns:[]}]});
  render(<ExtensionResults plugin={plugin} capability="list" context="test" search="missing" />);
  expect(await screen.findByText("No matching resources.")).toBeTruthy();
  expect(screen.queryByText("No resources returned by this app.")).toBeNull();
});

it("advances app resource ages without refreshing backend data", async () => {
  const {act}=await import("@testing-library/react");
  vi.useFakeTimers();
  const created="2026-09-13T12:00:00Z";
  vi.setSystemTime(new Date(created));
  vi.mocked(readExtension).mockResolvedValue({items:[{name:"apps",namespace:"team",age:"0s",created,columns:[]}]} as any);
  let view:ReturnType<typeof render>;
  try {
    await act(async()=>{view=render(<ExtensionResults plugin={plugin} capability="list" context="test" />);});
    expect(screen.getByText("0s")).toBeTruthy();
    act(()=>{vi.advanceTimersByTime(30000);});
    expect(screen.getByText("30s")).toBeTruthy();
    expect(readExtension).toHaveBeenCalledTimes(1);
  } finally {view!.unmount();vi.useRealTimers();}
});

it("shares one inventory poll and stops it after the last consumer unmounts", async () => {
  const {useExtensions}=await import("./Extensions");
  const {act}=await import("@testing-library/react");
  const Consumer=()=>{useExtensions();return null;};
  vi.useFakeTimers();
  let first:ReturnType<typeof render>,second:ReturnType<typeof render>;
  try {
    await act(async()=>{first=render(<Consumer/>);second=render(<Consumer/>);});
    expect(listExtensions).toHaveBeenCalledTimes(1);
    await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
    expect(listExtensions).toHaveBeenCalledTimes(2);
    first!.unmount();
    await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
    expect(listExtensions).toHaveBeenCalledTimes(3);
    second!.unmount();
    await act(async()=>{await vi.advanceTimersByTimeAsync(10000);});
    expect(listExtensions).toHaveBeenCalledTimes(3);
  } finally {first!?.unmount();second!?.unmount();vi.useRealTimers();}
});

it("queues lifecycle refreshes behind one pending poll and discards its stale result", async () => {
  const { useExtensions } = await import("./Extensions");
  const Consumer = () => <>{useExtensions().data?.plugins.map(p => <span key={p.manifest.id}>Installed plugin</span>)}</>;
  const {act}=await import("@testing-library/react");
  const {EXTENSIONS_CHANGED}=await import("@srelens/core");
  let finish:(value:any)=>void;
  vi.mocked(listExtensions).mockReturnValueOnce(new Promise(resolve=>{finish=resolve;}));
  vi.mocked(listExtensions).mockResolvedValue({plugins:[]} as any);
  render(<Consumer/>);
  fireEvent(window,new Event(EXTENSIONS_CHANGED));
  fireEvent(window,new Event(EXTENSIONS_CHANGED));
  expect(listExtensions).toHaveBeenCalledTimes(1);
  await act(async()=>{finish!({plugins:[plugin]});});
  expect(listExtensions).toHaveBeenCalledTimes(2);
  expect(screen.queryByText("Installed plugin")).toBeNull();
});

it.each([undefined, [1,2,3]])("installs catalog bytes and signature %j only after explicit review and grants", async (signature) => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 1, plugins: [] });
  const source = JSON.stringify(plugin.manifest);
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: source, signature });
  vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [{ id: plugin.manifest.id, name: "Catalog GitOps", description: "GitOps resources", repository: "https://github.com/example/gitops", license: "MIT", release: { version: "0.1.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: true } }] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] } as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  const install = await screen.findByText("Install and grant permissions");
  expect(validateExtension).toHaveBeenCalledWith(source, plugin.manifest.permissions, signature);
  expect(configureExtensions).not.toHaveBeenCalled();
  expect(readExtension).not.toHaveBeenCalled();
  fireEvent.click(install);
  await waitFor(() => expect(configureExtensions).toHaveBeenCalledWith({ action: "install", manifest: source, grants: plugin.manifest.permissions, ...(signature ? {signature} : {}) }));
});

it("separates installed apps from the catalog and collapses local installation by default", async () => {
  vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] } as any);
  render(<ExtensionManager />);
  expect((await screen.findByRole("tab", { name: "Apps" })).getAttribute("aria-selected")).toBe("true");
  expect(screen.getByLabelText("Local app manifest (JSON)").closest("details")?.open).toBe(false);
  expect(listExtensionCatalog).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("tab", { name: "Catalog" }));
  await waitFor(() => expect(listExtensionCatalog).toHaveBeenCalledWith(false));
  expect(screen.getByText("No apps installed.").closest("[hidden]")).not.toBeNull();
  fireEvent.click(screen.getByRole("tab", { name: "Apps" }));
  expect(screen.getByText("No apps installed.").closest("[hidden]")).toBeNull();
});

it("keeps app settings global without cluster selection or page launchers", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [plugin] });
  render(<ExtensionManager />);
  await screen.findByLabelText(`Enable ${plugin.manifest.name}`);
  expect(screen.queryByLabelText("App cluster")).toBeNull();
  expect(screen.queryByText("App pages")).toBeNull();
  expect(screen.queryByText("Choose a cluster")).toBeNull();
});

it("opens a clicked resource in its selected namespace and drops detail state on a cluster change", async () => {
  const {inspectExtensionResource}=await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue({items:[{name:"apps",namespace:"team",age:"1d",columns:[]}]});
  vi.mocked(inspectExtensionResource).mockResolvedValue({resource:{kind:"Kustomization",metadata:{name:"apps",namespace:"team",uid:"u",resourceVersion:"1"},status:{conditions:[{type:"Ready",status:"False",message:"Build failed"}]}},actions:[]});
  const app={...plugin,manifest:{...plugin.manifest,capabilities:[{...plugin.manifest.capabilities[0],target:"k8s.listCustomResource"}]}} as any;
  const view=render(<ExtensionResults plugin={app} capability="list" context="cluster/a"/>);
  fireEvent.click(await screen.findByRole("button",{name:"apps"}));
  expect(await screen.findByText("Build failed")).toBeTruthy();
  expect(screen.getByRole("button",{name:"apps"})).toBeTruthy();
  expect(screen.getByRole("region",{name:"apps"})).toBeTruthy();
  expect(inspectExtensionResource).toHaveBeenCalledWith({id:plugin.manifest.id,revision:plugin.revision,capability:"list",context:"cluster/a",namespace:"team",name:"apps"});
  view.rerender(<ExtensionResults plugin={app} capability="list" context="cluster/b"/>);
  expect(screen.queryByText("Build failed")).toBeNull();
});

it("uses CRD printer columns instead of app-defined generic columns",async()=>{
 vi.mocked(readExtension).mockResolvedValue({printerColumns:[{name:"Source",jsonPath:".spec.sourceRef.name",type:"string"}],items:[{name:"apps",namespace:"team",age:"1d",columns:["platform-config"]}]});
 render(<ExtensionResults plugin={plugin} capability="list" context="prod"/>);
 expect(await screen.findByRole("columnheader",{name:"Source"})).toBeTruthy();
 expect(screen.queryByRole("columnheader",{name:"Ready"})).toBeNull();
 expect(screen.getByText("platform-config")).toBeTruthy();
});
it("reports CRD discovery failure while retaining the fallback resource columns",async()=>{
 vi.mocked(readExtension).mockResolvedValue({printerColumns:[{name:"Ready",jsonPath:".status.ready"}],columnsError:"Forbidden",items:[{name:"apps",namespace:"team",age:"1d",columns:["True"]}]});
 render(<ExtensionResults plugin={plugin} capability="list" context="prod"/>);
 expect(await screen.findByText(/Could not load CRD columns: Forbidden/)).toBeTruthy();
 expect(await screen.findByRole("columnheader",{name:"Ready"})).toBeTruthy();
});
