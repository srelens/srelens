import { act, render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
const host = vi.hoisted(() => ({ tauri: true }));
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  isTauri: () => host.tauri,
  listExtensionCatalog: vi.fn(),
  reviewCatalogExtension: vi.fn(),
  reviewExtensionPackage: vi.fn(),
  listExtensions: vi.fn(),
  configureExtensions: vi.fn(),
  validateExtension: vi.fn(),
  readExtension: vi.fn(),
  resolveExtensionColumns: vi.fn(),
  inspectExtensionResource: vi.fn(),
  actOnExtensionResource: vi.fn(),
  saveTextFile: vi.fn(),
  listContexts: vi.fn(),
  setExtensionSecret: vi.fn(),
  clearExtensionSecret: vi.fn(),
  onExtensionInventoryChanged: vi.fn(),
}));
import {
  listExtensionCatalog,
  reviewCatalogExtension,
  reviewExtensionPackage,
  listExtensions,
  configureExtensions,
  validateExtension,
  readExtension,
  resolveExtensionColumns,
  saveTextFile,
  listContexts,
  setExtensionSecret,
  clearExtensionSecret,
  onExtensionInventoryChanged,
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
// jsdom's Blob has no arrayBuffer(), which every WebView the app runs in has; a package
// file chosen in Settings is read with it (#562).
if (!("arrayBuffer" in Blob.prototype)) {
  Object.defineProperty(Blob.prototype, "arrayBuffer", {
    value(this: Blob) {
      return new Promise<ArrayBuffer>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result as ArrayBuffer);
        reader.onerror = () => reject(reader.error);
        reader.readAsArrayBuffer(this);
      });
    },
  });
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
  vi.mocked(validateExtension).mockResolvedValue({ errors: [], permissionDiff: { previousRevision: null, added: ["Grant k8s.listCustomResource"], removed: [], unchanged: [] } });
  vi.mocked(listContexts).mockResolvedValue({ contexts: [] });
  vi.mocked(resolveExtensionColumns).mockResolvedValue({ columns: [], cells: [] });
  vi.mocked(onExtensionInventoryChanged).mockResolvedValue(() => {});
});

it("says when the app list cannot hear the host's announcements and is polling instead (#566)", async () => {
  vi.mocked(onExtensionInventoryChanged).mockRejectedValue(new Error("no event channel"));
  render(<ExtensionManager />);
  expect(await screen.findByText(/Live updates to this list are unavailable \(no event channel\)/)).toBeTruthy();
});

it("shows a declared native table column on the app's own resource page", async () => {
  const column = { id:"critical", title:"Critical CVEs", forKinds:["argoproj.io/Application"],
    source:{ jsonPath:".critical" }, format:"number" as const, sortable:true };
  const app = { ...plugin, manifest: { ...plugin.manifest,
    capabilities:[{ name:"list", target:"k8s.listCustomResource", arguments:{ group:"argoproj.io", kind:"Application", namespaced:true } }],
    contributions:{ ...plugin.manifest.contributions, tableColumns:[column] } } };
  vi.mocked(readExtension).mockResolvedValue({ items:[{name:"apps",namespace:"team",age:"1d",columns:[]}], printerColumns:[] });
  vi.mocked(resolveExtensionColumns).mockResolvedValue({ columns:[column], cells:[{uid:null,name:"apps",namespace:"team",values:{critical:"4"}}] });
  render(<ExtensionResults plugin={app} capability="list" context="prod" namespace="team" />);
  expect(await screen.findByRole("columnheader", {name:"Critical CVEs"})).toBeTruthy();
  expect(await screen.findByText("4")).toBeTruthy();
  expect(resolveExtensionColumns).toHaveBeenCalledTimes(1);
});
it("shows the host-resolved status as a word in its own column, searchable, when the kind has a resolver", async () => {
  const app = { ...plugin, manifest: { ...plugin.manifest,
    capabilities:[{ name:"list", target:"k8s.listCustomResource", arguments:{ group:"argoproj.io", kind:"Application", namespaced:true } }],
    contributions:{ ...plugin.manifest.contributions, statusResolvers:[{ forKinds:["argoproj.io/Application"],
      rules:[{ when:[], status:"unknown", label:"Unknown" }] }] } } };
  vi.mocked(readExtension).mockResolvedValue({ items:[
    { name:"guestbook", namespace:"team", age:"1d", columns:[], status:{ status:"healthy", label:"Healthy" } },
    { name:"billing", namespace:"team", age:"1d", columns:[], status:{ status:"warning", label:"Out of sync", reason:"abc123" } },
    { name:"legacy", namespace:"team", age:"1d", columns:[] },
  ], printerColumns:[] });
  const view = render(<ExtensionResults plugin={app} capability="list" context="prod" namespace="team" />);
  expect(await screen.findByRole("columnheader", { name:"Status" })).toBeTruthy();
  const rows = within(screen.getByRole("table")).getAllByRole("row").slice(1);
  expect(within(rows[0]).getByText("Healthy")).toBeTruthy();
  expect(within(rows[1]).getByText("Out of sync")).toBeTruthy();
  expect(within(rows[1]).getByText("abc123")).toBeTruthy();
  // A row the host returned no status for says so, rather than borrowing a word.
  expect(within(rows[2]).getByText("—")).toBeTruthy();
  view.rerender(<ExtensionResults plugin={app} capability="list" context="prod" namespace="team" search="out of sync" />);
  expect(screen.getByText("billing")).toBeTruthy();
  expect(screen.queryByText("guestbook")).toBeNull();
});
it("draws no status column for a kind without a resolver", async () => {
  // A real custom-resource reader, and a resolver — for another kind. The
  // only thing keeping the column away is that this kind has no resolver.
  const app = { ...plugin, manifest: { ...plugin.manifest,
    capabilities:[{ name:"list", target:"k8s.listCustomResource", arguments:{ group:"argoproj.io", kind:"Application", namespaced:true } }],
    contributions:{ ...plugin.manifest.contributions, statusResolvers:[{ forKinds:["argoproj.io/AppProject"],
      rules:[{ when:[], status:"unknown", label:"Unknown" }] }] } } };
  vi.mocked(readExtension).mockResolvedValue({ items:[{ name:"apps", namespace:"team", age:"1d", columns:["True"] }], printerColumns:[{ name:"Ready", jsonPath:".r" }] });
  render(<ExtensionResults plugin={app} capability="list" context="prod" namespace="team" />);
  expect(await screen.findByText("apps")).toBeTruthy();
  expect(screen.queryByRole("columnheader", { name:"Status" })).toBeNull();
});
it("sorts and searches opted-in app column values", async () => {
  const column = { id:"score", title:"Score", forKinds:["argoproj.io/Application"],
    source:{jsonPath:".score"}, format:"number" as const, sortable:true, filterable:true };
  const app = { ...plugin, manifest: { ...plugin.manifest,
    capabilities:[{name:"list", target:"k8s.listCustomResource", arguments:{group:"argoproj.io",kind:"Application",namespaced:true}}],
    contributions:{...plugin.manifest.contributions, tableColumns:[column]} } };
  vi.mocked(readExtension).mockResolvedValue({ items:[
    {name:"alpha",namespace:"team",age:"1d",columns:[]},
    {name:"beta",namespace:"team",age:"1d",columns:[]},
  ], printerColumns:[] });
  vi.mocked(resolveExtensionColumns).mockResolvedValue({ columns:[column], cells:[
    {uid:null,name:"alpha",namespace:"team",values:{score:"12"}},
    {uid:null,name:"beta",namespace:"team",values:{score:"4"}},
  ] });
  const view = render(<ExtensionResults plugin={app} capability="list" context="prod" namespace="team" />);
  expect(await screen.findByText("12")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", {name:"Sort by Score"}));
  expect(within(screen.getByRole("table")).getAllByRole("row").slice(1).map((row) => row.textContent)).toEqual([
    expect.stringContaining("beta"), expect.stringContaining("alpha"),
  ]);
  view.rerender(<ExtensionResults plugin={app} capability="list" context="prod" namespace="team" search="12" />);
  expect(screen.getByText("alpha")).toBeTruthy();
  expect(screen.queryByText("beta")).toBeNull();
});
it("shows update access additions and removals before unchanged access and installs the reviewed revision", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 8, plugins: [{ ...plugin, revision: 7 }] } as any);
  vi.mocked(validateExtension).mockResolvedValue({
    errors: [],
    permissionDiff: {
      previousRevision: 7,
      added: ["Action k8s.annotate on Deployment"],
      removed: ["Read k8s.listEvents on Pod"],
      unchanged: ["Grant k8s.listCustomResource"],
    },
  });
  render(<ExtensionManager />);
  const source = JSON.stringify({ ...plugin.manifest, version: "0.2.0" });
  fireEvent.change(await screen.findByLabelText("Local app manifest (JSON)"), { target: { value: source } });
  fireEvent.click(screen.getByText("Review manifest"));
  const review = await screen.findByLabelText("Review app permissions");
  expect(within(review).getByText(/Action k8s.annotate on Deployment/)).toBeTruthy();
  expect(within(review).getByText(/Read k8s.listEvents on Pod/)).toBeTruthy();
  const unchanged = within(review).getByText(/1 unchanged/).closest("details")!;
  expect(unchanged.open).toBe(false);
  const allBindings = within(review).getByText("Complete incoming bindings").closest("details")!;
  expect(allBindings.open).toBe(false);
  fireEvent.click(within(review).getByRole("button", { name: /Update and grant permissions/ }));
  await waitFor(() => expect(configureExtensions).toHaveBeenCalledWith({ action: "install", manifest: source, grants: plugin.manifest.permissions, reviewedRevision: 7 }));
});
it("keeps a missing access comparison distinct from a new installation", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 8, plugins: [{ ...plugin, revision: 7 }] } as any);
  vi.mocked(validateExtension).mockResolvedValue({ errors: [] });
  render(<ExtensionManager />);
  fireEvent.change(await screen.findByLabelText("Local app manifest (JSON)"), {
    target: { value: JSON.stringify({ ...plugin.manifest, version: "0.2.0" }) },
  });
  fireEvent.click(screen.getByText("Review manifest"));
  const review = await screen.findByLabelText("Review app permissions");
  expect(await within(review).findByText("Could not review access changes")).toBeTruthy();
  expect(within(review).queryByText(/requests a new installation/)).toBeNull();
  expect(within(review).queryByText("Complete incoming bindings")).toBeNull();
  expect(within(review).queryByRole("button", { name: /grant permissions/ })).toBeNull();
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
  const toggle = screen.getByLabelText("Enable org.test.gitops") as HTMLInputElement;
  expect(toggle.checked).toBe(false);
  expect(toggle.disabled).toBe(true);
  expect(screen.getByText("Remove")).toBeTruthy();
});
it("sends an unsigned app under a reserved ID to the Catalog for the signed release", async () => {
  // Stored before org.srelens. was reserved: the host quarantines it on load (#602).
  const reason = "App ID org.srelens.gitops is reserved for signed srelens releases";
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [
      {
        ...plugin,
        manifest: { ...plugin.manifest, id: "org.srelens.gitops" },
        source: "local",
        enabled: false,
        quarantined: reason,
      },
    ],
  } as any);
  render(<ExtensionManager />);
  expect((await screen.findByText(new RegExp(reason))).textContent).toBe(
    `Disabled: ${reason}. Remove it or reinstall it from the Catalog.`,
  );
  expect(screen.getByText(/Unsigned local/)).toBeTruthy();
  const toggle = screen.getByLabelText("Enable org.srelens.gitops") as HTMLInputElement;
  expect(toggle.checked).toBe(false);
  expect(toggle.disabled).toBe(true);
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
  expect(await screen.findByText("Install a local manifest or package")).toBeTruthy();
  expect(screen.getByLabelText("Local app package (.srelens-extension)")).toBeTruthy();
  expect(screen.queryByLabelText("App developer mode")).toBeNull();
  expect(configureExtensions).not.toHaveBeenCalled();
});

