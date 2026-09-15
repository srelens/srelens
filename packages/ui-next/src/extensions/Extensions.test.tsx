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
}));
import {
  listExtensionCatalog,
  reviewCatalogExtension,
  listExtensions,
  configureExtensions,
  validateExtension,
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
    nextRevision: 1,
    plugins: [],
  } as any);
  vi.mocked(configureExtensions).mockResolvedValue({} as any);
  vi.mocked(validateExtension).mockResolvedValue({ errors: [] });
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
  fireEvent.click(screen.getByText("App actions"));
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
