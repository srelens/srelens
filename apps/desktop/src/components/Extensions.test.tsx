import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
const host = vi.hoisted(() => ({ tauri: true }));
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: () => host.tauri,
  listExtensions: vi.fn(),
  configureExtensions: vi.fn(),
  readExtension: vi.fn(),
  listContexts: vi.fn(),
}));
import {
  listExtensions,
  configureExtensions,
  readExtension,
  listContexts,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionManager, ExtensionResourceSlot } from "./Extensions";
import manifest from "../../../../examples/extensions/argocd.json";
const plugin = {
  manifest,
  enabled: true,
  revision: 1,
  grants: manifest.permissions,
  settings: {},
  source: "catalog",
  installedAt: 1,
  history: [],
} as InstalledExtension;
beforeEach(() => {
  vi.clearAllMocks();
  host.tauri = true;
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [plugin],
  });
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
});
it("opens backend-owned app settings through classic controls", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, manifest: { ...plugin.manifest, settings: [{ id: "team", type: "string", title: "Team" }] } }],
  });
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: `Settings for ${manifest.name}` }));
  fireEvent.change(screen.getByRole("textbox", { name: "Team" }), {
    target: { value: "classic" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "settings",
      id: manifest.id,
      settings: { team: "classic" },
    }),
  );
});
it("renders the same namespace contribution using classic resource tabs", async () => {
  render(
    <ExtensionResourceSlot
      context="classic-cluster"
      kind="Namespace"
      namespace={null}
      name="argo"
    />,
  );
  expect(await screen.findByRole("tab", { name: "Argo CD" })).toBeTruthy();
  await waitFor(() =>
    expect(readExtension).toHaveBeenCalledWith(
      manifest.id,
      1,
      "applications",
      "classic-cluster",
      "argo",
      true,
    ),
  );
  fireEvent.click(screen.getByText("App links"));
  expect(
    screen.getByRole("button", { name: "Inspect Argo CD resources" }),
  ).toBeTruthy();
});

it("opens native app pages from classic's connected-cluster navigation without a settings picker", async () => {
  const {ClassicAppsNav}=await import("./Extensions");
  const open=vi.fn();
  render(<ClassicAppsNav context="cluster/a" onOpen={open}/>);
  const apps=await screen.findByText("Apps"); fireEvent.click(apps);
  fireEvent.click(await screen.findByRole("button",{name:"Open Applications"}));
  expect(open).toHaveBeenCalledWith("cluster/a",manifest.id,manifest.contributions.pages[0].id);
  expect(screen.queryByText("Choose a cluster")).toBeNull();
});

/** The web server keeps each user's own apps (#515), so classic's app tabs open there too. */
it("opens app pages from classic's cluster navigation on the web", async () => {
  host.tauri = false;
  const {ClassicAppsNav}=await import("./Extensions");
  const open=vi.fn();
  render(<ClassicAppsNav context="cluster/a" onOpen={open}/>);
  fireEvent.click(await screen.findByText("Apps"));
  fireEvent.click(await screen.findByRole("button",{name:"Open Applications"}));
  expect(open).toHaveBeenCalledWith("cluster/a",manifest.id,manifest.contributions.pages[0].id);
  expect(listExtensions).toHaveBeenCalled();
});

/** The contexts both clusters tests list: names are presentation, stable IDs are identity. */
const listTwoClusters = () =>
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [
      { name: "cluster/a", stableId: "/kube/a.yaml#cluster/a", key: "/kube/a.yaml#cluster/a" },
      { name: "cluster/b", stableId: "/kube/b.yaml#cluster/b", key: "/kube/b.yaml#cluster/b" },
    ],
  } as any);

it("offers an app's pages only on the clusters it is enabled for", async () => {
  listTwoClusters();
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/b.yaml#cluster/b"] }],
  });
  const { ClassicAppsNav } = await import("./Extensions");
  render(
    <>
      <div data-testid="cluster/a"><ClassicAppsNav context="cluster/a" onOpen={vi.fn()} /></div>
      <div data-testid="cluster/b"><ClassicAppsNav context="cluster/b" onOpen={vi.fn()} /></div>
    </>,
  );
  // Both navigations read the same inventory, so once cluster/b lists the app, cluster/a has loaded too.
  const apps = await screen.findAllByText("Apps");
  expect(apps.map((node) => node.closest("[data-testid]")?.getAttribute("data-testid"))).toEqual(["cluster/b"]);
});

it("offers an app's resource tabs only on the clusters it is enabled for", async () => {
  listTwoClusters();
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/b.yaml#cluster/b"] }],
  });
  render(
    <>
      <div data-testid="cluster/a"><ExtensionResourceSlot context="cluster/a" kind="Namespace" namespace={null} name="argo" /></div>
      <div data-testid="cluster/b"><ExtensionResourceSlot context="cluster/b" kind="Namespace" namespace={null} name="argo" /></div>
    </>,
  );
  const tabs = await screen.findAllByRole("tab", { name: "Argo CD" });
  expect(tabs.map((tab) => tab.closest("[data-testid]")?.getAttribute("data-testid"))).toEqual(["cluster/b"]);
  expect(readExtension).not.toHaveBeenCalledWith(manifest.id, 1, "applications", "cluster/a", "argo", true);
});