it("offers no package file on the web, whose host keeps no app files (#562)", async () => {
  host.tauri = false;
  try {
    render(<ExtensionManager />);
    expect(await screen.findByText("Install a local manifest")).toBeTruthy();
    expect(screen.queryByLabelText("Local app package (.srelens-extension)")).toBeNull();
  } finally {
    host.tauri = true;
  }
});

/** What the host returns for a verified package (#562). */
const packaged = {
  sha256: "f".repeat(64),
  digests: '{"format":"srelens-extension-package"}',
  files: [{ path: "extension.json", size: 700 }, { path: "icons/icon.svg", size: 215 }],
  icon: `data:image/svg+xml;base64,${btoa("<svg/>")}`,
};

it("reviews a package file as a whole and installs the bytes that were reviewed (#562)", async () => {
  const source = JSON.stringify(plugin.manifest);
  vi.mocked(reviewExtensionPackage).mockResolvedValue({ manifest: source, signature: [1, 2, 3], package: packaged });
  render(<ExtensionManager />);
  const input = await screen.findByLabelText("Local app package (.srelens-extension)");
  const content = new Uint8Array([0x1f, 0x8b, 0x08, 0x00, 0xff]);
  fireEvent.change(input, { target: { files: [new File([content], "gitops.srelens-extension")] } });
  const install = await screen.findByText("Install and grant permissions");
  expect(reviewExtensionPackage).toHaveBeenCalledWith(content);
  // The signature is over the digest list, so the check gets the list with it.
  expect(validateExtension).toHaveBeenCalledWith(source, plugin.manifest.permissions, [1, 2, 3], packaged.digests, undefined);
  const review = screen.getByRole("region", { name: "Review app permissions" });
  expect(within(review).getByText(/Package: 2 files, 915 B/)).toBeTruthy();
  expect(within(review).getByText("icons/icon.svg")).toBeTruthy();
  expect(review.querySelector("[data-extension-logo]")?.getAttribute("data-extension-logo")).toBe("package");
  fireEvent.click(install);
  await waitFor(() => expect(configureExtensions).toHaveBeenCalledWith({
    action: "installPackage", package: "H4sIAP8=", grants: plugin.manifest.permissions,
  }));
});

it("says why a package file was refused, and refuses one over the limit before reading it (#562)", async () => {
  vi.mocked(reviewExtensionPackage).mockRejectedValue(new Error("README.md does not match its digest"));
  render(<ExtensionManager />);
  const input = await screen.findByLabelText("Local app package (.srelens-extension)");
  fireEvent.change(input, { target: { files: [new File([new Uint8Array([1])], "tampered.srelens-extension")] } });
  expect((await screen.findByRole("alert")).textContent).toBe(
    "Could not review tampered.srelens-extension: README.md does not match its digest",
  );
  expect(screen.queryByRole("region", { name: "Review app permissions" })).toBeNull();
  const large = new File([new Uint8Array([1])], "large.srelens-extension");
  Object.defineProperty(large, "size", { value: 17 * 1024 * 1024 });
  fireEvent.change(input, { target: { files: [large] } });
  expect((await screen.findByRole("alert")).textContent).toBe(
    "large.srelens-extension is 17.0 MiB; a package may be at most 16.0 MiB.",
  );
  expect(reviewExtensionPackage).toHaveBeenCalledTimes(1);
  expect(configureExtensions).not.toHaveBeenCalled();
});

