import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

// The real workspace, tab store, picker and collection scope. Only the host is
// faked, and it answers a collection's name filter the way the backend does.
const host = vi.hoisted(() => ({
  readExtension: vi.fn(),
  resolveExtensionColumns: vi.fn(),
  matchCollectionNames: vi.fn(),
  resolveNamespaceCollections: vi.fn(),
  listCrds: vi.fn(),
}));
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), ...host, isTauri: () => false }));

import { settingsStorage, type ClusterContext, type InstalledExtension } from "@srelens/core";
import { ExtensionWorkspace } from "./ExtensionWorkspace";
import { COLLECTIONS_KEY, __resetCollectionsForTests, saveCollection } from "../lib/collections";
import { __resetCollectionScopeForTests } from "../lib/collectionScope";
import { makeTab } from "../lib/tabs";
import { TabScope } from "../lib/tabScope";
import { setState, setTabNamespaceCollections } from "../lib/tabsStore";

const KIND = { group: "kustomize.toolkit.fluxcd.io", kind: "Kustomization" };
const plugin = {
  enabled: true, revision: 3, grants: [], settings: {},
  manifest: {
    id: "org.test.flux", name: "Flux", version: "1.0.0", permissions: [],
    capabilities: [{ name: "apps", title: "Apps", target: "k8s.listCustomResource", inputs: [],
      arguments: { ...KIND, version: "v1", plural: "kustomizations", namespaced: true } }],
    contributions: {
      detailTabs: [], detailLinks: [],
      statusResolvers: [{ forKinds: [`${KIND.group}/${KIND.kind}`], rules: [] }],
      pages: [
        { id: "overview", title: "Overview", capability: "apps", dashboard: { pages: ["apps"] } },
        { id: "apps", title: "Kustomizations", capability: "apps" },
      ],
    },
  },
} as unknown as InstalledExtension;
const [dashboard, list] = plugin.manifest.contributions.pages;
const cluster = { name: "c", stableId: "c", key: "c", pinnedId: "c" } as ClusterContext;
const row = (name: string) => ({ name, namespace: "prod-eu", age: "1d", columns: [], status: { status: "healthy", label: "Ready" } });

function page(props: Partial<Parameters<typeof ExtensionWorkspace>[0]> = {}) {
  return render(
    <TabScope.Provider value="t">
      <ExtensionWorkspace plugin={plugin} page={list} cluster={cluster} context="c" route="/events" actions={document.body} onPage={vi.fn()} onCard={vi.fn()} {...props} />
    </TabScope.Provider>,
  );
}

beforeEach(async () => {
  vi.clearAllMocks();
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  __resetCollectionScopeForTests();
  vi.spyOn(await import("@srelens/core/lib/workloads"), "listNamespaces").mockResolvedValue({ namespaces: ["prod-eu"], summaries: [{ name: "prod-eu", labels: {} }] } as never);
  host.listCrds.mockResolvedValue({ crds: [{ name: "kustomizations.kustomize.toolkit.fluxcd.io", ...KIND, version: "v1", plural: "kustomizations", namespaced: true, versions: ["v1"], storageVersion: "v1" }] });
  host.readExtension.mockResolvedValue({ items: [row("prod-a"), row("dev-b")] });
  host.resolveExtensionColumns.mockResolvedValue({ columns: [], cells: [] });
  host.resolveNamespaceCollections.mockResolvedValue({ namespaces: ["prod-eu"], partial: false, collections: [{ name: "prod", namespaces: ["prod-eu"] }] });
  host.matchCollectionNames.mockImplementation(async (_collections, _kind, names: string[]) => ({
    filtered: true, matches: names.map((name) => name.startsWith("prod-")), problems: [], narrowing: ["prod"],
  }));
  setState({
    workspaces: [{ id: "w", name: "W", clusters: ["c"], activeCluster: "c", tabs: [{ ...makeTab("/events"), id: "t" }], activeId: "t", closed: [] }],
    currentId: "w",
  } as never);
  saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"], resources: { kinds: [`${KIND.group}/${KIND.kind}`], nameRegex: "^prod-" } });
  setTabNamespaceCollections("t", "c", ["prod"]);
});

it("narrows an app's list by the name filter of the collection in force, count included", async () => {
  page();
  expect(await screen.findByRole("cell", { name: "prod-a" })).toBeTruthy();
  await waitFor(() => expect(screen.queryByRole("cell", { name: "dev-b" })).toBeNull());
  expect(screen.getByText("1 item")).toBeTruthy();
  expect(screen.getByText(/Showing only kustomizations whose name matches/)).toBeTruthy();
  expect(host.matchCollectionNames).toHaveBeenCalledWith(expect.anything(), KIND, ["dev-b", "prod-a"]);
});

it("counts a dashboard's page by the same filter, so its ring agrees with the list", async () => {
  page({ page: dashboard });
  expect(await screen.findByLabelText("1 Kustomizations")).toBeTruthy();
});

it("leaves a card's rows as the card counted them", async () => {
  page({ card: "suspended", namespace: "prod-eu" });
  expect(await screen.findByRole("cell", { name: "prod-a" })).toBeTruthy();
  expect(screen.getByRole("cell", { name: "dev-b" })).toBeTruthy();
  expect(host.matchCollectionNames).not.toHaveBeenCalled();
});
