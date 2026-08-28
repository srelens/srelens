import { describe, it, expect } from "vitest";
import { hasSelector, requirementText, selectorOf } from "./workloadSelector";

/** A workload object whose selector carries whatever halves a case needs. */
const workload = (selector: unknown) => ({ spec: { selector } });

describe("selectorOf", () => {
  it("reads both halves of a selector", () => {
    expect(
      selectorOf(
        workload({
          matchLabels: { app: "web" },
          matchExpressions: [{ key: "track", operator: "NotIn", values: ["canary"] }],
        }),
      ),
    ).toEqual({
      matchLabels: { app: "web" },
      matchExpressions: [{ key: "track", operator: "NotIn", values: ["canary"] }],
    });
  });

  it("reads a selector written entirely in expressions", () => {
    // The half that used to vanish: `matchLabels` is absent, and reading it
    // alone yields `{}` — which the backend answers with no pods on purpose.
    const selector = selectorOf(workload({ matchExpressions: [{ key: "app", operator: "Exists" }] }));
    expect(selector.matchLabels).toEqual({});
    expect(selector.matchExpressions).toEqual([{ key: "app", operator: "Exists", values: [] }]);
  });

  it("keeps a requirement's operator verbatim, however it was spelled", () => {
    // "notin" is not one of the four operators; the backend refuses what it
    // cannot render, and an error beats a corrected selector — which is
    // simply a different selector, naming different pods.
    const selector = selectorOf(
      workload({ matchExpressions: [{ key: "track", operator: "notin", values: ["canary"] }] }),
    );
    expect(selector.matchExpressions).toEqual([
      { key: "track", operator: "notin", values: ["canary"] },
    ]);
  });

  it("keeps only the strings among a requirement's values", () => {
    const selector = selectorOf(
      workload({ matchExpressions: [{ key: "tier", operator: "In", values: ["web", 7, null] }] }),
    );
    expect(selector.matchExpressions[0].values).toEqual(["web"]);
  });

  it("yields two empty halves for a spec with no selector at all", () => {
    expect(selectorOf({ spec: {} })).toEqual({ matchLabels: {}, matchExpressions: [] });
    expect(selectorOf(undefined)).toEqual({ matchLabels: {}, matchExpressions: [] });
  });
});

describe("hasSelector", () => {
  it("counts a selector of expressions alone as a selector", () => {
    // The gate the Pods panel hangs on. Reading `matchLabels` alone did not
    // merely empty that panel for such a workload — it removed it.
    expect(hasSelector({ matchLabels: {}, matchExpressions: [{ key: "app", operator: "Exists" }] })).toBe(
      true,
    );
  });

  it("counts a selector of equality labels alone as a selector", () => {
    expect(hasSelector({ matchLabels: { app: "web" }, matchExpressions: [] })).toBe(true);
  });

  it("says no only when both halves are empty", () => {
    expect(hasSelector({ matchLabels: {}, matchExpressions: [] })).toBe(false);
  });
});

describe("requirementText", () => {
  it("writes each operator the way Kubernetes writes it", () => {
    expect(requirementText({ key: "app", operator: "In", values: ["web", "api"] })).toBe(
      "app in (web, api)",
    );
    expect(requirementText({ key: "track", operator: "NotIn", values: ["canary"] })).toBe(
      "track notin (canary)",
    );
    expect(requirementText({ key: "logging", operator: "Exists" })).toBe("logging");
    expect(requirementText({ key: "legacy", operator: "DoesNotExist" })).toBe("!legacy");
  });

  it("prints an operator it does not know as it was spelled, rather than dropping the row", () => {
    // The row must show what the object says, including the part the cluster
    // will refuse — the same rule the reader follows.
    // Deliberately not "notin": that spelling would read the same as the
    // operator this row does NOT have, and a case that cannot tell the two
    // apart proves nothing about which branch ran.
    expect(requirementText({ key: "track", operator: "NOTIN", values: ["canary"] })).toBe(
      "track NOTIN (canary)",
    );
    expect(requirementText({ key: "track", operator: "Nonsense" })).toBe("track Nonsense");
  });
});