/** A promise the test settles when it chooses. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => { resolve = settle; });
  return { promise, resolve };
}
/** A catalog listing one release of the test app. */
const oneRelease = () => ({ catalog: { extensions: [{ id: plugin.manifest.id, name: "Catalog GitOps", description: "GitOps resources",
  repository: "https://github.com/example/gitops", license: "MIT",
  release: { version: "0.1.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: false } }] },
  fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] }) as any;
const named = (name: string) => JSON.stringify({ ...plugin.manifest, name });

it("drops a package review that finishes loading after a catalog review started later (#562)", async () => {
  const load = deferred<Awaited<ReturnType<typeof reviewExtensionPackage>>>();
  vi.mocked(reviewExtensionPackage).mockReturnValue(load.promise);
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: named("From the catalog") });
  vi.mocked(listExtensionCatalog).mockResolvedValue(oneRelease());
  render(<ExtensionManager />);
  fireEvent.change(await screen.findByLabelText("Local app package (.srelens-extension)"),
    { target: { files: [new File([new Uint8Array([1])], "slow.srelens-extension")] } });
  fireEvent.click(screen.getByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  expect(await screen.findByText("From the catalog")).toBeTruthy();
  await act(async () => { load.resolve({ manifest: named("From the file"), package: packaged }); });
  expect(screen.queryByText("From the file")).toBeNull();
  expect(screen.getByText("From the catalog")).toBeTruthy();
  expect(validateExtension).toHaveBeenCalledTimes(1);
});

it("drops a catalog review that finishes downloading after a package review started later (#562)", async () => {
  const download = deferred<Awaited<ReturnType<typeof reviewCatalogExtension>>>();
  vi.mocked(reviewCatalogExtension).mockReturnValue(download.promise);
  vi.mocked(reviewExtensionPackage).mockResolvedValue({ manifest: named("From the file"), package: packaged });
  vi.mocked(listExtensionCatalog).mockResolvedValue(oneRelease());
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  await waitFor(() => expect(reviewCatalogExtension).toHaveBeenCalledTimes(1));
  fireEvent.change(screen.getByLabelText("Local app package (.srelens-extension)"),
    { target: { files: [new File([new Uint8Array([1])], "later.srelens-extension")] } });
  expect(await screen.findByText("From the file")).toBeTruthy();
  await act(async () => { download.resolve({ manifest: named("From the catalog") }); });
  expect(screen.queryByText("From the catalog")).toBeNull();
  expect(screen.getByText("From the file")).toBeTruthy();
  expect(validateExtension).toHaveBeenCalledTimes(1);
});

it("shows an installed app's package logo, and its initials without one (#562)", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 3, plugins: [
    { ...plugin, icon: packaged.icon },
    { ...plugin, manifest: { ...plugin.manifest, id: "org.test.plain", name: "Plain" } },
  ] } as any);
  render(<ExtensionManager />);
  await screen.findByText("Plain");
  const logos = [...document.querySelectorAll(".extension-installed [data-extension-logo]")]
    .map((logo) => logo.getAttribute("data-extension-logo"));
  expect(logos).toEqual(["package", "initials"]);
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
  vi.mocked(validateExtension).mockResolvedValue({ errors: [], permissionDiff: { previousRevision: null, added: ["Grant k8s.listCustomResource"], removed: [], unchanged: [] } });
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
  expect(validateExtension).toHaveBeenCalledWith(source, plugin.manifest.permissions, undefined, undefined, undefined);
  expect(screen.queryByText("Install and grant permissions")).toBeNull();
  expect(configureExtensions).not.toHaveBeenCalled();
});
it("does not show a pasted manifest's name until the host has accepted it", async () => {
  // A right-to-left override reorders the review line it is rendered into, so the name
  // is untrusted text until the host, which refuses such names, has checked it.
  const spoofed = {
    ...plugin.manifest,
    name: "‮Argo CD",
    permissions: ["k8s.listCustomResource‮"],
  };
  const source = JSON.stringify(spoofed);
  let finish!: (report: { errors: { code: string; path: string; message: string }[] }) => void;
  vi.mocked(validateExtension).mockImplementationOnce(
    () => new Promise((resolve) => { finish = resolve; }),
  );
  render(<ExtensionManager />);
  fireEvent.change(
    await screen.findByLabelText("Local app manifest (JSON)"),
    { target: { value: source } },
  );
  fireEvent.click(screen.getByText("Review manifest"));
  const review = await screen.findByLabelText("Review app permissions");
  // While the check is still running.
  expect(review.textContent).not.toContain("‮");
  expect(review.textContent).toContain("This manifest");
  await act(async () => {
    finish({
      errors: [
        { code: "EXTENSION_INVALID_VALUE", path: "name", message: "Must be 1–120 characters" },
      ],
    });
  });
  // And once it comes back refusing the name.
  await screen.findByRole("list", { name: "Manifest problems" });
  expect(review.textContent).not.toContain("‮");
  expect(review.textContent).toContain("This manifest");
});
it("shows the name of a manifest the host accepted", async () => {
  vi.mocked(validateExtension).mockResolvedValue({ errors: [], permissionDiff: { previousRevision: null, added: ["Grant k8s.listCustomResource"], removed: [], unchanged: [] } });
  render(<ExtensionManager />);
  fireEvent.change(
    await screen.findByLabelText("Local app manifest (JSON)"),
    { target: { value: JSON.stringify(plugin.manifest) } },
  );
  fireEvent.click(screen.getByText("Review manifest"));
  await screen.findByText("Install and grant permissions");
  const review = screen.getByLabelText("Review app permissions");
  expect(within(review).getByText("GitOps")).toBeTruthy();
  expect(review.textContent).toContain("k8s.listCustomResource");
});
it("shows a quarantined app by ID, not by a name this host no longer accepts", async () => {
  // An app installed before the rule keeps its stored name in the inventory. It is
  // quarantined, and its name is not drawn.
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [
      {
        ...plugin,
        enabled: false,
        quarantined: "name: Must be 1–120 characters (EXTENSION_INVALID_VALUE)",
        grants: ["k8s.listCustomResource"],
        source: "local",
        installedAt: 1,
        history: [],
        // A tag character (U+E0001) is above U+FFFF and must escape as a surrogate pair.
        manifest: { ...plugin.manifest, name: "‮Argo CD\u{E0001}" },
      },
    ],
  } as any);
  render(<ExtensionManager />);
  await screen.findByText(/Disabled:/);
  expect(document.body.textContent).not.toContain("‮");
  // Its stored manifest is shown with the character written as an escape, not drawn.
  fireEvent.click(screen.getByLabelText(`Details for ${plugin.manifest.id}`));
  const details = await screen.findByRole("region", { name: `${plugin.manifest.id} details` });
  const manifest = within(details).getByRole("textbox", { name: `${plugin.manifest.id} manifest` });
  await waitFor(() => expect(manifest.textContent).toContain(String.raw`\u202e`));
  expect(manifest.textContent).toContain(String.raw`\udb40\udc01`);
  expect(manifest.textContent).not.toContain(String.raw`\ue0001`);
  expect(document.body.textContent).not.toContain("‮");
  // And it can be taken away in one click: a stored manifest is read-only
  // text a reader opens in order to keep it. (#656 review)
  expect(within(details).getByRole("button", { name: "Copy" })).toBeTruthy();
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
  expect(validateExtension).toHaveBeenLastCalledWith(source, plugin.manifest.permissions, undefined, undefined, undefined);
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
  signedBy: { id: "example", name: "Example Labs" },
  history: [
    { manifest: { ...plugin.manifest, version: "0.1.0" }, grants: ["k8s.listCustomResource"], revision: 2, source: "local", installedAt: 1_690_000_000 },
  ],
});
async function openDetails(app: ReturnType<typeof updated>) {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 5, plugins: [app] } as any);
  render(<ExtensionManager />);
  // A quarantined app is shown by its ID: its stored name is one the host no longer accepts.
  const label = app.quarantined ? app.manifest.id : "GitOps";
  fireEvent.click(await screen.findByRole("button", { name: `Details for ${label}` }));
  return screen.getByRole("region", { name: `${label} details` });
}
it("inspects an installed app's source, grants and manifest, and exports or resets its settings", async () => {
  vi.mocked(saveTextFile).mockResolvedValue("/tmp/settings.json");
  const details = await openDetails(updated());
  // The publisher the host verified (#559), not one assumed from the proof.
  expect(details.textContent).toContain("Signed by Example Labs");
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
it("resets settings to their defaults, keeps a required one, which has none, and deletes the app's secrets", async () => {
  vi.mocked(clearExtensionSecret).mockResolvedValue({ set: false });
  const app = {
    ...updated(),
    manifest: { ...updated().manifest, settings: [
      { id: "url", type: "url", title: "URL", required: true },
      { id: "team", type: "string", title: "Team", default: "ops" },
      { id: "token", type: "secret-reference", title: "Token" },
    ] },
    settings: { url: "https://prom", team: "platform", token: { secretRef: "org.test.gitops/token" } },
  };
  const details = await openDetails(app as ReturnType<typeof updated>);
  expect(details.textContent).toContain("A secret is never saved in settings, so an export never holds one.");
  fireEvent.click(within(details).getByRole("button", { name: "Reset settings" }));
  // Says what reset does: keeps required values, and deletes the secrets
  // (#543): a reset that left a token in the keychain would not be one.
  const confirm = within(details).getByRole("alertdialog", { name: "Reset settings" });
  expect(confirm.textContent).toContain("except the required ones, which have no default");
  expect(confirm.textContent).toContain("its secrets are deleted from srelens's secrets vault");
  expect(confirm.textContent).not.toContain("Secrets stay set");
  fireEvent.click(within(details).getByRole("button", { name: "Reset to defaults" }));
  await waitFor(() => expect(clearExtensionSecret).toHaveBeenCalledWith("org.test.gitops"));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "settings", id: "org.test.gitops", settings: { url: "https://prom" } }),
  );
});
/** The web host keeps no app secrets (#522), so a reset there has none to delete and names no vault. */
it("resets an app with a secret setting on the web without asking the host to delete a secret", async () => {
  host.tauri = false;
  try {
    const app = {
      ...updated(),
      manifest: { ...updated().manifest, settings: [
        { id: "team", type: "string", title: "Team", default: "ops" },
        { id: "token", type: "secret-reference", title: "Token" },
      ] },
      settings: { team: "platform" },
    };
    const details = await openDetails(app as ReturnType<typeof updated>);
    fireEvent.click(within(details).getByRole("button", { name: "Reset settings" }));
    const confirm = within(details).getByRole("alertdialog", { name: "Reset settings" });
    expect(confirm.textContent).not.toContain("secrets vault");
    fireEvent.click(within(details).getByRole("button", { name: "Reset to defaults" }));
    await waitFor(() =>
      expect(configureExtensions).toHaveBeenCalledWith({ action: "settings", id: "org.test.gitops", settings: {} }),
    );
    expect(clearExtensionSecret).not.toHaveBeenCalled();
  } finally {
    host.tauri = true;
  }
});
it("keeps a secret through the host's store, never through settings, and shows why it cannot", async () => {
  const secretApp = (settings: Record<string, unknown>) => ({
    ...plugin, grants: ["k8s.listCustomResource", "extension.secretStore"],
    manifest: { ...plugin.manifest, settings: [{ id: "token", type: "secret-reference", title: "API token" }] },
    settings,
  });
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1, nextRevision: 2, plugins: [secretApp({})], secretStore: { available: true },
  } as any);
  vi.mocked(setExtensionSecret).mockResolvedValue({ set: true });
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings for GitOps" }));
  const form = screen.getByRole("form", { name: "GitOps settings" });
  const field = within(form).getByRole("group", { name: "API token" });
  // After the save the host lists the reference, never the value.
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1, nextRevision: 2, plugins: [secretApp({ token: { secretRef: "org.test.gitops/token" } })],
    secretStore: { available: true },
  } as any);
  fireEvent.change(within(field).getByLabelText("API token"), { target: { value: "typed-token" } });
  fireEvent.click(within(field).getByRole("button", { name: "Save secret" }));
  await waitFor(() => expect(setExtensionSecret).toHaveBeenCalledWith("org.test.gitops", "token", "typed-token"));
  await waitFor(() => expect(listExtensions).toHaveBeenCalledTimes(2));
  const again = within(await screen.findByRole("form", { name: "GitOps settings" })).getByRole("group", { name: "API token" });
  await waitFor(() => expect(within(again).getByRole("button", { name: "Replace secret" })).toBeTruthy());
  expect(configureExtensions).not.toHaveBeenCalled();
  expect(document.body.textContent).not.toContain("typed-token");
});
it("tells a person the store is unavailable, in the host's words", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1, nextRevision: 2,
    plugins: [{ ...plugin, grants: ["extension.secretStore"],
      manifest: { ...plugin.manifest, settings: [{ id: "token", type: "secret-reference", title: "API token" }] } }],
    secretStore: { available: false, reason: "This system has no usable keychain, so srelens cannot protect an app's secret" },
  } as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings for GitOps" }));
  const field = within(screen.getByRole("form", { name: "GitOps settings" })).getByRole("group", { name: "API token" });
  expect(field.textContent).toContain("no usable keychain");
  expect((within(field).getByLabelText("API token") as HTMLInputElement).disabled).toBe(true);
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
/** A metrics app reaching its Prometheus through network.http (#568). */
const networkApp = (allowLoopbackHttp?: boolean) => ({
  ...updated(),
  manifest: {
    ...updated().manifest,
    permissions: [{ capability: "network.http", hosts: ["${settings.prometheusUrl}", "api.github.com"] }],
    settings: [{ id: "prometheusUrl", type: "url", title: "Prometheus URL", required: true }],
  },
  grants: ["network.http"],
  ...(allowLoopbackHttp === undefined ? {} : { allowLoopbackHttp }),
});
it("lets a person allow an app's network.http requests plain HTTP to this computer (#568)", async () => {
  const details = await openDetails(networkApp() as unknown as ReturnType<typeof updated>);
  const network = within(details).getByRole("group", { name: "Network" });
  expect(network.textContent).toContain("api.github.com");
  expect(network.textContent).toContain("The host of the URL saved in Prometheus URL");
  // network.http is the broker's alone, so the catalog does not list it; that is not
  // "not provided".
  const grants = within(details).getByRole("list", { name: "Granted capabilities" });
  const grant = within(grants).getByText("network.http").closest("li")!.textContent;
  expect(grant).not.toContain("Not provided by this host");
  expect(grant).toContain("GET requests to this app's hosts only");
  const box = within(network).getByRole("checkbox", { name: "Allow plain HTTP to this computer (loopback)" });
  expect((box as HTMLInputElement).checked).toBe(false);
  fireEvent.click(box);
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "loopbackHttp", id: "org.test.gitops", allowLoopbackHttp: true }),
  );
});
it("shows plain HTTP to this computer as allowed, and turns it off", async () => {
  const details = await openDetails(networkApp(true) as unknown as ReturnType<typeof updated>);
  const box = within(details).getByRole("checkbox", { name: "Allow plain HTTP to this computer (loopback)" });
  expect((box as HTMLInputElement).checked).toBe(true);
  fireEvent.click(box);
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "loopbackHttp", id: "org.test.gitops", allowLoopbackHttp: false }),
  );
});
it("offers no network switch to an app that makes no requests", async () => {
  const details = await openDetails(updated());
  expect(within(details).queryByRole("group", { name: "Network" })).toBeNull();
});
it("reviews a rollback that reaches other hosts under the same grant", async () => {
  const app = {
    ...networkApp(),
    history: [
      {
        manifest: { ...networkApp().manifest, version: "0.1.0", permissions: [{ capability: "network.http", hosts: ["evil.example"] }] },
        grants: ["network.http"], revision: 2, source: "local", installedAt: 1_690_000_000,
      },
    ],
  };
  const details = await openDetails(app as unknown as ReturnType<typeof updated>);
  fireEvent.click(within(details).getByRole("button", { name: "Roll back to 0.1.0" }));
  const review = screen.getByRole("region", { name: "Review rollback" });
  expect(review.textContent).toContain("It requests: network.http.");
  expect(review.textContent).toContain("It reaches: evil.example.");
  expect(review.textContent).not.toContain("[object Object]");
  fireEvent.click(within(review).getByRole("button", { name: "Roll back and grant permissions" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "rollback", id: "org.test.gitops", revision: 2, grants: ["network.http"] }),
  );
});
it("reviews a rollback whose network.http request sends something else under the same grants and hosts", async () => {
  const request = (args: Record<string, unknown>) => ({
    name: "up", title: "Targets up", target: "network.http", inputs: [],
    arguments: { url: "${settings.prometheusUrl}", path: "/api/v1/query", ...args },
  });
  const current = networkApp();
  const app = {
    ...current,
    manifest: { ...current.manifest, capabilities: [request({})] },
    history: [
      {
        manifest: {
          ...current.manifest,
          version: "0.1.0",
          capabilities: [request({ secretHeaders: { Authorization: { secret: "token", prefix: "Bearer " } } })],
        },
        grants: ["network.http"], revision: 2, source: "local", installedAt: 1_690_000_000,
      },
    ],
  };
  const details = await openDetails(app as unknown as ReturnType<typeof updated>);
  fireEvent.click(within(details).getByRole("button", { name: "Roll back to 0.1.0" }));
  const review = screen.getByRole("region", { name: "Review rollback" });
  expect(review.textContent).toContain("Its network requests differ from this version's");
  // What the restored version would send, before anything is confirmed.
  const up = within(review).getByRole("listitem", { name: "Binding up" });
  expect(up.textContent).toContain("sends secret token as the Authorization header");
  expect(within(review).getByRole("button", { name: "Roll back and grant permissions" })).toBeTruthy();
});
it("reviews a rollback whose host setting has another default under the same host entries", async () => {
  // No saved value, so each version's default is where its requests go.
  const withDefault = (url: string) => [{ id: "prometheusUrl", type: "url", title: "Prometheus URL", default: url }];
  const current = networkApp();
  const app = {
    ...current,
    settings: {},
    manifest: { ...current.manifest, settings: withDefault("https://prometheus.example.com") },
    history: [
      {
        manifest: { ...current.manifest, version: "0.1.0", settings: withDefault("https://prometheus.elsewhere.example") },
        grants: ["network.http"], revision: 2, source: "local", installedAt: 1_690_000_000,
      },
    ],
  };
  const details = await openDetails(app as unknown as ReturnType<typeof updated>);
  fireEvent.click(within(details).getByRole("button", { name: "Roll back to 0.1.0" }));
  const review = screen.getByRole("region", { name: "Review rollback" });
  expect(review.textContent).toContain(
    "It reaches: The host of the URL saved in Prometheus URL (default https://prometheus.elsewhere.example), api.github.com.",
  );
  expect(within(review).getByRole("button", { name: "Roll back and grant permissions" })).toBeTruthy();
});
it("says a rollback with the same requests uses the permissions granted now", async () => {
  const current = networkApp();
  const app = {
    ...current,
    history: [{ manifest: { ...current.manifest, version: "0.1.0" }, grants: ["network.http"], revision: 2, source: "local", installedAt: 1_690_000_000 }],
  };
  const details = await openDetails(app as unknown as ReturnType<typeof updated>);
  fireEvent.click(within(details).getByRole("button", { name: "Roll back to 0.1.0" }));
  const review = screen.getByRole("region", { name: "Review rollback" });
  expect(review.textContent).toContain("It uses the permissions granted now.");
  expect(within(review).queryByRole("listitem", { name: "Binding up" })).toBeNull();
});
it("closes the reset confirmation with Escape without resetting", async () => {
  const details = await openDetails(updated());
  fireEvent.click(within(details).getByRole("button", { name: "Reset settings" }));
  const dialog = within(details).getByRole("alertdialog", { name: "Reset settings" });
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(within(details).queryByRole("alertdialog", { name: "Reset settings" })).toBeNull();
  expect(configureExtensions).not.toHaveBeenCalled();
});
it("limits an app to chosen clusters from its details, by stable context ID", async () => {
  // The name is presentation only; the saved list keys on each context's stable ID (#265).
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [
      { name: "cluster/a", stableId: "/kube/a.yaml#cluster/a", key: "/kube/a.yaml#cluster/a" },
      { name: "cluster/b", stableId: "/kube/b.yaml#cluster/b", key: "/kube/b.yaml#cluster/b" },
    ],
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
      expect(configureExtensions).toHaveBeenCalledWith({
        action: "clusters",
        id: "org.test.gitops",
        contexts: ["/kube/b.yaml#cluster/b"],
      }),
    );
  } finally {
    Object.assign(Range.prototype, measuring);
  }
});
it("allows every cluster again, and keeps listing a chosen cluster the kubeconfig no longer has", async () => {
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [{ name: "cluster/a", stableId: "/kube/a.yaml#cluster/a", key: "/kube/a.yaml#cluster/a" }],
  } as any);
  const app = { ...updated(), contexts: ["/kube/a.yaml#cluster/a", "/kube/gone.yaml#retired"] };
  const details = await openDetails(app);
  const clusters = within(details).getByRole("group", { name: "Clusters" });
  expect((within(clusters).getByLabelText("Only these clusters") as HTMLInputElement).checked).toBe(true);
  // A listed context shows its name; one the kubeconfig no longer has shows its ID, so it can be removed.
  expect(await within(clusters).findByRole("button", { name: "Remove cluster/a" })).toBeTruthy();
  fireEvent.click(within(clusters).getByRole("button", { name: "Remove /kube/gone.yaml#retired" }));
  expect(within(clusters).queryByRole("button", { name: "Remove /kube/gone.yaml#retired" })).toBeNull();
  fireEvent.click(within(clusters).getByLabelText("All clusters"));
  fireEvent.click(within(clusters).getByRole("button", { name: "Save clusters" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "clusters", id: "org.test.gitops", contexts: null }),
  );
});
it("offers a cluster added while the app details stay open", async () => {
  const { saveKubeconfigFiles, settingsStorage } = await import("@srelens/core");
  vi.mocked(listContexts)
    .mockResolvedValueOnce({ contexts: [{ name: "cluster/a", stableId: "/kube/a.yaml#cluster/a", key: "/kube/a.yaml#cluster/a" }] } as any)
    .mockResolvedValue({
      contexts: [
        { name: "cluster/a", stableId: "/kube/a.yaml#cluster/a", key: "/kube/a.yaml#cluster/a" },
        { name: "edge", stableId: "/kube/edge.yaml#edge", key: "/kube/edge.yaml#edge" },
      ],
    } as any);
  const app = { ...updated(), contexts: ["/kube/a.yaml#cluster/a"] };
  const details = await openDetails(app);
  const clusters = within(details).getByRole("group", { name: "Clusters" });
  expect(await within(clusters).findByRole("button", { name: "Remove cluster/a" })).toBeTruthy();
  // Connections adds a kubeconfig; storage may not even hold it yet.
  act(() => {
    const fail = vi.spyOn(settingsStorage, "setItem").mockImplementation(() => { throw new Error("unavailable"); });
    saveKubeconfigFiles(["/kube/edge.yaml"]);
    fail.mockRestore();
  });
  await waitFor(() => expect(listContexts).toHaveBeenLastCalledWith(["/kube/edge.yaml"]));
  fireEvent.click(within(clusters).getByRole("combobox", { name: "Add a cluster" }));
  expect(await screen.findByRole("option", { name: "edge" })).toBeTruthy();
});
it("offers a limited app on exactly the chosen one of two clusters sharing a stable ID", async () => {
  const { ExtensionResourceSlot } = await import("./Extensions");
  const installed = structuredClone(plugin);
  installed.manifest.contributions.detailTabs = [
    { id: "detail", title: "GitOps apps", capability: "list", forKinds: ["/Namespace"] },
  ];
  // `a` + `b#c` and `a#b` + `c` share a stable ID; the key encodes both parts (#623).
  installed.contexts = ["/kube/a#b%23c"];
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [installed] });
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [
      { name: "b#c", stableId: "/kube/a#b#c", key: "/kube/a#b%23c" },
      { name: "c", stableId: "/kube/a#b#c", key: "/kube/a%23b#c" },
    ],
  } as any);
  render(
    <>
      <div data-testid="b#c"><ExtensionResourceSlot context="b#c" kind="Namespace" namespace={null} name="argo" /></div>
      <div data-testid="c"><ExtensionResourceSlot context="c" kind="Namespace" namespace={null} name="argo" /></div>
    </>,
  );
  const tabs = await screen.findAllByRole("tab", { name: "GitOps apps" });
  expect(tabs.map((tab) => tab.closest("[data-testid]")?.getAttribute("data-testid"))).toEqual(["b#c"]);
  expect(screen.queryByRole("alert")).toBeNull();
});
it("says why the cluster list could not be loaded, and retries it", async () => {
  vi.mocked(listContexts)
    .mockResolvedValueOnce({ error: "kubeconfig unreadable" })
    .mockResolvedValue({ contexts: [{ name: "cluster/a", stableId: "a", key: "a" }] } as any);
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
  expect(source.textContent).not.toContain("Signed by");
});
it("names no publisher for a proof the host did not verify", async () => {
  const { signedBy: _unverified, ...app } = updated();
  const details = await openDetails(app);
  const source = within(details).getByText(/revision 4/);
  expect(source.textContent).toContain("Signature not verified");
  expect(source.textContent).not.toContain("Signed by");
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
it("says the settings were saved, though the save gives the app a new revision", async () => {
  const typed = (revision: number) => ({
    ...plugin, revision, manifest: { ...plugin.manifest, settings: [{ id: "team", type: "string", title: "Team" }] },
  });
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [typed(1)] } as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings for GitOps" }));
  const settings = screen.getByRole("form", { name: "GitOps settings" });
  fireEvent.change(within(settings).getByRole("textbox", { name: "Team" }), { target: { value: "platform" } });
  // The host answers the save with the app at its next revision.
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 3, plugins: [{ ...typed(2), settings: { team: "platform" } }] } as any);
  fireEvent.click(within(settings).getByRole("button", { name: "Save settings" }));
  await waitFor(() => expect(listExtensions).toHaveBeenCalledTimes(2));
  const form = await screen.findByRole("form", { name: "GitOps settings" });
  expect(within(form).getByRole("status").textContent).toBe("Settings saved.");
  expect((within(form).getByRole("textbox", { name: "Team" }) as HTMLInputElement).value).toBe("platform");
});
it("persists settings, disable and remove through the backend", async () => {
  const typed = { ...plugin, manifest: { ...plugin.manifest, settings: [{ id: "team", type: "string", title: "Team" }] } };
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [typed],
  });
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings for GitOps" }));
  // The app's declared settings, as a host form: no free-form JSON (#542).
  expect(screen.queryByLabelText("App settings (JSON object)")).toBeNull();
  const settings = screen.getByRole("form", { name: "GitOps settings" });
  fireEvent.change(within(settings).getByRole("textbox", { name: "Team" }), {
    target: { value: "platform" },
  });
  fireEvent.click(within(settings).getByRole("button", { name: "Save settings" }));
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
it("says removing an app deletes its secrets too", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1, nextRevision: 2,
    plugins: [{ ...plugin, manifest: { ...plugin.manifest, settings: [{ id: "token", type: "secret-reference", title: "Token" }] } }],
  } as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Remove" }));
  const dialog = screen.getByRole("alertdialog", { name: "Remove app" });
  expect(dialog.textContent).toContain("deletes its secrets from srelens's secrets vault");
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
it("says a limited app's resource views could not be checked when the clusters fail to list, and retries", async () => {
  const { ExtensionResourceSlot } = await import("./Extensions");
  const installed = structuredClone(plugin);
  installed.manifest.contributions.detailTabs = [
    { id: "detail", title: "GitOps apps", capability: "list", forKinds: ["/Namespace"] },
  ];
  installed.contexts = ["/kube/s.yaml#staging"];
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [installed] });
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
  vi.mocked(listContexts).mockResolvedValueOnce({ error: "kubeconfig unreadable" });
  render(<ExtensionResourceSlot context="staging" kind="Namespace" namespace={null} name="argo" />);
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("Could not list clusters");
  expect(alert.textContent).toContain("kubeconfig unreadable");
  vi.mocked(listContexts).mockResolvedValue({ contexts: [{ name: "staging", stableId: "/kube/s.yaml#staging", key: "/kube/s.yaml#staging" }] } as any);
  fireEvent.click(within(alert).getByRole("button", { name: "Retry" }));
  expect(await screen.findByRole("tab", { name: "GitOps apps" })).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});
