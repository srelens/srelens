import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage } from "@srelens/core";

const refreshSetting = vi.hoisted(() => vi.fn());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), refreshSetting }));

import {
  COLLECTIONS_KEY,
  __resetCollectionsForTests,
  byRecentUse,
  collectionsSaved,
  deleteCollections,
  getCollections,
  markCollectionsUsed,
  onCollectionRenamed,
  refreshCollections,
  saveCollection,
} from "./collections";

const stored = () => JSON.parse(settingsStorage.getItem(COLLECTIONS_KEY) ?? "null");

beforeEach(() => {
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  refreshSetting.mockReset().mockResolvedValue(undefined);
});

/** Another window saving its own change, seen when this one reads the setting again. */
function anotherWindowSaves(collections: unknown[]): void {
  refreshSetting.mockImplementationOnce(async () => {
    settingsStorage.setItem(COLLECTIONS_KEY, JSON.stringify({ version: 1, collections }));
  });
}

describe("the saved collections", () => {
  it("start as none", () => {
    expect(getCollections()).toEqual([]);
  });

  it("keep a saved collection across a restart, in a versioned document", async () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
    await collectionsSaved();
    __resetCollectionsForTests();
    expect(getCollections()).toEqual([{ name: "prod", type: "namespace", patterns: ["prod-*"] }]);
    expect(stored()).toEqual({ version: 1, collections: [{ name: "prod", type: "namespace", patterns: ["prod-*"] }] });
  });

  /** An edit replaces what the editor shows and keeps what it does not. */
  it("replace a collection in place on an edit, renamed or not, keeping fields the editor does not show", () => {
    settingsStorage.setItem(
      COLLECTIONS_KEY,
      JSON.stringify({ version: 1, collections: [{ name: "prod", type: "namespace", patterns: ["prod-*"], clusters: ["eu"] }] }),
    );
    __resetCollectionsForTests();
    saveCollection({ name: "production", type: "namespace", patterns: ["prod-*", "live-*"] }, "prod");
    expect(getCollections()).toEqual([
      { name: "production", type: "namespace", patterns: ["prod-*", "live-*"], clusters: ["eu"] },
    ]);
  });

  /**
   * The editor leaves an emptied field out of what it saves. Merged over the
   * old collection, that kept the old value: a cleared pattern still matched.
   */
  it("clear a field the editor cleared, and keep when it was last used", async () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"], labelSelector: "env=prod", members: ["a"], description: "P" });
    markCollectionsUsed("namespace", ["prod"], 7);
    saveCollection({ name: "prod", type: "namespace", members: ["a"] }, "prod");
    expect(getCollections()).toEqual([{ name: "prod", type: "namespace", members: ["a"], lastUsedAt: 7 }]);
    await collectionsSaved();
    __resetCollectionsForTests();
    expect(getCollections()).toEqual([{ name: "prod", type: "namespace", members: ["a"], lastUsedAt: 7 }]);
  });

  it("drop a resource filter the editor switched off (#885)", () => {
    saveCollection({ name: "prod", type: "namespace", resources: { kinds: ["Service"], nameRegex: "^a" } });
    saveCollection({ name: "prod", type: "namespace" }, "prod");
    expect(getCollections()).toEqual([{ name: "prod", type: "namespace" }]);
  });

  it("tell their listeners when a collection is renamed, and not when it is only edited", () => {
    const heard = vi.fn();
    onCollectionRenamed(heard);
    saveCollection({ name: "prod", type: "namespace" });
    saveCollection({ name: "prod", type: "namespace", patterns: ["p-*"] }, "prod");
    expect(heard).not.toHaveBeenCalled();
    saveCollection({ name: "production", type: "namespace" }, "prod");
    expect(heard).toHaveBeenCalledExactlyOnceWith("namespace", "prod", "production");
  });

  it("delete several at once, of one type only", () => {
    saveCollection({ name: "a", type: "namespace" });
    saveCollection({ name: "b", type: "namespace" });
    saveCollection({ name: "c", type: "namespace" });
    saveCollection({ name: "a", type: "resource", kinds: ["apps/Deployment"] });
    deleteCollections("namespace", ["a", "c"]);
    expect(getCollections().map((c) => `${c.type}:${c.name}`)).toEqual(["namespace:b", "resource:a"]);
  });

  it("order a type's collections by when a picker last used them, then by name", () => {
    saveCollection({ name: "b", type: "namespace" });
    saveCollection({ name: "a", type: "namespace" });
    saveCollection({ name: "c", type: "namespace" });
    saveCollection({ name: "z", type: "resource", kinds: ["Service"] });
    markCollectionsUsed("namespace", ["c"], 100);
    markCollectionsUsed("namespace", ["b"], 200);
    expect(byRecentUse(getCollections(), "namespace").map((c) => c.name)).toEqual(["b", "c", "a"]);
  });

  /**
   * Each write starts from the stored document, which a refused write never
   * reached: the second of two changes dropped the first.
   */
  it("keep every change for the session when storage refuses them all", async () => {
    const original = settingsStorage.setItem;
    settingsStorage.setItem = () => {
      throw new Error("Settings backend is unavailable");
    };
    try {
      saveCollection({ name: "prod", type: "namespace" });
      saveCollection({ name: "staging", type: "namespace" });
      await collectionsSaved();
      markCollectionsUsed("namespace", ["prod"], 3);
      await collectionsSaved();
    } finally {
      settingsStorage.setItem = original;
    }
    expect(getCollections().map((c) => [c.name, c.lastUsedAt])).toEqual([
      ["prod", 3],
      ["staging", undefined],
    ]);
    // And the next write that is accepted carries them all.
    saveCollection({ name: "dev", type: "namespace" });
    await collectionsSaved();
    __resetCollectionsForTests();
    expect(getCollections().map((c) => c.name)).toEqual(["prod", "staging", "dev"]);
  });

  it("hold a change for the session when storage refuses it", () => {
    const original = settingsStorage.setItem;
    settingsStorage.setItem = () => {
      throw new Error("Settings backend is unavailable");
    };
    try {
      saveCollection({ name: "prod", type: "namespace" });
    } finally {
      settingsStorage.setItem = original;
    }
    expect(getCollections().map((c) => c.name)).toEqual(["prod"]);
  });
});

