import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";

// The real page and workspace. Only the capability wrappers are faked, and
// `readExtension` narrows the way the host does: a card's rows only, in the
// namespaces it is given. Removing the narrowing anywhere between the route
// and the read shows rows the card never counted, which these tests catch.
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  isTauri: () => true,
  listExtensions: vi.fn(),
  listContexts: vi.fn(),
  listCrds: vi.fn(),
  readExtension: vi.fn(),
  resolveExtensionColumns: vi.fn(),
}));
vi.mock("../lib/clusters", () => ({
  useContexts: () => [{ name: "cluster/a", stableId: "cluster/a", key: "cluster/a" }],
  useContextsStatus: () => "loaded",
  useContextsError: () => "",
  getContexts: () => [],
  getKubeconfigFiles: () => [],
  setContexts: vi.fn(),
}));
vi.mock("../lib/tabsStore", () => ({ openTab: vi.fn() }));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
HTMLElement.prototype.scrollIntoView ??= () => {};

import {
  extensionCardRoute,
  extensionClusterRoute,
  listContexts,
  listCrds,
  listExtensions,
  readExtension,
  resolveExtensionColumns,
  type InstalledExtension,
} from "@srelens/core";
import { openTab } from "../lib/tabsStore";
import { ExtensionPage } from "./ExtensionPage";

const manifest = {
  id: "org.example.certs", name: "cert-manager", version: "1.0.0", srelensApiVersion: "^0.3", kind: "declarative",
  permissions: ["k8s.listCustomResource"],
  capabilities: [{ name: "certificates", title: "Certificates", target: "k8s.listCustomResource",
    arguments: { group: "cert-manager.io", version: "v1", plural: "certificates", kind: "Certificate", namespaced: true },
    inputs: ["context", "namespace"] }],
  contributions: {
    pages: [{ id: "certificates", title: "Certificates", capability: "certificates" }],
    detailTabs: [], detailLinks: [],
    dashboardCards: [{ id: "expiring", title: "Certificates expiring soon", size: "s", type: "count",
      source: "certificates", target: { page: "certificates" } }],
  },
};
const plugin = { manifest, enabled: true, revision: 4, grants: manifest.permissions, settings: {}, source: "local",
  installedAt: 1, history: [] } as unknown as InstalledExtension;

/** Every certificate on the cluster; the card counts the first three. */
const ROWS = [
  { name: "web-tls", namespace: "team", age: "1d", columns: [] as string[] },
  { name: "shop-tls", namespace: "prod", age: "1d", columns: [] as string[] },
  { name: "odd-tls", namespace: "other", age: "1d", columns: [] as string[] },
  { name: "plain-tls", namespace: "other", age: "1d", columns: [] as string[] },
];
const COUNTED = new Set(["web-tls", "shop-tls", "odd-tls"]);

beforeEach(async () => {
  vi.clearAllMocks();
  const workloads = await import("@srelens/core/lib/workloads");
  vi.spyOn(workloads, "listNamespaces").mockResolvedValue({ namespaces: ["other", "prod", "team"], summaries: [] });
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 5, plugins: [plugin] });
  vi.mocked(listContexts).mockResolvedValue({ contexts: [] } as never);
  vi.mocked(listCrds).mockResolvedValue({ crds: [{ name: "certificates.cert-manager.io", group: "cert-manager.io",
    version: "v1", kind: "Certificate", plural: "certificates", namespaced: true, versions: ["v1"], storageVersion: "v1" }] } as never);
  vi.mocked(resolveExtensionColumns).mockResolvedValue({ columns: [], cells: [] });
  vi.mocked(readExtension).mockImplementation((async (
    _id: string, _revision: number, _capability: string, _context: string,
    namespace = "", _crd = false, card?: string, namespaces?: string[],
  ) => {
    const scope = namespaces?.length ? namespaces : namespace ? [namespace] : [];
    return { items: ROWS.filter((row) => (!card || COUNTED.has(row.name)) && (!scope.length || scope.includes(row.namespace))) };
  }) as never);
});

const cardRoute = extensionCardRoute("cluster/a", manifest.id, "certificates", "", "expiring", ["team", "prod"]);
/**
 * Everything the last interaction set in motion, finished: React's pending
 * work is flushed, then every read the fake has been asked for has answered
 * and its answer has rendered. No clock is involved, so "the rows did not
 * change" means no read changed them, not that none had time to.
 */
async function settle() {
  await act(async () => {});
  await act(async () => {
    await Promise.allSettled(vi.mocked(readExtension).mock.results.map((result) => result.value));
  });
  await act(async () => {});
}

const shownRows = () =>
  screen.queryAllByRole("button").map((b) => b.textContent).filter((t) => t?.endsWith("-tls")).sort();

it("shows exactly the rows the card counted in its namespaces, under a banner naming them", async () => {
  render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={cardRoute} />);
  await waitFor(() => expect(shownRows()).toEqual(["shop-tls", "web-tls"]));
  // `odd-tls` matches the card too, but in a namespace the card did not count.
  expect(screen.queryByText("odd-tls")).toBeNull();
  expect(screen.getByText(/counted by/).closest("[role=status]")?.textContent).toContain("in prod and team");
  expect(readExtension).toHaveBeenCalledWith(manifest.id, 4, "certificates", "cluster/a", "", true, "expiring", ["prod", "team"]);
});

it("shows the card's namespaces in the page's picker, not All namespaces", async () => {
  render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={cardRoute} />);
  const picker = await screen.findByRole("combobox", { name: "App namespace" });
  expect(picker.textContent).toContain("prod, team");
  expect(picker.textContent).not.toContain("All namespaces");
});

it("leaves a single-namespace card route too, rather than showing the card's rows elsewhere", async () => {
  const single = extensionCardRoute("cluster/a", manifest.id, "certificates", "team", "expiring");
  render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={single} />);
  await waitFor(() => expect(shownRows()).toEqual(["web-tls"]));
  fireEvent.click(screen.getByRole("combobox", { name: "App namespace" }));
  fireEvent.click(await screen.findByRole("option", { name: "prod" }));
  expect(openTab).toHaveBeenCalledWith(
    extensionClusterRoute("cluster/a", manifest.id, "certificates", "prod"),
    { clusterName: "cluster/a" },
  );
  await settle();
  expect(shownRows()).toEqual(["web-tls"]);
});

it("leaves the card route when the picker chooses another namespace, and keeps the card's rows until then", async () => {
  render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={cardRoute} />);
  await waitFor(() => expect(shownRows()).toEqual(["shop-tls", "web-tls"]));
  fireEvent.click(screen.getByRole("combobox", { name: "App namespace" }));
  fireEvent.click(await screen.findByRole("option", { name: "other" }));
  // The plain page for the new namespace: no card, no namespace list.
  expect(openTab).toHaveBeenCalledWith(
    extensionClusterRoute("cluster/a", manifest.id, "certificates", "other"),
    { clusterName: "cluster/a" },
  );
  // This tab is still the card's route, so it still shows what the card counted.
  await settle();
  expect(shownRows()).toEqual(["shop-tls", "web-tls"]);
  expect(readExtension).not.toHaveBeenCalledWith(manifest.id, 4, "certificates", "cluster/a", "other", true, "expiring");
  const status = screen.getByText(/counted by/).closest("[role=status]") as HTMLElement;
  expect(within(status).getByText(/in prod and team/)).toBeTruthy();
});