it("ignores a slower, older cluster listing that finishes after a newer one", async () => {
  const { ExtensionResourceSlot } = await import("./Extensions");
  const installed = structuredClone(plugin);
  installed.manifest.contributions.detailTabs = [
    { id: "detail", title: "GitOps apps", capability: "list", forKinds: ["/Namespace"] },
  ];
  installed.contexts = ["/kube/s.yaml#staging"];
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [installed] });
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
  let finishOlder: (outcome: unknown) => void = () => {};
  vi.mocked(listContexts)
    .mockReturnValueOnce(new Promise((resolve) => (finishOlder = resolve)) as any)
    .mockResolvedValueOnce({ contexts: [{ name: "staging", stableId: "/kube/s.yaml#staging", key: "/kube/s.yaml#staging" }] } as any);
  render(<ExtensionResourceSlot context="staging" kind="Namespace" namespace={null} name="argo" />);
  await waitFor(() => expect(listContexts).toHaveBeenCalledTimes(1));
  fireEvent.focus(window);
  expect(await screen.findByRole("tab", { name: "GitOps apps" })).toBeTruthy();
  // The first listing answers last, with a list that no longer has the cluster.
  await act(async () => finishOlder({ contexts: [] }));
  expect(screen.getByRole("tab", { name: "GitOps apps" })).toBeTruthy();
});
it("lists the clusters again when the kubeconfig files change, without waiting for focus", async () => {
  const { ExtensionResourceSlot } = await import("./Extensions");
  const { saveKubeconfigFiles } = await import("@srelens/core");
  const installed = structuredClone(plugin);
  installed.manifest.contributions.detailTabs = [
    { id: "detail", title: "GitOps apps", capability: "list", forKinds: ["/Namespace"] },
  ];
  installed.contexts = ["/kube/edge.yaml#edge"];
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [installed] });
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
  render(<ExtensionResourceSlot context="edge" kind="Namespace" namespace={null} name="argo" />);
  await waitFor(() => expect(listContexts).toHaveBeenCalledTimes(1));
  // Settings → Contexts adds the kubeconfig that declares `edge`.
  vi.mocked(listContexts).mockResolvedValue({ contexts: [{ name: "edge", stableId: "/kube/edge.yaml#edge", key: "/kube/edge.yaml#edge" }] } as any);
  act(() => saveKubeconfigFiles(["/kube/edge.yaml"]));
  expect(await screen.findByRole("tab", { name: "GitOps apps" })).toBeTruthy();
  saveKubeconfigFiles([]);
});
it("lists with the kubeconfig files in use when saving them failed", async () => {
  const { ExtensionResourceSlot } = await import("./Extensions");
  const { saveKubeconfigFiles, settingsStorage } = await import("@srelens/core");
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [] });
  render(<ExtensionResourceSlot context="edge" kind="Namespace" namespace={null} name="argo" />);
  await waitFor(() => expect(listContexts).toHaveBeenCalledTimes(1));
  // Storage refused the save, so nothing is stored; the app still uses the new file.
  act(() => {
    const fail = vi.spyOn(settingsStorage, "setItem").mockImplementation(() => { throw new Error("unavailable"); });
    saveKubeconfigFiles(["/kube/edge.yaml"]);
    fail.mockRestore();
  });
  await waitFor(() => expect(listContexts).toHaveBeenLastCalledWith(["/kube/edge.yaml"]));
  // And keeps using it on the next refresh too.
  act(() => {
    fireEvent.focus(window);
  });
  await waitFor(() => expect(listContexts).toHaveBeenCalledTimes(3));
  expect(listContexts).toHaveBeenLastCalledWith(["/kube/edge.yaml"]);
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
it("explains a missing app API by every version a reader accepts (#547)", async () => {
  const installed = structuredClone(plugin);
  const binding = installed.manifest.capabilities[0];
  binding.versions = ["v2", "v2beta2"];
  delete binding.arguments.version;
  Object.assign(binding.arguments, { group: "helm.toolkit.fluxcd.io", plural: "helmreleases", kind: "HelmRelease" });
  vi.mocked(readExtension).mockRejectedValueOnce(new Error("ApiError: 404 page not found"));
  render(<ExtensionResults plugin={installed} capability="list" context="M01" />);
  expect(await screen.findByText("HelmRelease API unavailable")).toBeTruthy();
  expect(screen.getByRole("alert").textContent).toContain("helm.toolkit.fluxcd.io/v2 or v2beta2");
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
  let announce: () => void = () => {};
  vi.mocked(onExtensionInventoryChanged).mockImplementation(async (listener) => { announce = listener; return () => {}; });
  const {act}=await import("@testing-library/react");
  vi.useFakeTimers();
  let view: ReturnType<typeof render>;
  try {
    await act(async()=>{view=render(<Consumer />);});
    expect(screen.getByText("Installed plugin")).toBeTruthy();
    vi.mocked(listExtensions).mockResolvedValue({plugins:[]} as any);
    // No poll: nothing changes until the host announces the write (#566).
    await act(async()=>{await vi.advanceTimersByTimeAsync(30000);});
    expect(screen.getByText("Installed plugin")).toBeTruthy();
    await act(async()=>{announce(); await vi.advanceTimersByTimeAsync(0);});
    expect(screen.queryByText("Installed plugin")).toBeNull();
  } finally {view!.unmount();vi.useRealTimers();}
});
it("distinguishes filtered rows from an empty resource response", async () => {
  vi.mocked(readExtension).mockResolvedValue({items:[{name:"apps",namespace:"team",age:"1d",columns:[]}]});
  render(<ExtensionResults plugin={plugin} capability="list" context="test" search="missing" />);
  expect(await screen.findByText("No matching resources.")).toBeTruthy();
  expect(screen.queryByText("No resources returned by this app.")).toBeNull();
});

