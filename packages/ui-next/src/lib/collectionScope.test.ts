import { beforeEach, describe, expect, it, vi } from "vitest";
import { SELECTS_NOTHING, settingsStorage, type NamespaceResolution } from "@srelens/core";

const resolveNamespaceCollections = vi.hoisted(() => vi.fn());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), resolveNamespaceCollections }));

import { COLLECTIONS_KEY, __resetCollectionsForTests, deleteCollections, saveCollection } from "./collections";
import { __resetCollectionScopeForTests, collectionScope, noteNamespaceListing, subscribeScopes } from "./collectionScope";

const answer = (namespaces: string[], partial = false): NamespaceResolution => ({
  namespaces,
  collections: [{ name: "prod", matched: namespaces, members: [] }],
  partial,
});
const LISTING = [{ name: "prod-eu", labels: { env: "prod" } }, { name: "dev" }];

async function settled() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  __resetCollectionScopeForTests();
  resolveNamespaceCollections.mockReset().mockResolvedValue(answer(["prod-eu"]));
  saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
});

describe("collectionScope", () => {
  /** Waiting is never "every namespace": a list scoped to prod must not show the cluster meanwhile. */
  it("selects nothing until a listing arrives, then resolves against it", async () => {
    expect(collectionScope("c1", ["prod"])).toMatchObject({ status: "waiting", selection: SELECTS_NOTHING });
    expect(resolveNamespaceCollections).not.toHaveBeenCalled();

    noteNamespaceListing("c1", LISTING, true);
    expect(collectionScope("c1", ["prod"])).toMatchObject({ status: "resolving", selection: SELECTS_NOTHING });
    expect(resolveNamespaceCollections).toHaveBeenCalledExactlyOnceWith(
      [{ name: "prod", type: "namespace", patterns: ["prod-*"] }],
      LISTING,
      true,
    );
    await settled();
    expect(collectionScope("c1", ["prod"])).toMatchObject({ status: "ready", selection: ["prod-eu"] });
  });

  it("asks once per listing and definition, however often it is read, and answers the same object", async () => {
    noteNamespaceListing("c1", LISTING, true);
    collectionScope("c1", ["prod"]);
    collectionScope("c1", ["prod"]);
    await settled();
    expect(collectionScope("c1", ["prod"])).toBe(collectionScope("c1", ["prod"]));
    expect(resolveNamespaceCollections).toHaveBeenCalledTimes(1);
    // The same listing again changes nothing.
    noteNamespaceListing("c1", [...LISTING], true);
    collectionScope("c1", ["prod"]);
    expect(resolveNamespaceCollections).toHaveBeenCalledTimes(1);
  });

  it("is a selection of nothing, not every namespace, when nothing matches", async () => {
    resolveNamespaceCollections.mockResolvedValue(answer([]));
    noteNamespaceListing("c1", LISTING, true);
    collectionScope("c1", ["prod"]);
    await settled();
    expect(collectionScope("c1", ["prod"])).toMatchObject({ status: "ready", selection: SELECTS_NOTHING });
  });

  /** #885: a collection edited while in use re-resolves at once, and so does a new namespace. */
  it("resolves again when the collection is edited or the listing changes, and tells its listeners", async () => {
    const heard = vi.fn();
    subscribeScopes(heard);
    noteNamespaceListing("c1", LISTING, true);
    collectionScope("c1", ["prod"]);
    await settled();

    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*", "live-*"] }, "prod");
    expect(heard).toHaveBeenCalled();
    resolveNamespaceCollections.mockResolvedValue(answer(["prod-eu", "live-1"]));
    collectionScope("c1", ["prod"]);
    await settled();
    expect(collectionScope("c1", ["prod"]).selection).toEqual(["prod-eu", "live-1"]);

    noteNamespaceListing("c1", [...LISTING, { name: "live-2" }], true);
    resolveNamespaceCollections.mockResolvedValue(answer(["prod-eu", "live-1", "live-2"]));
    collectionScope("c1", ["prod"]);
    await settled();
    expect(collectionScope("c1", ["prod"]).selection).toEqual(["prod-eu", "live-1", "live-2"]);
    expect(resolveNamespaceCollections).toHaveBeenCalledTimes(3);
  });

  it("does not resolve again when only the last-used time changes", async () => {
    noteNamespaceListing("c1", LISTING, true);
    collectionScope("c1", ["prod"]);
    await settled();
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"], lastUsedAt: 99 }, "prod");
    collectionScope("c1", ["prod"]);
    expect(resolveNamespaceCollections).toHaveBeenCalledTimes(1);
  });

  it("is gone when every named collection has been deleted, and resolves the ones left otherwise", async () => {
    saveCollection({ name: "dev", type: "namespace", members: ["dev"] });
    noteNamespaceListing("c1", LISTING, true);
    deleteCollections("namespace", ["prod"]);
    expect(collectionScope("c1", ["prod"]).status).toBe("gone");
    expect(collectionScope("c1", ["prod", "dev"]).collections.map((c) => c.name)).toEqual(["dev"]);
  });

  it("keeps clusters apart", async () => {
    noteNamespaceListing("c1", LISTING, true);
    collectionScope("c1", ["prod"]);
    await settled();
    expect(collectionScope("c2", ["prod"]).status).toBe("waiting");
  });

  it("says so when the backend could not resolve it, and selects nothing", async () => {
    resolveNamespaceCollections.mockRejectedValue(new Error("backend gone"));
    noteNamespaceListing("c1", LISTING, true);
    collectionScope("c1", ["prod"]);
    await settled();
    expect(collectionScope("c1", ["prod"])).toMatchObject({ status: "failed", error: "backend gone", selection: SELECTS_NOTHING });
  });
});
