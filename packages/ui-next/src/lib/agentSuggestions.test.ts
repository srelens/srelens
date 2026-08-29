// jsdom, not node: this reaches `routes.ts`, whose graph reaches the
// Terminals screen through `screenFor` and `@xterm/addon-fit`, a UMD bundle
// that reads `self` while it evaluates — see `routes.test.ts` for the same
// note against the same import.
import { describe, it, expect } from "vitest";
import { suggestionsFor, contextLabelFor } from "./agentSuggestions";

describe("what the dock offers to ask", () => {
  it("offers log questions on a logs route", () => {
    expect(suggestionsFor("/logs")).toContain("Summarise the last 500 lines");
  });

  it("offers helm questions on helm", () => {
    expect(suggestionsFor("/helm")).toContain("What did release 119 change?");
  });

  it("offers resource questions on a detail route", () => {
    expect(suggestionsFor("/k/Pod/checkout/api-0")).toContain("Why is this workload degraded?");
  });

  it("offers resource questions on a legacy /resources/<name> tab too", () => {
    // A tab a previous session persisted at the pre-`/k/` shape is still a
    // resource — `describe` gives it `kind: "resource"` for exactly this
    // reason (see `routes.ts`'s legacy branch) — so it must not fall through
    // to the fallback set just because `parseDetailRoute` cannot parse it.
    expect(suggestionsFor("/resources/api-0")).toContain("Why is this workload degraded?");
  });

  it("does not mistake the browsed workloads list for a single resource", () => {
    // `/resources` (no name segment) is the LIST screen, `kind: "workloads"`
    // — the guard against over-correcting the fix above into a literal
    // `/resources` prefix match, which would wrongly claim this route too.
    expect(suggestionsFor("/resources")).toEqual(suggestionsFor("/settings"));
  });

  it("offers incident questions on the control room", () => {
    expect(suggestionsFor("/")).toContain("Why is checkout-api returning 5xx?");
  });

  it("falls back to cluster-wide questions on a route with no subject", () => {
    expect(suggestionsFor("/settings")).toContain("What is unhealthy right now?");
  });

  it("falls back rather than returning nothing for an unrecognised route", () => {
    // A missing reading is absent, never a placeholder: an unknown route
    // still gets a real, non-empty set of suggestions.
    expect(suggestionsFor("/some-route-nobody-minted")).toEqual(suggestionsFor("/settings"));
    expect(suggestionsFor("/some-route-nobody-minted").length).toBeGreaterThan(0);
  });
});

describe("what the dock says it is asking about", () => {
  it("names the resource on a detail route", () => {
    expect(contextLabelFor("/k/Pod/checkout/api-0", "prod-eu")).toBe("prod-eu / api-0");
  });

  it("names the screen where there is no resource", () => {
    expect(contextLabelFor("/helm", "prod-eu")).toBe("prod-eu / helm");
  });

  it("is just the cluster when the route is the control room", () => {
    expect(contextLabelFor("/", "prod-eu")).toBe("prod-eu");
  });

  it("says nothing rather than a bare separator when there is no cluster", () => {
    expect(contextLabelFor("/helm", "")).toBe("helm");
  });

  it("says nothing rather than a bare separator when there is no subject either", () => {
    expect(contextLabelFor("/", "")).toBe("");
  });
});
