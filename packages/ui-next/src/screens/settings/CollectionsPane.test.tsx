import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage, type ClusterContext } from "@srelens/core";

(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

const core = vi.hoisted(() => ({ resolveNamespaceCollections: vi.fn(), validateCollection: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), ...core }));
const active = vi.hoisted(() => ({ context: undefined as ClusterContext | undefined }));
vi.mock("../../lib/clusters", async (orig) => ({
  ...(await orig<typeof import("../../lib/clusters")>()),
  useActiveContext: () => active.context,
}));

import { COLLECTIONS_KEY, __resetCollectionsForTests, getCollections, saveCollection } from "../../lib/collections";
import { __resetCollectionScopeForTests, noteNamespaceListing } from "../../lib/collectionScope";
import { CollectionsPane } from "./CollectionsPane";

const SHOP: ClusterContext = {
  name: "kind-shop",
  stableId: "id-shop",
  key: "id-shop",
  cluster: "kind-shop",
  server: "https://127.0.0.1:6443",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

const row = (name: string) => document.querySelector(`[data-collection="${name}"]`) as HTMLElement;

beforeEach(() => {
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  __resetCollectionScopeForTests();
  core.validateCollection.mockReset().mockResolvedValue([]);
  core.resolveNamespaceCollections.mockReset();
  active.context = undefined;
});

describe("CollectionsPane", () => {
  it("says there are none, and how to make one", () => {
    render(<CollectionsPane />);
    expect(screen.getByText("No namespace collections yet")).toBeDefined();
  });

  it("lists the namespace collections by name, each with its rules, and no resource collection", () => {
    saveCollection({ name: "staging", type: "namespace", patterns: ["staging-*"] });
    saveCollection({ name: "prod", type: "namespace", description: "Production", patterns: ["prod-*"], labelSelector: "env=prod", members: ["shared"] });
    saveCollection({ name: "release", type: "resource", kinds: ["apps/Deployment"] });
    render(<CollectionsPane />);
    const names = [...document.querySelectorAll("[data-collection]")].map((el) => el.getAttribute("data-collection"));
    expect(names).toEqual(["prod", "staging"]);
    expect(within(row("prod")).getByText("Production")).toBeDefined();
    expect(within(row("prod")).getByText("shared · prod-* · env=prod")).toBeDefined();
  });

  it("creates a collection once the backend finds nothing wrong with it", async () => {
    render(<CollectionsPane />);
    await userEvent.click(screen.getByRole("button", { name: "New collection…" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Name" }), "prod");
    await userEvent.type(within(dialog).getByLabelText("Patterns"), "prod-*");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Label selector" }), "env=prod");
    await waitFor(() =>
      expect(core.validateCollection).toHaveBeenLastCalledWith(
        { name: "prod", type: "namespace", patterns: ["prod-*"], labelSelector: "env=prod" },
        [],
      ),
    );
    await userEvent.click(within(dialog).getByRole("button", { name: "Save collection" }));
    expect(getCollections()).toEqual([{ name: "prod", type: "namespace", patterns: ["prod-*"], labelSelector: "env=prod" }]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  /** #885: invalid patterns and selectors are refused inline, and block the save. */
  it("shows what the backend refuses against its field, and does not save", async () => {
    core.validateCollection.mockResolvedValue([
      { field: "patterns", index: 0, message: '"!prod-*": patterns starting with ! are reserved for exclusions.' },
      { field: "labelSelector", message: '"env in prod": the values after `in` go in parentheses, e.g. (a,b).' },
    ]);
    render(<CollectionsPane />);
    await userEvent.click(screen.getByRole("button", { name: "New collection…" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Name" }), "prod");
    expect(await within(dialog).findByText(/reserved for exclusions/)).toBeDefined();
    expect(within(dialog).getByText(/go in parentheses/)).toBeDefined();
    expect(within(dialog).getByRole("button", { name: "Save collection" }).hasAttribute("disabled")).toBe(true);
    expect(getCollections()).toEqual([]);
  });

  /** #885: each explicit member is marked found or not found, so a typo shows before go-live. */
  it("previews what a collection matches on the cluster in focus, and marks a member it does not have", async () => {
    active.context = SHOP;
    noteNamespaceListing(SHOP.stableId, [{ name: "payments" }, { name: "checkout" }], true);
    core.resolveNamespaceCollections.mockResolvedValue({
      namespaces: ["payments"],
      collections: [
        {
          name: "release",
          matched: ["payments"],
          members: [
            { name: "payments", status: "found" },
            { name: "paymnets", status: "missing" },
          ],
        },
      ],
      partial: false,
    });
    render(<CollectionsPane />);
    await userEvent.click(screen.getByRole("button", { name: "New collection…" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByRole("textbox", { name: "Name" }), "release");
    await userEvent.type(within(dialog).getByLabelText("Members"), "payments, paymnets");
    expect(await within(dialog).findByText("Matches 1 namespace on kind-shop.")).toBeDefined();
    const rows = within(dialog).getByRole("list", { name: "Matched namespaces" });
    expect(within(rows).getByText("member")).toBeDefined();
    expect(within(rows).getByText("not found")).toBeDefined();
    expect(core.resolveNamespaceCollections).toHaveBeenLastCalledWith(
      [{ name: "release", type: "namespace", members: ["payments", "paymnets"] }],
      [{ name: "payments" }, { name: "checkout" }],
      true,
    );
  });

  /** A pattern typed before the name is worth seeing at once. */
  it("previews a pattern before the collection has a name", async () => {
    active.context = SHOP;
    noteNamespaceListing(SHOP.stableId, [{ name: "team-a-api" }, { name: "team-b-api" }], true);
    core.validateCollection.mockResolvedValue([{ field: "name", message: "Give the collection a name." }]);
    core.resolveNamespaceCollections.mockResolvedValue({
      namespaces: ["team-a-api"],
      collections: [{ name: "preview", matched: ["team-a-api"], members: [] }],
      partial: false,
    });
    render(<CollectionsPane />);
    await userEvent.click(screen.getByRole("button", { name: "New collection…" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Patterns"), "team-a-*");
    expect(await within(dialog).findByText("Matches 1 namespace on kind-shop.")).toBeDefined();
    expect(core.resolveNamespaceCollections).toHaveBeenLastCalledWith(
      [{ name: "preview", type: "namespace", patterns: ["team-a-*"] }],
      expect.any(Array),
      true,
    );
    // The name is still asked for before saving.
    expect(within(dialog).queryByText("Give the collection a name.")).toBeNull();
  });

  it("keeps a namespace the preview found as a member when it is ticked, and drops it when unticked", async () => {
    active.context = SHOP;
    noteNamespaceListing(SHOP.stableId, [{ name: "team-a-api" }, { name: "team-a-web" }], true);
    core.resolveNamespaceCollections.mockResolvedValue({
      namespaces: ["team-a-api", "team-a-web"],
      collections: [{ name: "team-a", matched: ["team-a-api", "team-a-web"], members: [] }],
      partial: false,
    });
    render(<CollectionsPane />);
    await userEvent.click(screen.getByRole("button", { name: "New collection…" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Patterns"), "team-a-*");
    const keep = await within(dialog).findByRole("checkbox", { name: "Keep team-a-web as a member" });
    await userEvent.click(keep);
    const members = within(dialog).getByLabelText("Members") as HTMLTextAreaElement;
    expect(members.value).toBe("team-a-web");
    await userEvent.click(within(dialog).getByRole("checkbox", { name: "Keep team-a-api as a member" }));
    expect(members.value).toBe("team-a-web\nteam-a-api");
    await userEvent.click(within(dialog).getByRole("checkbox", { name: "Keep team-a-web as a member" }));
    expect(members.value).toBe("team-a-api");
  });

  it("edits a collection in place, and passes the other names it may not reuse", async () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    saveCollection({ name: "staging", type: "namespace", patterns: ["staging-*"] });
    render(<CollectionsPane />);
    await userEvent.click(within(row("prod")).getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");
    const patterns = within(dialog).getByLabelText("Patterns");
    await userEvent.clear(patterns);
    await userEvent.type(patterns, "live-*");
    await waitFor(() => expect(core.validateCollection).toHaveBeenLastCalledWith(
      { name: "prod", type: "namespace", patterns: ["live-*"] },
      ["staging"],
    ));
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(getCollections().find((c) => c.name === "prod")?.patterns).toEqual(["live-*"]);
  });

  it("deletes several at once, after asking", async () => {
    for (const name of ["release-1", "release-2", "prod"]) saveCollection({ name, type: "namespace", members: ["a"] });
    render(<CollectionsPane />);
    await userEvent.click(within(row("release-1")).getByRole("checkbox"));
    await userEvent.click(within(row("release-2")).getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Delete 2 selected" }));
    const confirm = screen.getByRole("dialog");
    expect(within(confirm).getByText("Delete 2 collections?")).toBeDefined();
    await userEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
    expect(getCollections().map((c) => c.name)).toEqual(["prod"]);
  });
});