it("says the clusters could not be listed, rather than that a limited app is not enabled, and retries", async () => {
  listTwoClusters();
  vi.mocked(listContexts).mockResolvedValueOnce({ error: "kubeconfig unreadable" });
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/a.yaml#cluster/a"] }],
  });
  const { ClassicAppPage } = await import("./Extensions");
  render(<ClassicAppPage context="cluster/a" id={manifest.id} page={manifest.contributions.pages[0].id} onPage={vi.fn()} />);
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("Could not list clusters");
  expect(alert.textContent).toContain("kubeconfig unreadable");
  expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  expect(listContexts).toHaveBeenCalledTimes(2);
  expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
});

it("keeps an app's pages when a later cluster listing fails", async () => {
  listTwoClusters();
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/a.yaml#cluster/a"] }],
  });
  const { ClassicAppsNav } = await import("./Extensions");
  render(<ClassicAppsNav context="cluster/a" onOpen={vi.fn()} />);
  expect(await screen.findByText("Apps")).toBeTruthy();
  vi.mocked(listContexts).mockResolvedValue({ error: "kubeconfig unreadable" });
  await act(async () => {
    fireEvent.focus(window);
  });
  expect(listContexts).toHaveBeenCalledTimes(2);
  // A refresh that could not be made takes nothing away: cluster/a is still the allowed cluster.
  expect(screen.getByText("Apps")).toBeTruthy();
});

it("says why the classic Apps navigation cannot show a limited app, and retries", async () => {
  listTwoClusters();
  vi.mocked(listContexts).mockResolvedValueOnce({ error: "kubeconfig unreadable" });
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/a.yaml#cluster/a"] }],
  });
  const { ClassicAppsNav } = await import("./Extensions");
  render(<ClassicAppsNav context="cluster/a" onOpen={vi.fn()} />);
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("Could not list clusters");
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(await screen.findByText("Apps")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("opens a limited app on the chosen one of two clusters sharing a stable ID, by key", async () => {
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [
      { name: "b#c", stableId: "/kube/a#b#c", key: "/kube/a#b%23c" },
      { name: "c", stableId: "/kube/a#b#c", key: "/kube/a%23b#c" },
    ],
  } as any);
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/a#b%23c"] }],
  });
  const { ClassicAppPage } = await import("./Extensions");
  const { unmount } = render(<ClassicAppPage context="c" id={manifest.id} page={manifest.contributions.pages[0].id} onPage={vi.fn()} />);
  expect(await screen.findByText(/not enabled for this cluster/)).toBeTruthy();
  unmount();
  render(<ClassicAppPage context="b#c" id={manifest.id} page={manifest.contributions.pages[0].id} onPage={vi.fn()} />);
  await waitFor(() => expect(screen.queryByText(/not enabled for this cluster|Loading app/)).toBeNull());
});
it("says it cannot tell which cluster a name means when the name is also another context's pinned ID", async () => {
  // A context literally named after another's pinned ID: the host's find_context refuses the
  // string, so which one this page is for is not known, and "not enabled" would be a guess.
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [
      { name: "c", stableId: "/kube/a#b#c", key: "/kube/a%23b#c", pinnedId: "srelens-context:/kube/a%23b#c" },
      { name: "srelens-context:/kube/a%23b#c", stableId: "/kube/x#srelens-context:/kube/a%23b#c",
        key: "/kube/x#srelens-context:/kube/a%2523b%23c", pinnedId: "srelens-context:/kube/x#srelens-context:/kube/a%2523b%23c" },
    ],
  } as any);
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/x#srelens-context:/kube/a%2523b%23c"] }],
  });
  const { ClassicAppPage } = await import("./Extensions");
  render(<ClassicAppPage context="srelens-context:/kube/a%23b#c" id={manifest.id} page={manifest.contributions.pages[0].id} onPage={vi.fn()} />);
  expect(await screen.findByText(/cannot tell which one it is for/)).toBeTruthy();
  expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
  expect(readExtension).not.toHaveBeenCalled();
});
it("says the cluster is gone rather than that a limited app is not enabled", async () => {
  vi.mocked(listContexts).mockResolvedValue({
    contexts: [{ name: "cluster/b", stableId: "/kube/b.yaml#cluster/b", key: "/kube/b.yaml#cluster/b" }],
  } as any);
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["/kube/a.yaml#cluster/a"] }],
  });
  const { ClassicAppPage } = await import("./Extensions");
  render(<ClassicAppPage context="cluster/a" id={manifest.id} page={manifest.contributions.pages[0].id} onPage={vi.fn()} />);
  expect(await screen.findByText(/no longer in your kubeconfig files/)).toBeTruthy();
  expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
});
