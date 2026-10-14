import { beforeEach, describe, expect, it } from "vitest";
import { settingsStorage } from "@srelens/core";
import {
  COLLECTIONS_KEY,
  __resetCollectionsForTests,
  byRecentUse,
  deleteCollections,
  getCollections,
  markCollectionsUsed,
  saveCollection,
} from "./collections";

const stored = () => JSON.parse(settingsStorage.getItem(COLLECTIONS_KEY) ?? "null");

beforeEach(() => {
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
});

describe("the saved collections", () => {
  it("start as none", () => {
    expect(getCollections()).toEqual([]);
  });

  it("keep a saved collection across a restart, in a versioned document", () => {
    saveCollection({ name: "prod", type: "namespace", patterns: ["prod-*"] });
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