it("says when the backend capped the list and pages matching rows", async () => {
  const items = Array.from({ length: 150 }, (_, i) => ({
    name: `app-${i}`,
    namespace: "team",
    age: "1d",
    columns: [] as string[],
  }));
  vi.mocked(readExtension).mockResolvedValue({ items, truncated: true });
  render(<ExtensionResults plugin={plugin} capability="list" context="test" />);
  expect(
    await screen.findByText(
      "Showing the first 150 resources; more remain on the cluster.",
    ),
  ).toBeTruthy();
  expect(screen.getByText("app-0")).toBeTruthy();
  expect(screen.queryByText("app-100")).toBeNull();
  expect(screen.getByText(/50 matching rows not shown/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: /Show 50 more/ }));
  expect(screen.getByText("app-100")).toBeTruthy();
  expect(screen.queryByText(/matching rows not shown/)).toBeNull();
});

it("resets the visible page after a refresh", async () => {
  const items = Array.from({ length: 150 }, (_, i) => ({
    name: `app-${i}`,
    namespace: "team",
    age: "1d",
    columns: [] as string[],
  }));
  vi.mocked(readExtension).mockResolvedValue({ items });
  render(<ExtensionResults plugin={plugin} capability="list" context="test" />);
  await screen.findByText("app-0");
  fireEvent.click(screen.getByRole("button", { name: /Show 50 more/ }));
  expect(screen.getByText("app-100")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => {
    expect(screen.queryByText("app-100")).toBeNull();
    expect(screen.getByText("app-0")).toBeTruthy();
  });
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

it("shares one inventory feed and stops it after the last consumer unmounts", async () => {
  const {useExtensions}=await import("./Extensions");
  const {act}=await import("@testing-library/react");
  const Consumer=()=>{useExtensions();return null;};
  const stop=vi.fn();
  vi.mocked(onExtensionInventoryChanged).mockResolvedValue(stop);
  let first:ReturnType<typeof render>,second:ReturnType<typeof render>;
  try {
    await act(async()=>{first=render(<Consumer/>);second=render(<Consumer/>);});
    expect(listExtensions).toHaveBeenCalledTimes(1);
    expect(onExtensionInventoryChanged).toHaveBeenCalledTimes(1);
    first!.unmount();
    expect(stop).not.toHaveBeenCalled();
    second!.unmount();
    expect(stop).toHaveBeenCalledTimes(1);
  } finally {first!?.unmount();second!?.unmount();}
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
  const keyId = signature ? "ab".repeat(32) : undefined;
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: source, signature, keyId });
  vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [{ id: plugin.manifest.id, name: "Catalog GitOps", description: "GitOps resources", repository: "https://github.com/example/gitops", license: "MIT", release: { version: "0.1.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: true } }] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] } as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  const install = await screen.findByText("Install and grant permissions");
  expect(validateExtension).toHaveBeenCalledWith(source, plugin.manifest.permissions, signature, undefined, keyId);
  expect(configureExtensions).not.toHaveBeenCalled();
  expect(readExtension).not.toHaveBeenCalled();
  fireEvent.click(install);
  await waitFor(() => expect(configureExtensions).toHaveBeenCalledWith({ action: "install", manifest: source, grants: plugin.manifest.permissions, ...(signature ? {signature, keyId} : {}) }));
});
/**
 * The review names a publisher only once the host has verified the signature over these
 * exact bytes (#559): its `signedBy`, never an assumed "srelens".
 */
it("says who signed a catalog release only after the host verified it", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 1, plugins: [] });
  const source = JSON.stringify(plugin.manifest);
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: source, signature: [1, 2, 3] });
  vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [{ id: plugin.manifest.id, name: "Catalog GitOps", description: "GitOps resources", repository: "https://github.com/example/gitops", license: "MIT", release: { version: "0.1.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: true } }] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] } as any);
  let answer: (report: any) => void = () => {};
  vi.mocked(validateExtension).mockReturnValueOnce(new Promise((resolve) => { answer = resolve; }));
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  const review = await screen.findByLabelText("Review app permissions");
  expect(review.textContent).toContain("Checking the signature");
  expect(review.textContent).not.toContain("verified");
  answer({ errors: [], permissionDiff: { previousRevision: null, added: [], removed: [], unchanged: [] }, signedBy: { id: "example", name: "Example Labs" } });
  await waitFor(() => expect(review.textContent).toContain("Signature verified · Example Labs"));
  // A check that found a problem and verified no signature says so.
  vi.mocked(validateExtension).mockResolvedValueOnce({ errors: [{ code: "EXTENSION_INVALID_SIGNATURE", path: "", message: "App publisher signature is invalid" }] });
  fireEvent.click(within(review).getByRole("button", { name: "Cancel" }));
  fireEvent.click(await screen.findByText("Review installation"));
  const again = await screen.findByLabelText("Review app permissions");
  await waitFor(() => expect(again.textContent).toContain("Signature not verified"));
  expect(again.textContent).not.toContain("Signature verified");
});
it("installs a catalog release's package by the release and the package that were reviewed (#562)", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 1, plugins: [] });
  const source = JSON.stringify(plugin.manifest);
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: source, signature: [1, 2, 3], package: packaged });
  vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [{ id: plugin.manifest.id, name: "Catalog GitOps", description: "GitOps resources", repository: "https://github.com/example/gitops", license: "MIT", release: { version: "0.1.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: false, package: { url: "https://github.com/example/gitops/releases/download/v0.1.0/gitops.srelens-extension", sha256: packaged.sha256 } } }] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] } as any);
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  const install = await screen.findByText("Install and grant permissions");
  expect(validateExtension).toHaveBeenCalledWith(source, plugin.manifest.permissions, [1, 2, 3], packaged.digests, undefined);
  fireEvent.click(install);
  await waitFor(() => expect(configureExtensions).toHaveBeenCalledWith({
    action: "installCatalogPackage", id: plugin.manifest.id, sha256: "digest", packageSha256: packaged.sha256,
    grants: plugin.manifest.permissions,
  }));
});
it("reviews a signed catalog replacement against the installed revision before submitting it", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 8, plugins: [{ ...plugin, revision: 7 }] } as any);
  const source = JSON.stringify({ ...plugin.manifest, version: "0.2.0" });
  const signature = [1, 2, 3];
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: source, signature });
  vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [{ id: plugin.manifest.id, name: "GitOps", description: "", repository: "https://example.com", license: "MIT", release: { version: "0.2.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: true } }] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.1.0"], incompatible: [] } as any);
  vi.mocked(validateExtension).mockResolvedValue({ errors: [], permissionDiff: { previousRevision: 7, added: ["Action k8s.annotate on Widget"], removed: [], unchanged: ["Grant k8s.listCustomResource"] } });
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review replacement"));
  const review = await screen.findByLabelText("Review app permissions");
  expect(within(review).getByText(/Action k8s.annotate on Widget/)).toBeTruthy();
  fireEvent.click(within(review).getByRole("button", { name: "Cancel" }));
  expect(configureExtensions).not.toHaveBeenCalled();
  fireEvent.click(await screen.findByText("Review replacement"));
  fireEvent.click(await screen.findByRole("button", { name: "Update and grant permissions" }));
  await waitFor(() => expect(configureExtensions).toHaveBeenCalledWith({ action: "install", manifest: source, grants: plugin.manifest.permissions, signature, reviewedRevision: 7 }));
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

