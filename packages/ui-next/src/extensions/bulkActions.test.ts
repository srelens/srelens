import { describe, expect, it } from "vitest";
import type { BulkOutcome } from "@srelens/core";
import {
  bulkActionResult,
  bulkApplicability,
  bulkResourceKey,
  type BulkResource,
} from "./bulkActions";

const many = (count: number): BulkResource[] =>
  Array.from({ length: count }, (_, i) => ({ namespace: "team", name: `app-${i}` }));

describe("bulkResourceKey", () => {
  it("keys a namespaced resource by namespace and name, a cluster-scoped one by name", () => {
    expect(bulkResourceKey({ namespace: "team", name: "apps" })).toBe("team/apps");
    expect(bulkResourceKey({ namespace: "", name: "node-a" })).toBe("node-a");
  });
});

describe("bulkApplicability", () => {
  it("says how many of the selection the action applies to", () => {
    const selection = many(12);
    const applicability = bulkApplicability(selection, (r) => !["app-0", "app-1", "app-2"].includes(r.name));
    expect(applicability.total).toBe(12);
    expect(applicability.applicable.map((r) => r.name)).not.toContain("app-0");
    expect(applicability.applicable).toHaveLength(9);
    expect(applicability.note).toBe("applies to 9 of 12");
  });

  it("says nothing when the action applies to every selected resource", () => {
    // The seam's default until #550 lands: the host has no predicate to run,
    // so it does not claim the action is unavailable anywhere.
    const applicability = bulkApplicability(many(3));
    expect(applicability.applicable).toHaveLength(3);
    expect(applicability.note).toBeNull();
  });

  it("says it applies to none rather than leaving the count out", () => {
    expect(bulkApplicability(many(2), () => false).note).toBe("applies to 0 of 2");
  });
});

const outcomes = (...items: BulkOutcome<BulkResource>[]) => items;
const at = (name: string): BulkResource => ({ namespace: "team", name });

describe("bulkActionResult", () => {
  it("reports a run the cluster accepted whole as a success", () => {
    const result = bulkActionResult(
      outcomes({ item: at("a"), status: "ok" }, { item: at("b"), status: "ok" }),
    );
    expect(result).toEqual({ status: "success", succeeded: ["team/a", "team/b"], failed: [], cancelled: [] });
  });

  it("reports a mixed run as partial, never as success, with a reason per resource", () => {
    const result = bulkActionResult(
      outcomes(
        { item: at("a"), status: "ok" },
        { item: at("b"), status: "error", error: "admission webhook denied the request" },
      ),
    );
    expect(result.status).toBe("partial");
    expect(result.succeeded).toEqual(["team/a"]);
    expect(result.failed).toEqual([
      { resource: "team/b", reason: "admission webhook denied the request" },
    ]);
  });

  it("names a rejection that carried no reason rather than dropping the row", () => {
    const result = bulkActionResult(
      outcomes({ item: at("a"), status: "error", error: "   " }, { item: at("b"), status: "error" }),
    );
    expect(result.status).toBe("failed");
    expect(result.failed).toEqual([
      { resource: "team/a", reason: "The cluster rejected the request without a reason." },
      { resource: "team/b", reason: "The cluster rejected the request without a reason." },
    ]);
  });

  it("reports a cancelled run as partial even when everything sent was accepted", () => {
    const result = bulkActionResult(
      outcomes(
        { item: at("a"), status: "ok" },
        { item: at("b"), status: "cancelled" },
        { item: at("c"), status: "cancelled" },
      ),
    );
    // Two of the three were never requested. That is not a success.
    expect(result.status).toBe("partial");
    expect(result.succeeded).toEqual(["team/a"]);
    expect(result.cancelled).toEqual(["team/b", "team/c"]);
    // A resource nobody asked the cluster about is not a resource the cluster refused.
    expect(result.failed).toEqual([]);
  });

  it("reports a run where nothing was accepted as failed", () => {
    const result = bulkActionResult(
      outcomes({ item: at("a"), status: "error", error: "forbidden" }, { item: at("b"), status: "cancelled" }),
    );
    expect(result.status).toBe("failed");
  });
});
