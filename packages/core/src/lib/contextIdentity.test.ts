import { describe, expect, it } from "vitest";
import {
  mergeFromNames,
  mergeOrderFromNames,
  remapTabsToContexts,
  migrateOrder,
  migrateRecordKeys,
  projectOrderToNames,
  projectToNames,
  resolveStoredKey,
  unprefixedName,
  type ContextIdentity,
} from "./contextIdentity";

const ctx = (name: string, stableId: string): ContextIdentity => ({ name, stableId });

describe("unprefixedName", () => {
  it("strips the disambiguating file prefix", () => {
    expect(unprefixedName("prod-kube/M01")).toBe("M01");
    expect(unprefixedName("M01")).toBe("M01");
  });

  it("keeps only the last segment, since context names may contain slashes", () => {
    // OpenShift context names look like default/api-example-com:6443/user.
    expect(unprefixedName("file/a/b")).toBe("b");
  });
});

describe("resolveStoredKey", () => {
  const contexts = [ctx("prod-kube/M01", "/k/prod.yaml#M01"), ctx("k8s02", "/k/other.yaml#k8s02")];

  it("matches a key written under the current display name", () => {
    expect(resolveStoredKey("k8s02", contexts)?.stableId).toBe("/k/other.yaml#k8s02");
  });

  it("recovers a key written BEFORE the rename happened", () => {
    // The whole point: the user configured "M01" while it was still called
    // that, then a second file renamed it to "prod-kube/M01".
    expect(resolveStoredKey("M01", contexts)?.stableId).toBe("/k/prod.yaml#M01");
  });

  it("refuses to guess when several contexts share the original name", () => {
    // Both files declare M01, so which one the setting belonged to is
    // genuinely unknowable — handing it to either would be inventing data.
    const ambiguous = [
      ctx("prod-kube/M01", "/k/prod.yaml#M01"),
      ctx("dev-kube/M01", "/k/dev.yaml#M01"),
    ];
    expect(resolveStoredKey("M01", ambiguous)).toBeNull();
  });

  it("returns null for a context that is not present", () => {
    expect(resolveStoredKey("nothing-like-this", contexts)).toBeNull();
  });
});

describe("migrateRecordKeys", () => {
  const contexts = [ctx("prod-kube/M01", "/k/prod.yaml#M01"), ctx("k8s02", "/k/other.yaml#k8s02")];

  it("rekeys settings onto the stable id", () => {
    const { migrated, changed } = migrateRecordKeys(
      { M01: { shortName: "M1", color: "#f00" } },
      contexts,
    );
    expect(changed).toBe(true);
    expect(migrated).toEqual({ "/k/prod.yaml#M01": { shortName: "M1", color: "#f00" } });
  });

  it("leaves already-migrated entries alone and reports no change", () => {
    const stored = { "/k/prod.yaml#M01": { shortName: "M1" } };
    const { migrated, changed } = migrateRecordKeys(stored, contexts);
    expect(changed).toBe(false);
    expect(migrated).toEqual(stored);
  });

  it("keeps settings for a context that is not currently connected", () => {
    // Disconnecting a cluster must not discard its identity.
    const { migrated, changed } = migrateRecordKeys({ "some-offline-ctx": { color: "#00f" } }, contexts);
    expect(changed).toBe(false);
    expect(migrated).toEqual({ "some-offline-ctx": { color: "#00f" } });
  });

  it("never lets a legacy key overwrite an existing stable-id entry", () => {
    // The id-keyed value was written under the new scheme, so it is newer.
    const { migrated } = migrateRecordKeys(
      { M01: { shortName: "old" }, "/k/prod.yaml#M01": { shortName: "new" } },
      contexts,
    );
    expect(migrated["/k/prod.yaml#M01"]).toEqual({ shortName: "new" });
    expect(migrated.M01).toEqual({ shortName: "old" });
  });
});

describe("migrateOrder", () => {
  const contexts = [ctx("prod-kube/M01", "/k/prod.yaml#M01"), ctx("k8s02", "/k/other.yaml#k8s02")];

  it("rekeys while preserving position", () => {
    const { migrated, changed } = migrateOrder(["k8s02", "M01"], contexts);
    expect(changed).toBe(true);
    expect(migrated).toEqual(["/k/other.yaml#k8s02", "/k/prod.yaml#M01"]);
  });

  it("keeps unrecognized entries so the hotbar does not silently reorder", () => {
    const { migrated } = migrateOrder(["offline-ctx", "k8s02"], contexts);
    expect(migrated).toEqual(["offline-ctx", "/k/other.yaml#k8s02"]);
  });

  it("does not duplicate when both the old and new key are present", () => {
    const { migrated } = migrateOrder(["/k/prod.yaml#M01", "M01"], contexts);
    expect(migrated).toEqual(["/k/prod.yaml#M01"]);
  });
});

