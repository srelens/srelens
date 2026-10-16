import { describe, expect, it, vi } from "vitest";
import {
  apiKindName,
  filtersKind,
  listApiKinds,
  matchCollectionNames,
  parseCollectionsDocument,
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

describe("a collection's resource filter (#885)", () => {
  const prod: Collection = {
    name: "prod",
    type: "namespace",
    patterns: ["prod-*"],
    resources: { kinds: ["apps/Deployment", "Service"], nameRegex: "^payment-" },
  };

  it("is read from the stored document, and dropped when it is not the shape this build reads", () => {
    const doc = parseCollectionsDocument({
      version: 1,
      collections: [
        prod,
        { name: "bad", type: "namespace", resources: { kinds: ["Service"] } },
        { name: "odd", type: "namespace", resources: { kinds: [3, "Service"], nameRegex: "x" } },
      ],
    });
    expect(doc.collections[0].resources).toEqual(prod.resources);
    expect(doc.collections[1].resources).toBeUndefined();
    expect(doc.collections[2].resources).toEqual({ kinds: ["Service"], nameRegex: "x" });
  });

  it("is sent to the backend with the rest of the collection", async () => {
    const invoke = vi.fn().mockResolvedValue({ problems: [] });
    await validateCollection(prod, [], invoke);
    expect(invoke.mock.calls[0][1].collection.resources).toEqual(prod.resources);
  });

  it("matches a list's names through the backend, with the kind's group only for a custom kind", async () => {
    const invoke = vi.fn().mockResolvedValue({ filtered: true, matches: [true, false], problems: [] });
    await matchCollectionNames([prod], { kind: "Deployment" }, ["payment-service", "order-service"], invoke);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("collections.matchNames", {
      collections: [expect.objectContaining({ name: "prod", resources: prod.resources })],
      kind: "Deployment",
      names: ["payment-service", "order-service"],
    });
    await matchCollectionNames([prod], { kind: "Widget", group: "acme.io" }, [], invoke);
    expect(invoke.mock.calls[1][1]).toMatchObject({ kind: "Widget", group: "acme.io" });
  });

  it("names a built-in kind by its name and a custom kind by its group too", () => {
    expect(filtersKind(prod, { kind: "Deployment" })).toBe(true);
    expect(filtersKind(prod, { kind: "Service" })).toBe(true);
    expect(filtersKind(prod, { kind: "ConfigMap" })).toBe(false);
    expect(filtersKind(prod, { kind: "Deployment", group: "acme.io" })).toBe(false);
    expect(filtersKind({ name: "x", type: "namespace" }, { kind: "Deployment" })).toBe(false);
  });

  it("lists the kinds a cluster serves, named as a filter names them", async () => {
    const kinds = [
      { group: "apps", version: "v1", kind: "Deployment", plural: "deployments", namespaced: true },
      { group: "", version: "v1", kind: "Service", plural: "services", namespaced: true },
    ];
    const invoke = vi.fn().mockResolvedValue({ kinds });
    expect(await listApiKinds("kind-dev", invoke)).toEqual(kinds);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("k8s.listApiKinds", { context: "kind-dev" });
    expect(kinds.map(apiKindName)).toEqual(["apps/Deployment", "Service"]);
  });
});
