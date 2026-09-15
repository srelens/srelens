import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: () => true,
  listExtensions: vi.fn(),
  configureExtensions: vi.fn(),
  readExtension: vi.fn(),
}));
import {
  listExtensions,
  configureExtensions,
  readExtension,
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
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [plugin],
  });
  vi.mocked(readExtension).mockResolvedValue({ items: [] });
});
it("opens backend-owned app settings through classic controls", async () => {
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  fireEvent.change(screen.getByLabelText("App settings (JSON object)"), {
    target: { value: '{"team":"classic"}' },
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

it("offers an app's pages only on the clusters it is enabled for", async () => {
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["cluster/b"] }],
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
  vi.mocked(listExtensions).mockResolvedValue({
    schemaVersion: 1,
    nextRevision: 2,
    plugins: [{ ...plugin, contexts: ["cluster/b"] }],
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