describe("projectToNames / mergeFromNames", () => {
  const contexts = [ctx("prod-kube/M01", "/k/prod.yaml#M01"), ctx("k8s02", "/k/other.yaml#k8s02")];

  it("renders by display name while storing by identity", () => {
    const byId = { "/k/prod.yaml#M01": { color: "#f00" } };
    expect(projectToNames(byId, contexts)).toEqual({ "prod-kube/M01": { color: "#f00" } });
  });

  it("keeps settings for disconnected contexts when writing back", () => {
    // The projection can't contain an offline cluster, so a naive replace
    // would delete it. This is the case that would silently destroy config.
    const byId = { "/k/prod.yaml#M01": { color: "#f00" }, "/k/offline.yaml#X": { color: "#0f0" } };
    const edited = { "prod-kube/M01": { color: "#00f" } };
    const merged = mergeFromNames(byId, edited, contexts);
    expect(merged["/k/prod.yaml#M01"]).toEqual({ color: "#00f" });
    // The case that would silently destroy config:
    expect(merged["/k/offline.yaml#X"]).toEqual({ color: "#0f0" });
  });

  it("treats a removed CONNECTED context as a real deletion", () => {
    const byId = { "/k/prod.yaml#M01": { color: "#f00" }, "/k/other.yaml#k8s02": { color: "#0f0" } };
    const merged = mergeFromNames(byId, { "prod-kube/M01": { color: "#f00" } }, contexts);
    expect("/k/other.yaml#k8s02" in merged).toBe(false);
  });

  it("survives a rename round trip", () => {
    // Same context, now renamed by a collision — its settings still resolve.
    const byId = { "/k/prod.yaml#M01": { shortName: "M1" } };
    const renamed = [ctx("prod-kube/M01", "/k/prod.yaml#M01")];
    expect(projectToNames(byId, renamed)).toEqual({ "prod-kube/M01": { shortName: "M1" } });
  });
});

describe("order projection", () => {
  const contexts = [ctx("prod-kube/M01", "/k/prod.yaml#M01"), ctx("k8s02", "/k/other.yaml#k8s02")];

  it("shows only connected contexts, by display name", () => {
    const byId = ["/k/other.yaml#k8s02", "/k/offline.yaml#X", "/k/prod.yaml#M01"];
    expect(projectOrderToNames(byId, contexts)).toEqual(["k8s02", "prod-kube/M01"]);
  });

  it("remembers where a disconnected context sat when the hotbar is reordered", () => {
    // The UI never saw the offline entry, so a naive write would forget it.
    const byId = ["/k/other.yaml#k8s02", "/k/offline.yaml#X", "/k/prod.yaml#M01"];
    const merged = mergeOrderFromNames(byId, ["prod-kube/M01", "k8s02"], contexts);
    expect(merged).toEqual(["/k/prod.yaml#M01", "/k/other.yaml#k8s02", "/k/offline.yaml#X"]);
  });
});

describe("order deletion", () => {
  const contexts = [ctx("prod-kube/M01", "/k/prod.yaml#M01"), ctx("k8s02", "/k/other.yaml#k8s02")];

  it("removes a connected context the caller dropped", () => {
    // Deleting a context filters it out of the name list. Treating that as
    // "offline" and re-appending made deletion impossible.
    const byId = ["/k/prod.yaml#M01", "/k/other.yaml#k8s02"];
    const merged = mergeOrderFromNames(byId, ["prod-kube/M01"], contexts);
    expect(merged).toEqual(["/k/prod.yaml#M01"]);
    expect(merged).not.toContain("/k/other.yaml#k8s02");
  });
});

describe("remapTabsToContexts", () => {
  it("follows its context through the rename a collision causes", () => {
    // THE reported case: a second file declares M01, so BOTH contexts gain a
    // prefix. Name matching cannot resolve that — identity can.
    const before = [{ cluster: "M01", clusterId: "/k/prod.yaml#M01" }];
    const afterCollision = [
      ctx("prod/M01", "/k/prod.yaml#M01"),
      ctx("dev/M01", "/k/dev.yaml#M01"),
    ];
    expect(remapTabsToContexts(before, afterCollision)).toEqual([
      { cluster: "prod/M01", clusterId: "/k/prod.yaml#M01" },
    ]);
  });

  it("adopts an id for a tab restored from before identities existed", () => {
    const legacy = [{ cluster: "k8s02" }];
    const contexts = [ctx("k8s02", "/k/other.yaml#k8s02")];
    expect(remapTabsToContexts(legacy, contexts)).toEqual([
      { cluster: "k8s02", clusterId: "/k/other.yaml#k8s02" },
    ]);
  });

  it("leaves a genuinely removed context alone, for the prune to handle", () => {
    const tabs = [{ cluster: "gone", clusterId: "/k/gone.yaml#gone" }];
    expect(remapTabsToContexts(tabs, [ctx("k8s02", "/k/other.yaml#k8s02")])).toEqual(tabs);
  });

  it("returns the same objects when nothing moved", () => {
    const contexts = [ctx("k8s02", "/k/other.yaml#k8s02")];
    const tabs = [{ cluster: "k8s02", clusterId: "/k/other.yaml#k8s02" }];
    expect(remapTabsToContexts(tabs, contexts)[0]).toBe(tabs[0]);
  });

  it("ignores cluster-less tabs", () => {
    const tabs = [{ cluster: null }];
    expect(remapTabsToContexts(tabs, [])).toEqual(tabs);
  });
});