it("offers both contexts sharing a stable ID by their distinct keys", async () => {
 const {ExtensionClusters}=await import("./ExtensionClusters");
 vi.mocked(listContexts).mockResolvedValue({contexts:[{name:"one",stableId:"/k/a#b#c",key:"/k/a#b%23c"},{name:"two",stableId:"/k/a#b#c",key:"/k/a%23b#c"}]} as any);
 const change=vi.fn().mockResolvedValue(true);
 render(<ExtensionClusters plugin={{...plugin,contexts:["/k/a#b%23c"]}} busy={false} change={change}/>);
 expect(await screen.findByRole("button",{name:"Remove one"})).toBeTruthy();
 fireEvent.click(screen.getByRole("combobox",{name:"Add a cluster"}));
 fireEvent.click(await screen.findByRole("option",{name:"two"}));
 fireEvent.click(screen.getByRole("button",{name:"Save clusters"}));
 await waitFor(()=>expect(change).toHaveBeenCalledWith({action:"clusters",id:plugin.manifest.id,contexts:["/k/a#b%23c","/k/a%23b#c"]}));
});
it("retains published files across context-store subscriptions", async () => {
 const {ExtensionResourceSlot}=await import("./Extensions");
 const {saveKubeconfigFiles,settingsStorage}=await import("@srelens/core");
 const first=render(<ExtensionResourceSlot context="edge" kind="Namespace" namespace={null} name="argo"/>);
 await waitFor(()=>expect(listContexts).toHaveBeenCalled());
 const fail=vi.spyOn(settingsStorage,"setItem").mockImplementation(()=>{throw new Error("unavailable");});
 act(()=>saveKubeconfigFiles(["/kube/live.yaml"]));
 fail.mockRestore();
 await waitFor(()=>expect(listContexts).toHaveBeenLastCalledWith(["/kube/live.yaml"]));
 first.unmount();
 const failAgain=vi.spyOn(settingsStorage,"setItem").mockImplementation(()=>{throw new Error("unavailable");});
 saveKubeconfigFiles(["/kube/newer.yaml"]);
 failAgain.mockRestore();
 vi.mocked(listContexts).mockClear();
 render(<ExtensionResourceSlot context="edge" kind="Namespace" namespace={null} name="argo"/>);
 await waitFor(()=>expect(listContexts).toHaveBeenLastCalledWith(["/kube/newer.yaml"]));
});
it("uses live files saved before the cluster picker mounts", async () => {
 const {ExtensionClusters}=await import("./ExtensionClusters");
 const {saveKubeconfigFiles,settingsStorage}=await import("@srelens/core");
 const fail=vi.spyOn(settingsStorage,"setItem").mockImplementation(()=>{throw new Error("unavailable");});
 saveKubeconfigFiles(["/kube/session.yaml"]);
 fail.mockRestore();
 render(<ExtensionClusters plugin={plugin} busy={false} change={vi.fn()}/>);
 await waitFor(()=>expect(listContexts).toHaveBeenLastCalledWith(["/kube/session.yaml"]));
 saveKubeconfigFiles([]);
});

