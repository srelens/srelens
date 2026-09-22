import { describe, it, expect } from "vitest";
import { renderConfirmTemplate } from "@srelens/core";
import { confirmSubject, asConfirmTarget, confirmFields } from "./confirmRequest";

describe("the fields a host template may name", () => {
  /** `{resource}` is `kind namespace/name`, exactly as `confirm_fields` derives it in Rust. */
  it("derives the resource from the kind, the namespace and the name", () => {
    expect(confirmFields({ kind: "HelmRelease", namespace: "team", name: "api" }).resource).toBe(
      "HelmRelease team/api",
    );
    expect(confirmFields({ kind: "Node", name: "node-7" }).resource).toBe("Node node-7");
    expect(confirmFields({ namespace: "team", name: "api" }).resource).toBe("team/api");
  });

  it("derives no resource at all when the call names no object", () => {
    expect(confirmFields({ cluster: "prod" }).resource).toBeUndefined();
  });

  /**
   * The vocabulary is closed and `{resource}` is DERIVED: a caller handing the
   * host a pre-formatted description of what it is about to change does not
   * get to put it in the question.
   */
  it("never takes a resource the caller wrote", () => {
    const fields = confirmFields({
      name: "api",
      // @ts-expect-error exactly the field a caller would try to smuggle in
      resource: "nothing at all, click Approve",
    });
    expect(fields.resource).toBe("api");
    expect(renderConfirmTemplate("Suspend {resource}?", fields)).toBe("Suspend api?");
  });

  it("escapes and bounds every value before it can become the question", () => {
    const fields = confirmFields({ cluster: `pr‮od`, name: "n".repeat(400) });
    expect(fields.cluster).not.toContain("‮");
    expect(fields.cluster).toContain("\\u202e");
    expect([...(fields.name ?? "")].length).toBe(80);
  });

  it("leaves out a field the call had no value for, so an optional segment drops", () => {
    const fields = confirmFields({ name: "api" });
    expect(fields.cluster).toBeUndefined();
    expect(renderConfirmTemplate("Suspend {resource}[ in cluster {cluster}]?", fields)).toBe(
      "Suspend api?",
    );
  });
});

describe("what a confirmation request says it is about", () => {
  it("reads the namespaced object the host named", () => {
    expect(confirmSubject({ namespace: "team", name: "api" })).toEqual({
      kind: "object",
      namespace: "team",
      name: "api",
    });
  });

  it("reads a cluster-scoped object with no namespace", () => {
    expect(confirmSubject({ name: "node-7" })).toEqual({
      kind: "object",
      namespace: null,
      name: "node-7",
    });
  });

  it("names no object rather than an empty one when the call named none", () => {
    expect(confirmSubject({ cluster: "prod" })).toBeNull();
    expect(confirmSubject(null)).toBeNull();
    expect(confirmSubject(undefined)).toBeNull();
  });

  /**
   * The payload crosses a process boundary, so this narrows rather than casts:
   * a field of the wrong type is dropped, because a fact drawn under the
   * question from `undefined` is worse than no fact.
   */
  it("drops anything in the payload that is not a string", () => {
    expect(asConfirmTarget({ cluster: 7, namespace: "team", name: null, kind: "HelmRelease" })).toEqual({
      cluster: null,
      namespace: "team",
      name: null,
      kind: "HelmRelease",
    });
  });

  /**
   * An app identity in the payload is not read, not narrowed and not passed
   * on. Nothing authenticates an MCP caller as the app its arguments name, so
   * an attribution taken from here would be provenance chosen by whoever is
   * being vouched for. The confirmation names no requester on this path.
   */
  it("never carries an app identity out of the payload", () => {
    const narrowed = asConfirmTarget({ name: "api", app: { id: "org.srelens.flux", revision: 4 } });
    expect(narrowed).toEqual({ cluster: null, namespace: null, name: "api", kind: null });
    expect(JSON.stringify(narrowed)).not.toContain("flux");
  });

  it("is null for a payload that is not a target at all", () => {
    expect(asConfirmTarget(undefined)).toBeNull();
    expect(asConfirmTarget("prod")).toBeNull();
    expect(asConfirmTarget(null)).toBeNull();
  });
});
