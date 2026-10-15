import { describe, expect, it, vi } from "vitest";
import {
  collectionCoversKind,
  parseCollectionsDocument,
  resolveResourceCollections,
  parseMemberList,
  resolveNamespaceCollections,
  validateCollection,
  type Collection,
} from "./collections";

describe("parseCollectionsDocument", () => {
  it("is an empty document for nothing, junk and the wrong shape", () => {
    for (const raw of [null, "", "{not json", "[]", '{"collections":"x"}', 7]) {
      expect(parseCollectionsDocument(raw)).toEqual({ version: 1, collections: [] });
    }
  });

  /**
   * #885 asks that the loader tolerate what it does not know: a later build's
   * fields, on the document and on each collection, survive being read and
   * saved again by this one.
   */
  it("keeps fields it does not know, on the document and on each collection", () => {
    const doc = parseCollectionsDocument(
      JSON.stringify({
        version: 1,
        exportedAt: "2026-10-14",
        collections: [{ name: "prod", type: "namespace", patterns: ["prod-*"], clusters: ["eu"] }],
      }),
    );
    expect(doc.exportedAt).toBe("2026-10-14");
    expect(doc.collections[0]).toEqual({ name: "prod", type: "namespace", patterns: ["prod-*"], clusters: ["eu"] });
  });

  it("drops a collection with no name or no known type, and a field of the wrong shape, keeping the rest", () => {
    const doc = parseCollectionsDocument({
      version: 1,
      collections: [
        { name: "", type: "namespace" },
        { name: "x", type: "folder" },
        { type: "namespace" },
        "prod",
        { name: "ok", type: "namespace", members: ["a", 3, "b"], patterns: "prod-*", labelSelector: 4, lastUsedAt: "now" },
      ],
    });
    expect(doc.collections).toEqual([{ name: "ok", type: "namespace", members: ["a", "b"] }]);
  });
});

describe("parseMemberList", () => {
  it("splits a pasted list on commas, newlines and spaces, once each in order", () => {
    expect(parseMemberList("payment-service, order-service\nradar-app\r\n\n shipment-tracker ,order-service")).toEqual([
      "payment-service",
      "order-service",
      "radar-app",
      "shipment-tracker",
    ]);
    expect(parseMemberList("  \n , ")).toEqual([]);
  });
});

describe("the backend calls", () => {
  const prod: Collection = {
    name: "prod",
    type: "namespace",
    patterns: ["prod-*"],
    labelSelector: "  env=prod ",
    lastUsedAt: 5,
    clusters: ["eu"],
  };

  it("validates with the fields the backend reads, and the names it may not reuse", async () => {
    const invoke = vi.fn().mockResolvedValue({ problems: [{ field: "name", message: "taken" }] });
    const problems = await validateCollection(prod, ["staging"], invoke);
    expect(problems).toEqual([{ field: "name", message: "taken" }]);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("collections.validate", {
      collection: {
        name: "prod",
        description: "",
        type: "namespace",
        kinds: [],
        members: [],
        patterns: ["prod-*"],
        labelSelector: "env=prod",
      },
      taken: ["staging"],
    });
  });

  it("resolves against the listing, labels included, and says whether it is complete", async () => {
    const resolution = { namespaces: ["prod-eu"], collections: [], partial: false };
    const invoke = vi.fn().mockResolvedValue(resolution);
    const out = await resolveNamespaceCollections(
      [{ name: "picked", type: "namespace", members: ["a"] }],
      [{ name: "prod-eu", labels: { env: "prod" } }, { name: "dev" }],
      false,
      invoke,
    );
    expect(out).toBe(resolution);
    const [id, input] = invoke.mock.calls[0];
    expect(id).toBe("collections.resolveNamespaces");
    expect(input).toEqual({
      collections: [{ name: "picked", description: "", type: "namespace", kinds: [], members: ["a"], patterns: [] }],
      namespaces: [
        { name: "prod-eu", labels: { env: "prod" } },
        { name: "dev", labels: {} },
      ],
      complete: false,
    });
  });
});

describe("resource collections (#885)", () => {
  const release: Collection = { name: "release", type: "resource", kinds: ["apps/Deployment", "Service"], members: ["payment-service"] };

  it("resolve a built-in kind by its name, in the namespaces in view", async () => {
    const invoke = vi.fn().mockResolvedValue({ keys: [], collections: [], partial: false });
    await resolveResourceCollections("kind-dev", { kind: "Deployment" }, ["dev", "prod"], [release], invoke);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("collections.resolveResources", {
      context: "kind-dev",
      kind: "Deployment",
      namespaces: ["dev", "prod"],
      collections: [
        { name: "release", description: "", type: "resource", kinds: ["apps/Deployment", "Service"], members: ["payment-service"], patterns: [] },
      ],
    });
  });

  it("resolve a custom kind by its group, version and plural", async () => {
    const invoke = vi.fn().mockResolvedValue({ keys: [], collections: [], partial: false });
    await resolveResourceCollections("kind-dev", { kind: "IngressRoute", group: "traefik.io", version: "v1alpha1", plural: "ingressroutes" }, [], [], invoke);
    expect(invoke.mock.calls[0][1]).toMatchObject({ kind: "IngressRoute", group: "traefik.io", version: "v1alpha1", plural: "ingressroutes" });
  });

  it("cover a kind they list, by group for a custom kind and by name for a built-in one", () => {
    expect(collectionCoversKind(release, { kind: "Deployment" })).toBe(true);
    expect(collectionCoversKind(release, { kind: "Service" })).toBe(true);
    expect(collectionCoversKind(release, { kind: "ConfigMap" })).toBe(false);
    expect(collectionCoversKind(release, { kind: "Deployment", group: "acme.io" })).toBe(false);
    expect(collectionCoversKind({ ...release, kinds: ["acme.io/Deployment"] }, { kind: "Deployment", group: "acme.io" })).toBe(true);
    expect(collectionCoversKind({ ...release, type: "namespace" }, { kind: "Deployment" })).toBe(false);
  });
});