/** An app whose binding reads a custom resource, so the host offers actions on its rows. */
const actionable = {
  ...plugin,
  manifest: {
    ...plugin.manifest,
    capabilities: [{ ...plugin.manifest.capabilities[0], target: "k8s.listCustomResource" }],
  },
} as any;
const threeRows = {
  items: [
    { name: "apps", namespace: "team", age: "1d", columns: [] },
    { name: "infra", namespace: "team", age: "2d", columns: [] },
    { name: "web", namespace: "shop", age: "3d", columns: [] },
  ],
};
const menuDetail = {
  resource: { kind: "Kustomization", metadata: { name: "apps", namespace: "team", uid: "u", resourceVersion: "1" } },
  actions: ["reconcile"],
  actionMeta: { reconcile: { title: "Reconcile", availableWhen: [{jsonPath:".spec.suspend",notEquals:true,reason:"Resume this resource before requesting reconciliation"}], impact: "medium", confirm: "Reconcile [{kind} ]in cluster {cluster}?" } },
};

it("selects rows one by one and all at once, and says how many are selected", async () => {
  const { inspectExtensionResource } = await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue(threeRows as any);
  vi.mocked(inspectExtensionResource).mockResolvedValue(menuDetail as any);
  render(<ExtensionResults plugin={actionable} capability="list" context="cluster/a" />);
  const row = (await screen.findByLabelText("Select team/apps")) as HTMLInputElement;
  // Nothing is selected until the reader says so.
  expect(row.checked).toBe(false);
  expect(screen.queryByTestId("bulk-count")).toBeNull();
  fireEvent.click(row);
  expect((await screen.findByTestId("bulk-count")).textContent).toBe("1 selected");
  const all = screen.getByLabelText("Select all") as HTMLInputElement;
  // One of three: the header box says "some", not "none" and not "all".
  expect(all.indeterminate).toBe(true);
  fireEvent.click(all);
  expect(screen.getByTestId("bulk-count").textContent).toBe("3 selected");
  expect((screen.getByLabelText("Select shop/web") as HTMLInputElement).checked).toBe(true);
  fireEvent.click(all);
  expect(screen.queryByTestId("bulk-count")).toBeNull();
});

it("offers no selection column on a table whose rows the host has no actions for", async () => {
  vi.mocked(readExtension).mockResolvedValue(threeRows as any);
  render(<ExtensionResults plugin={plugin} capability="list" context="cluster/a" />);
  await screen.findByText("apps");
  expect(screen.queryByLabelText("Select all")).toBeNull();
  expect(screen.queryByLabelText("Select team/apps")).toBeNull();
});

it("drops the selection when the cluster under it changes", async () => {
  const { inspectExtensionResource } = await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue(threeRows as any);
  vi.mocked(inspectExtensionResource).mockResolvedValue(menuDetail as any);
  const view = render(<ExtensionResults plugin={actionable} capability="list" context="cluster/a" />);
  fireEvent.click(await screen.findByLabelText("Select team/apps"));
  expect((await screen.findByTestId("bulk-count")).textContent).toBe("1 selected");
  // A rail switch behind the bar must not leave prod's rows selected on staging.
  view.rerender(<ExtensionResults plugin={actionable} capability="list" context="cluster/b" />);
  expect(screen.queryByTestId("bulk-count")).toBeNull();
});

it("keeps a bulk run on screen while the writes it accepts refresh the list under it", async () => {
  // Every accepted write announces its resource, and this list reloads on it.
  // A reload that replaces the whole section with "Loading app resources…"
  // takes the run's progress and its result down with it — the reader watches
  // a twelve-resource run vanish at the first acceptance and is told nothing.
  const { inspectExtensionResource, actOnExtensionResource, EXTENSION_RESOURCE_CHANGED } =
    await import("@srelens/core");
  // The reload is a cluster read: it takes time, and while it is out the list
  // has no rows. That is the window the run has to survive.
  vi.mocked(readExtension)
    .mockResolvedValueOnce(threeRows as any)
    .mockReturnValue(new Promise(() => {}) as any);
  vi.mocked(inspectExtensionResource).mockResolvedValue(menuDetail as any);
  vi.mocked(actOnExtensionResource).mockImplementation(async (resource) => {
    window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED, { detail: resource }));
    return { requested: true };
  });
  render(<ExtensionResults plugin={actionable} capability="list" context="cluster/a" />);
  fireEvent.click(await screen.findByLabelText("Select all"));
  fireEvent.click(await screen.findByRole("button", { name: "Reconcile" }));
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 3 resources" }));
  expect((await screen.findByTestId("bulk-result")).getAttribute("data-status")).toBe("success");
  await act(async () => {
    await Promise.resolve();
  });
  // Still there after the reload the run's own writes set off, and the list is
  // reported as refreshing inside the section rather than replacing it.
  expect(screen.queryByTestId("bulk-result")).not.toBeNull();
  expect(screen.getByTestId("bulk-count").textContent).toBe("3 selected");
  expect(screen.queryByText("Loading app resources…")).toBeNull();
});

it("passes the host's availability predicate through to the count the bar shows", async () => {
  // The seam #550 fills. Nothing in this build supplies a predicate, so the
  // table has to be able to hand one down once something does.
  const { inspectExtensionResource } = await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue(threeRows as any);
  vi.mocked(inspectExtensionResource).mockResolvedValue(menuDetail as any);
  render(
    <ExtensionResults
      plugin={actionable}
      capability="list"
      context="cluster/a"
      actionAvailability={(_action, resource) => resource.namespace === "team"}
    />,
  );
  fireEvent.click(await screen.findByLabelText("Select all"));
  fireEvent.click(await screen.findByRole("button", { name: "Reconcile" }));
  expect(screen.getByTestId("bulk-applies").textContent).toBe("applies to 2 of 3");
  expect(screen.getByTestId("bulk-resources").textContent).not.toContain("shop/web");
});

it("confirms a bulk action once for the whole selection from the table", async () => {
  const { inspectExtensionResource, actOnExtensionResource } = await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue(threeRows as any);
  vi.mocked(inspectExtensionResource).mockResolvedValue(menuDetail as any);
  vi.mocked(actOnExtensionResource).mockResolvedValue({ requested: true });
  render(<ExtensionResults plugin={actionable} capability="list" context="cluster/a" />);
  fireEvent.click(await screen.findByLabelText("Select all"));
  fireEvent.click(await screen.findByRole("button", { name: "Reconcile" }));
  expect(screen.getByTestId("host-confirm-target").textContent).toBe("3 resources");
  expect(screen.getByTestId("bulk-resources").textContent).toContain("shop/web");
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 3 resources" }));
  await waitFor(() => expect(actOnExtensionResource).toHaveBeenCalledTimes(3));
  expect((await screen.findByTestId("bulk-result")).getAttribute("data-status")).toBe("success");
});

it("stops queued old-cluster writes on a scope change while in-flight writes finish", async () => {
  const { inspectExtensionResource, actOnExtensionResource } = await import("@srelens/core");
  const items = Array.from({ length: 8 }, (_, i) => ({ name: `app-${i}`, namespace: "team", age: "1d", columns: [] }));
  vi.mocked(readExtension).mockResolvedValue({ items } as any);
  vi.mocked(inspectExtensionResource).mockResolvedValue(menuDetail as any);
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  const completed: string[] = [];
  vi.mocked(actOnExtensionResource).mockImplementation(async (s) => {
    await held;
    completed.push(`${s.context}/${s.name}`);
    return { requested: true };
  });
  const view = render(<ExtensionResults plugin={actionable} capability="list" context="cluster/a" />);
  fireEvent.click(await screen.findByLabelText("Select all"));
  fireEvent.click(await screen.findByRole("button", { name: "Reconcile" }));
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 8 resources" }));
  await waitFor(() => expect(actOnExtensionResource).toHaveBeenCalledTimes(4));
  view.rerender(<ExtensionResults plugin={actionable} capability="list" context="cluster/b" />);
  await screen.findByLabelText("Select all");
  await act(async () => release());
  expect(completed).toEqual(["cluster/a/app-0", "cluster/a/app-1", "cluster/a/app-2", "cluster/a/app-3"]);
  expect(actOnExtensionResource).toHaveBeenCalledTimes(4);
  fireEvent.click(screen.getByLabelText("Select all"));
  expect(screen.queryByTestId("bulk-result")).toBeNull();
  expect(screen.queryByTestId("bulk-progress")).toBeNull();
});

it("uses real resource predicates for bulk applicability without an injected callback", async () => {
  const { inspectExtensionResource, actOnExtensionResource } = await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue(threeRows as any);
  vi.mocked(inspectExtensionResource).mockImplementation(async (s) => ({
    ...menuDetail,
    resource: { ...menuDetail.resource, spec: { suspend: s.name === "web" } },
  }) as any);
  vi.mocked(actOnExtensionResource).mockResolvedValue({ requested: true });
  render(<ExtensionResults plugin={actionable} capability="list" context="cluster/a" />);
  fireEvent.click(await screen.findByLabelText("Select all"));
  fireEvent.click(await screen.findByRole("button", { name: "Reconcile" }));
  expect(screen.getByTestId("bulk-applies").textContent).toBe("applies to 2 of 3");
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 2 resources" }));
  await screen.findByTestId("bulk-result");
  expect(vi.mocked(actOnExtensionResource).mock.calls.map(([s]) => s.name)).toEqual(["apps", "infra"]);
});