/**
 * Each window keeps its own copy of the document and used to write it whole,
 * so a window that had loaded the collections before another added one put
 * its old copy back — the other window's collection, gone.
 */
describe("a change made in another window", () => {
  it("is not undone by saving, deleting or merely using a collection here", async () => {
    saveCollection({ name: "prod", type: "namespace" });
    await collectionsSaved();

    anotherWindowSaves([{ name: "prod", type: "namespace" }, { name: "staging", type: "namespace" }]);
    saveCollection({ name: "dev", type: "namespace" });
    await collectionsSaved();
    expect(getCollections().map((c) => c.name)).toEqual(["prod", "staging", "dev"]);

    anotherWindowSaves([{ name: "prod", type: "namespace" }, { name: "staging", type: "namespace" }, { name: "dev", type: "namespace" }, { name: "qa", type: "namespace" }]);
    markCollectionsUsed("namespace", ["prod"], 5);
    await collectionsSaved();
    expect(getCollections().map((c) => c.name)).toEqual(["prod", "staging", "dev", "qa"]);
    expect(getCollections()[0].lastUsedAt).toBe(5);

    anotherWindowSaves([{ name: "prod", type: "namespace" }, { name: "qa", type: "namespace" }, { name: "uat", type: "namespace" }]);
    deleteCollections("namespace", ["prod"]);
    await collectionsSaved();
    __resetCollectionsForTests();
    expect(getCollections().map((c) => c.name)).toEqual(["qa", "uat"]);
  });

  it("shows once this window reads the setting again", async () => {
    getCollections();
    anotherWindowSaves([{ name: "staging", type: "namespace" }]);
    await refreshCollections();
    expect(getCollections().map((c) => c.name)).toEqual(["staging"]);
  });

  it("does not cost this window its change when the setting cannot be read again", async () => {
    refreshSetting.mockRejectedValueOnce(new Error("backend gone"));
    saveCollection({ name: "prod", type: "namespace" });
    await collectionsSaved();
    __resetCollectionsForTests();
    expect(getCollections().map((c) => c.name)).toEqual(["prod"]);
  });
});
