import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage, type ClusterContext, type NamespaceResolution } from "@srelens/core";
import type { NamespaceOptions } from "@srelens/core/react";

// jsdom has neither; the picker's command list uses both.
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
Element.prototype.scrollIntoView ??= () => {};

const core = vi.hoisted(() => ({ resolveNamespaceCollections: vi.fn(), validateCollection: vi.fn(), listNamespaces: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), ...core }));
const openSettings = vi.hoisted(() => vi.fn());
vi.mock("../../lib/settingsRequest", () => ({ openSettings }));

import { COLLECTIONS_KEY, __resetCollectionsForTests, getCollections, saveCollection } from "../../lib/collections";
import { __resetCollectionScopeForTests } from "../../lib/collectionScope";
import { makeTab } from "../../lib/tabs";
import { getState, setState, tabNamespaceCollections } from "../../lib/tabsStore";
import { TabScope } from "../../lib/tabScope";
import { CollectionScopeNotice, NamespaceScopePicker, RELIST_MS } from "./NamespaceScope";

const CONTEXT: ClusterContext = {
  name: "kind-shop",
  stableId: "id-shop",
  key: "id-shop",
  cluster: "kind-shop",
  server: "https://127.0.0.1:6443",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

const OPTIONS: NamespaceOptions = {
  namespaces: ["prod-eu", "prod-us", "dev"],
  items: [
    { name: "prod-eu", labels: { env: "prod" } },
    { name: "prod-us", labels: { env: "prod" } },
    { name: "dev", labels: {} },
  ],
  complete: true,
  scope: "",
  error: "",
};

const resolution = (namespaces: string[], partial = false): NamespaceResolution => ({
  namespaces,
  collections: [{ name: "prod", matched: namespaces, members: [] }],
  partial,
});

function paint(options: NamespaceOptions = OPTIONS) {
  return render(
    <TabScope.Provider value="tab-a">
      <NamespaceScopePicker context={CONTEXT} options={options} />
      <CollectionScopeNotice context={CONTEXT} />
    </TabScope.Provider>,
  );
}

const picker = () => screen.getByRole("combobox", { name: "Namespaces" });
const showTab = (name: RegExp) => userEvent.click(screen.getByRole("tab", { name }));
/** Open the picker and choose a collection on its tab. */
async function choose(name: string) {
  await userEvent.click(picker());
  await showTab(/^Collections/);
  await userEvent.click(screen.getByRole("option", { name }));
}
const tab = () => getState().workspaces[0].tabs.find((t) => t.id === "tab-a")!;

beforeEach(() => {
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  __resetCollectionScopeForTests();
  core.resolveNamespaceCollections.mockReset().mockResolvedValue(resolution(["prod-eu", "prod-us"]));
  core.validateCollection.mockReset().mockResolvedValue([]);
  core.listNamespaces.mockReset();
  openSettings.mockReset();
  setState({
    workspaces: [
      {
        id: "w",
        name: "W",
        clusters: [CONTEXT.stableId],
        activeCluster: CONTEXT.stableId,
        tabs: [{ ...makeTab("/k/pods", { clusterName: CONTEXT.name }), id: "tab-a" }],
        activeId: "tab-a",
        closed: [],
      },
    ],
    currentId: "w",
  });
});

describe("NamespaceScopePicker", () => {
  it("opens on the namespaces, beside a Collections tab that says how to make one while there are none", async () => {
    paint();
    await userEvent.click(picker());
    expect(screen.getByRole("tab", { name: "Namespaces" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("option", { name: "prod-eu" })).toBeDefined();
    await showTab(/^Collections/);
    expect(screen.getByText("No collections yet. Pick namespaces, then Save as collection…")).toBeDefined();
    expect(screen.queryByRole("option", { name: "prod-eu" })).toBeNull();
  });

  it("offers the saved namespace collections on their tab, most recently used first, and no resource collection", async () => {
    saveCollection({ name: "staging", type: "namespace", patterns: ["staging-*"] });
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"], description: "Production", lastUsedAt: 5 });
    saveCollection({ name: "release", type: "resource", kinds: ["apps/Deployment"] });
    paint();
    await userEvent.click(picker());
    await showTab(/^Collections/);
    expect(screen.getAllByRole("option").map((o) => o.getAttribute("aria-label"))).toEqual(["prod", "staging"]);
  });

  it("scopes the tab to a chosen collection and says what it came to", async () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    paint();
    await choose("prod");
    expect(tabNamespaceCollections("tab-a", CONTEXT.stableId)).toEqual(["prod"]);
    await waitFor(() => expect(picker().textContent).toBe("prod · 2 namespaces"));
    expect(screen.getByRole("tab", { name: "Collections (1)" })).toBeDefined();
    await showTab(/^Namespaces/);
    expect(core.resolveNamespaceCollections).toHaveBeenCalledWith(
      [expect.objectContaining({ name: "prod" })],
      OPTIONS.items,
      true,
    );
    // The namespaces it matched are ticked, first, and "All namespaces" is not the state.
    expect(screen.getByRole("option", { name: "All namespaces" }).getAttribute("aria-checked")).toBe("false");
    expect(screen.getByRole("option", { name: "prod-eu" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("option", { name: "prod-us" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("option", { name: "dev" }).getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText("2 selected")).toBeDefined();
    expect(screen.getByRole("button", { name: "Save as collection…" }).hasAttribute("disabled")).toBe(true);
  });

  /** "This collection, less one" is a hand-picked set, starting from the collection's. */
  it("turns into namespaces picked by hand, starting from the collection's, when one is unticked", async () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    paint();
    await choose("prod");
    await waitFor(() => expect(screen.getByRole("tab", { name: "Namespaces (2)" })).toBeDefined());
    await showTab(/^Namespaces/);
    await userEvent.click(screen.getByRole("option", { name: "prod-us" }));
    expect(tabNamespaceCollections("tab-a", CONTEXT.stableId)).toBeUndefined();
    expect(tab().namespaces).toEqual({ [CONTEXT.stableId]: ["prod-eu"] });
    expect(picker().textContent).toBe("prod-eu");
    expect(screen.getByRole("tab", { name: "Collections" })).toBeDefined();
  });

  it("goes back to every namespace on Reset, collection and all", async () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    paint();
    await choose("prod");
    await userEvent.click(screen.getByRole("button", { name: "Reset" }));
    expect(tabNamespaceCollections("tab-a", CONTEXT.stableId)).toBeUndefined();
    expect(picker().textContent).toBe("All namespaces");
  });

  it("saves the namespaces picked by hand as a collection, named by the reader, and scopes the tab to it", async () => {
    paint();
    await userEvent.click(picker());
    expect(screen.getByRole("button", { name: "Save as collection…" }).hasAttribute("disabled")).toBe(true);
    await userEvent.click(screen.getByRole("option", { name: "prod-eu" }));
    await userEvent.click(screen.getByRole("option", { name: "dev" }));
    await userEvent.click(screen.getByRole("button", { name: "Save as collection…" }));

    const dialog = await screen.findByRole("dialog");
    expect((within(dialog).getByLabelText("Members") as HTMLTextAreaElement).value).toBe("prod-eu\ndev");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Name" }), "picked");
    await waitFor(() => expect(core.validateCollection).toHaveBeenLastCalledWith(
      { name: "picked", type: "namespace", members: ["prod-eu", "dev"] },
      [],
    ));
    await userEvent.click(within(dialog).getByRole("button", { name: "Save collection" }));

    expect(getCollections()).toEqual([{ name: "picked", type: "namespace", members: ["prod-eu", "dev"], lastUsedAt: expect.any(Number) }]);
    expect(tabNamespaceCollections("tab-a", CONTEXT.stableId)).toEqual(["picked"]);
  });

  /**
   * The view reads its namespaces once, when it opens; a collection is a rule,
   * and a namespace created or relabelled since may now match it.
   */
  it("reads the namespaces again while a collection is in force, and resolves against what it finds", async () => {
    // Before anything mounts, so the picker's interval is a fake one.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
      paint();
      await choose("prod");
      await waitFor(() => expect(picker().textContent).toBe("prod · 2 namespaces"));
      core.listNamespaces.mockResolvedValue({
        namespaces: ["prod-eu", "prod-us", "prod-ap", "dev"],
        summaries: ["prod-eu", "prod-us", "prod-ap", "dev"].map((name) => ({ name, phase: "Active", labels: {}, age: "1d" })),
      });
      core.resolveNamespaceCollections.mockResolvedValue(resolution(["prod-eu", "prod-us", "prod-ap"]));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(RELIST_MS);
      });
      await waitFor(() => expect(picker().textContent).toBe("prod · 3 namespaces"));
      expect(core.listNamespaces).toHaveBeenCalledWith(CONTEXT.name);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not read the namespaces again while nothing but namespaces is chosen", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      paint();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(RELIST_MS * 2);
      });
      expect(core.listNamespaces).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("opens on the Collections tab while a collection is in force", async () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    paint();
    await choose("prod");
    await userEvent.keyboard("{Escape}");
    await userEvent.click(picker());
    expect(screen.getByRole("tab", { name: "Collections (1)" }).getAttribute("aria-selected")).toBe("true");
  });

  it("opens Settings on Collections from Manage collections…", async () => {
    paint();
    await userEvent.click(picker());
    await userEvent.click(screen.getByRole("button", { name: "Manage collections…" }));
    expect(openSettings).toHaveBeenCalledExactlyOnceWith("collections");
  });
});

describe("CollectionScopeNotice", () => {
  async function scopedTo(namespaces: string[], partial = false) {
    core.resolveNamespaceCollections.mockResolvedValue(resolution(namespaces, partial));
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    paint();
    await choose("prod");
    await userEvent.keyboard("{Escape}");
  }

  it("says plainly when a collection matches no namespace here, and offers every namespace back", async () => {
    await scopedTo([]);
    const alert = await screen.findByText("No namespaces match prod on kind-shop");
    expect(alert).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Show all namespaces" }));
    expect(tabNamespaceCollections("tab-a", CONTEXT.stableId)).toBeUndefined();
  });

  it("says nothing once the collection has matched, and says it may be partial for restricted credentials", async () => {
    await scopedTo(["prod-eu"], true);
    expect(await screen.findByText(/Results may be partial/)).toBeDefined();
  });

  it("says it is working the collection out until the answer comes", async () => {
    let answer: (r: NamespaceResolution) => void = () => {};
    core.resolveNamespaceCollections.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    paint();
    await choose("prod");
    expect(screen.getByText(/Working out which namespaces prod covers on kind-shop/)).toBeDefined();
    await act(async () => answer(resolution(["prod-eu"])));
    expect(screen.queryByText(/Working out/)).toBeNull();
  });
});