it("reports a failed availability read and retries instead of excluding an unread row", async () => {
  const { inspectExtensionResource, actOnExtensionResource } = await import("@srelens/core");
  vi.mocked(readExtension).mockResolvedValue(threeRows as any);
  vi.mocked(inspectExtensionResource).mockImplementation(async (s) => {
    if (s.name === "web") throw new Error("Resource read timed out");
    return menuDetail as any;
  });
  render(<ExtensionResults plugin={actionable} capability="list" context="cluster/a" />);
  fireEvent.click(await screen.findByLabelText("Select all"));
  expect((await screen.findByRole("alert")).textContent).toContain("Resource read timed out");
  expect(screen.queryByRole("button", { name: "Reconcile" })).toBeNull();
  expect(actOnExtensionResource).not.toHaveBeenCalled();
  vi.mocked(inspectExtensionResource).mockResolvedValue(menuDetail as any);
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  fireEvent.click(await screen.findByRole("button", { name: "Reconcile" }));
  expect(screen.getByTestId("host-confirm-target").textContent).toBe("3 resources");
});

it("persists the unsigned apps policy and explains affected apps without removing them", async () => {
  const blocked = { ...plugin, enabled: false, source: "catalog", policyBlocked: 'Turn on "Allow unsigned apps to modify clusters and run code" in Settings → Apps to enable this app.' };
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, allowUnsignedApps: false, plugins: [blocked] } as any);
  render(<ExtensionManager />);
  const policy = await screen.findByLabelText("Allow unsigned apps to modify clusters and run code") as HTMLInputElement;
  expect(policy.checked).toBe(false);
  expect((screen.getByLabelText("Enable GitOps") as HTMLInputElement).disabled).toBe(true);
  expect(screen.getByText(/Disabled: Turn on/)).toBeTruthy();
  expect(screen.getByText("Remove")).toBeTruthy();
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, allowUnsignedApps: true, plugins: [{ ...plugin, enabled: false, source: "catalog" }] } as any);
  fireEvent.click(policy);
  await waitFor(() => expect(configureExtensions).toHaveBeenCalledWith({ action: "unsignedApps", allowUnsignedApps: true }));
  await waitFor(() => expect(policy.checked).toBe(true));
  expect((screen.getByLabelText("Enable GitOps") as HTMLInputElement).checked).toBe(false);
});
/** An administrator's policy (#578), as a web server reports it with the user's apps. */
const serverPolicy = {
  allowedApps: null,
  blockedApps: ["org.test.blocked"],
  allowedPublishers: ["srelens"],
  allowUnsignedApps: false,
  allowedCapabilities: ["k8s.listCustomResource", "network.http"],
  allowWriteActions: false,
  networkCeiling: ["api.example.com", "*.corp.example.com"],
  allowExecutableApps: false,
  requiredApps: ["org.test.gitops", "org.srelens.flux"],
};
it("says what the server's policy allows, keeps a required app, and points to a missing one", async () => {
  host.tauri = false;
  try {
    vi.mocked(listExtensions).mockResolvedValue({
      schemaVersion: 1, nextRevision: 2, plugins: [plugin], policy: serverPolicy,
    } as any);
    vi.mocked(listExtensionCatalog).mockResolvedValue({ catalog: { extensions: [] }, fetchedAt: 1, stale: false, error: null, hostApiVersions: ["0.4.0"], incompatible: [] } as any);
    render(<ExtensionManager />);
    const policy = await screen.findByRole("region", { name: "Server policy" });
    const text = policy.textContent ?? "";
    for (const rule of [
      "Blocked apps: org.test.blocked",
      "Signed apps from: srelens",
      "Unsigned apps are not allowed",
      "Capabilities apps may use: k8s.listCustomResource, network.http",
      "Apps may not write to clusters",
      "network.http may reach: api.example.com, *.corp.example.com",
    ]) expect(text).toContain(rule);
    expect(text).not.toContain("Only these apps");
    // A required app the user has stays: neither removed nor turned off.
    expect(screen.getByText("Required by this server")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Remove" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByLabelText("Enable GitOps") as HTMLInputElement).disabled).toBe(true);
    // One they don't have is named, with the way to install it.
    expect(text).toContain("This server requires an app you have not installed: org.srelens.flux");
    fireEvent.click(within(policy).getByRole("button", { name: "Open Catalog" }));
    await waitFor(() => expect(listExtensionCatalog).toHaveBeenCalled());
    expect(screen.getByRole("tab", { name: "Catalog" }).getAttribute("aria-selected")).toBe("true");
    // The user's own switch for unsigned apps cannot grant what the policy refuses.
    const unsigned = screen.getByLabelText("Allow unsigned apps to modify clusters and run code") as HTMLInputElement;
    expect(unsigned.disabled).toBe(true);
    expect(screen.getByText("This server's policy allows only signed apps.")).toBeTruthy();
  } finally {
    host.tauri = true;
  }
});
it("describes a policy that leaves apps open, and one whose ceiling lets network.http reach nothing", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1, nextRevision: 2, plugins: [plugin],
    policy: { ...serverPolicy, allowedApps: ["org.test.gitops"], blockedApps: [], allowedPublishers: null, allowUnsignedApps: true,
      allowedCapabilities: null, allowWriteActions: true, networkCeiling: [], requiredApps: [] },
  } as any);
  render(<ExtensionManager />);
  const text = (await screen.findByRole("region", { name: "Server policy" })).textContent ?? "";
  expect(text).toContain("Only these apps: org.test.gitops");
  expect(text).toContain("network.http may reach no host");
  for (const absent of ["Blocked apps", "Signed apps from", "Unsigned apps are not allowed", "Capabilities apps may use", "may not write", "requires an app"])
    expect(text).not.toContain(absent);
  expect((screen.getByRole("button", { name: "Remove" }) as HTMLButtonElement).disabled).toBe(false);
  expect((screen.getByLabelText("Allow unsigned apps to modify clusters and run code") as HTMLInputElement).disabled).toBe(false);
});
it("says nothing about a policy on a host that has none", async () => {
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 2, plugins: [plugin] } as any);
  render(<ExtensionManager />);
  await screen.findByText("GitOps");
  expect(screen.queryByRole("region", { name: "Server policy" })).toBeNull();
  expect(screen.queryByText("Required by this server")).toBeNull();
});
/** A required app the user turned off before it was required can be turned back on. */
it("lets a required app that is off be enabled", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1, nextRevision: 2, plugins: [{ ...plugin, enabled: false }], policy: serverPolicy,
  } as any);
  render(<ExtensionManager />);
  const enable = (await screen.findByLabelText("Enable GitOps")) as HTMLInputElement;
  expect(enable.disabled).toBe(false);
  fireEvent.click(enable);
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({ action: "enable", id: "org.test.gitops", enabled: true }),
  );
});
/** A policy refusal is the server's rule, not something the manifest's author can fix. */
it("says a policy refusal is the server's, and counts only the manifest's own problems as its to fix", async () => {
  const refused = { code: "EXTENSION_POLICY_REFUSED", path: "id", message: "The administrator's policy blocks org.test.gitops" };
  vi.mocked(validateExtension).mockResolvedValue({ errors: [refused] } as any);
  render(<ExtensionManager />);
  fireEvent.change(await screen.findByLabelText("Local app manifest (JSON)"), { target: { value: JSON.stringify(plugin.manifest) } });
  fireEvent.click(screen.getByText("Review manifest"));
  const review = await screen.findByLabelText("Review app permissions");
  expect(await within(review).findByText("This server's policy does not allow installing this app:")).toBeTruthy();
  expect(within(review).queryByText(/in the manifest before installing/)).toBeNull();
  expect(within(review).queryByRole("button", { name: /grant permissions/ })).toBeNull();

  fireEvent.click(within(review).getByRole("button", { name: "Cancel" }));
  const invalid = { code: "EXTENSION_INVALID_VALUE", path: "name", message: "Must be 1–120 characters" };
  vi.mocked(validateExtension).mockResolvedValue({ errors: [invalid, refused] } as any);
  fireEvent.click(screen.getByText("Review manifest"));
  const again = await screen.findByLabelText("Review app permissions");
  // Only the manifest's own problem is counted as the manifest's to fix.
  expect(await within(again).findByText("This server's policy does not allow installing this app, and the manifest has a problem to fix:")).toBeTruthy();
  fireEvent.click(within(again).getByRole("button", { name: "Cancel" }));
  vi.mocked(validateExtension).mockResolvedValue({ errors: [invalid, { ...invalid, path: "version" }] } as any);
  fireEvent.click(screen.getByText("Review manifest"));
  const manifestOnly = await screen.findByLabelText("Review app permissions");
  expect(await within(manifestOnly).findByText("Fix these 2 problems in the manifest before installing:")).toBeTruthy();
});
// #567: the pod capabilities are the broker's alone too, and a granted one says what
// the host holds it to rather than "not provided".
it("describes a granted pod capability by the host's own facts", async () => {
  const app = {
    ...updated(),
    manifest: {
      ...updated().manifest,
      srelensApiVersion: "^0.5",
      permissions: ["k8s.listDeployments", "k8s.exec"],
      capabilities: [
        { name: "controllers", title: "Controllers", target: "k8s.listDeployments", arguments: {}, inputs: ["context", "namespace"] },
        { name: "status", title: "Status", target: "k8s.exec", inputs: [], arguments: { resource: "controllers", command: ["cmctl", "status"] } },
      ],
    },
    grants: ["k8s.listDeployments", "k8s.exec"],
  };
  const details = await openDetails(app as unknown as ReturnType<typeof updated>);
  const grants = within(details).getByRole("list", { name: "Granted capabilities" });
  const grant = within(grants).getByText("k8s.exec").closest("li")!.textContent;
  expect(grant).not.toContain("Not provided by this host");
  expect(grant).toContain("Sensitive · high impact · confirmed on every run");
});
