import { describe, expect, it } from "vitest";
import { MAX_PREDICATES, resolvePath, unmetPredicate, type ActionPredicate } from "./actionPredicates";

// The Rust half is crates/capability/src/predicate.rs, whose own tests state
// the same cases. A surface that answered differently from the host would
// offer a control the host refuses, or hide one it would have accepted.

const resource = {
  apiVersion: "helm.toolkit.fluxcd.io/v2",
  kind: "HelmRelease",
  metadata: { name: "api", namespace: "team", annotations: { "acme.io/pinned": "yes" } },
  spec: { suspend: true, chart: { spec: { version: "1.2.3" } } },
  status: { conditions: [{ type: "Ready", status: "False" }] },
};

const predicate = (p: Partial<ActionPredicate>): ActionPredicate =>
  ({ jsonPath: ".spec.suspend", reason: "r", ...p }) as ActionPredicate;

describe("actionPredicates", () => {
  it("holds equals only for that literal at that path", () => {
    const suspended = [predicate({ equals: true })];
    expect(unmetPredicate(suspended, resource)).toBeUndefined();
    expect(unmetPredicate(suspended, { spec: { suspend: false } })).toBe(suspended[0]);
    expect(unmetPredicate(suspended, { spec: {} })).toBe(suspended[0]);
    expect(unmetPredicate(suspended, { spec: { suspend: "true" } })).toBe(suspended[0]);
  });

  it("holds notEquals when the value differs or was never set", () => {
    const live = [predicate({ notEquals: true })];
    expect(unmetPredicate(live, resource)).toBe(live[0]);
    expect(unmetPredicate(live, { spec: { suspend: false } })).toBeUndefined();
    expect(unmetPredicate(live, { spec: {} })).toBeUndefined();
  });

  it("reads null as unset for present and absent", () => {
    const running = [predicate({ jsonPath: ".operation", present: true })];
    const idle = [predicate({ jsonPath: ".operation", absent: true })];
    expect(unmetPredicate(running, { operation: { sync: {} } })).toBeUndefined();
    expect(unmetPredicate(idle, { operation: { sync: {} } })).toBe(idle[0]);
    for (const without of [{}, { operation: null }]) {
      expect(unmetPredicate(idle, without)).toBeUndefined();
      expect(unmetPredicate(running, without)).toBe(running[0]);
    }
  });

  it("addresses quoted keys and list elements", () => {
    expect(resolvePath(resource, ".metadata.annotations['acme.io/pinned']")).toBe("yes");
    expect(resolvePath(resource, ".status.conditions[0].status")).toBe("False");
    expect(resolvePath(resource, "$.spec.chart.spec.version")).toBe("1.2.3");
    expect(resolvePath(resource, ".spec.missing")).toBeUndefined();
    expect(resolvePath(resource, ".status.conditions[9]")).toBeUndefined();
  });

  it("fails closed on a predicate the host would not evaluate", () => {
    // The host refuses these at install, so a surface should never see one —
    // and if it does, it must not read it as "the condition is met".
    for (const broken of [
      predicate({ jsonPath: "spec.suspend", present: true }),
      predicate({ jsonPath: ".status.conditions[?(@.type=='Ready')].status", equals: "True" }),
      predicate({ jsonPath: ".spec.*", present: true }),
      predicate({ equals: { suspend: true } as unknown as boolean }),
      predicate({}),
      predicate({ equals: true, present: true }),
      predicate({ jsonPath: ".spec.suspend", equals: true, reason: " " }),
    ]) {
      expect(unmetPredicate([broken], resource), `${JSON.stringify(broken)}`).toBe(broken);
    }
  });

  it("bounds how many predicates one list may hold", () => {
    const many = Array.from({ length: MAX_PREDICATES + 1 }, () => predicate({ equals: true }));
    expect(unmetPredicate(many, resource)).toBe(many[0]);
  });

  it("reports the first failing predicate, per resource", () => {
    const declared = [
      predicate({ jsonPath: ".operation", absent: true, reason: "A sync is already running" }),
      predicate({ notEquals: true, reason: "Resume this resource before requesting reconciliation" }),
    ];
    expect(unmetPredicate(declared, resource)?.reason).toBe(
      "Resume this resource before requesting reconciliation",
    );
    // Per resource, so a list view can count how much of a selection applies.
    const rows = [resource, { spec: {} }, { operation: {} }];
    expect(rows.filter((row) => !unmetPredicate(declared, row)).length).toBe(1);
  });

  it("holds an empty list", () => {
    expect(unmetPredicate([], resource)).toBeUndefined();
    expect(unmetPredicate(undefined, resource)).toBeUndefined();
  });
});
