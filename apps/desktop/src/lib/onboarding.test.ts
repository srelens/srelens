import { describe, expect, it, beforeEach } from "vitest";
import { emptyListMessage, loadOnboarded, saveOnboarded, shouldShowFirstRun } from "./onboarding";

beforeEach(() => localStorage.clear());

describe("the onboarded flag", () => {
  it("starts unset and sticks once saved", () => {
    expect(loadOnboarded()).toBe(false);
    saveOnboarded();
    expect(loadOnboarded()).toBe(true);
  });

  it("treats any other stored value as not yet onboarded", () => {
    localStorage.setItem("srelens.onboarded", "maybe");
    expect(loadOnboarded()).toBe(false);
  });
});

describe("shouldShowFirstRun", () => {
  it("shows once, for someone who has clusters to open", () => {
    expect(shouldShowFirstRun(false, 3)).toBe(true);
    expect(shouldShowFirstRun(true, 3)).toBe(false);
  });

  it("shows while the kubeconfig is still being read", () => {
    // Otherwise the card would flash in a moment after the page settles.
    expect(shouldShowFirstRun(false, null)).toBe(true);
  });

  it("stays out of the way when there are no contexts at all", () => {
    // That user has a more specific problem, and gets the connect-a-cluster
    // call to action instead of tips about the command palette.
    expect(shouldShowFirstRun(false, 0)).toBe(false);
  });
});

describe("emptyListMessage", () => {
  it("blames the search when there is one", () => {
    const m = emptyListMessage({ kind: "pods", query: "nginx", namespaces: ["default"] });
    expect(m.title).toBe("No pods match “nginx”");
    expect(m.hint).toMatch(/Clear the search/);
  });

  it("does not blame a selected search column, which narrows nothing on its own", () => {
    // Picking a column decides WHERE a search looks, not which rows survive:
    // with an empty query the list really is empty, and telling the user to
    // clear a filter sends them after something that cannot change the rows.
    const m = emptyListMessage({ kind: "pods", namespaces: [] });
    expect(m.title).toBe("No pods");
    expect(m.hint).toBe("This cluster has no pods you can see.");
  });

  it("names the namespace being looked at", () => {
    const m = emptyListMessage({ kind: "pods", namespaces: ["kube-system"] });
    expect(m.title).toBe("No pods in kube-system");
    expect(m.hint).toMatch(/Switch namespace/);
  });

  it("counts a multi-namespace scope rather than listing it", () => {
    expect(emptyListMessage({ kind: "pods", namespaces: ["a", "b", "c"] }).title).toBe(
      "No pods in 3 namespaces",
    );
  });

  it("ignores a whitespace-only search, as the table itself does", () => {
    const m = emptyListMessage({ kind: "pods", query: "   ", namespaces: [] });
    expect(m.title).toBe("No pods");
  });

  it("says the cluster is empty when nothing is narrowing the view", () => {
    // No search, no filter, all namespaces: the list really is empty, and the
    // hint must not send the user hunting for a filter that isn't set.
    const m = emptyListMessage({ kind: "pods", namespaces: [] });
    expect(m.title).toBe("No pods");
    expect(m.hint).toBe("This cluster has no pods you can see.");
  });

  it("ignores namespaces for a cluster-scoped kind", () => {
    const m = emptyListMessage({ kind: "nodes", namespaces: ["default"], namespaced: false });
    expect(m.title).toBe("No nodes");
    expect(m.hint).toMatch(/no nodes you can see/);
  });

  it("prefers the search over the namespace scope", () => {
    // Both narrow the view, but the one the user just typed is the one they
    // can undo without thinking.
    const m = emptyListMessage({ kind: "pods", query: "web", namespaces: ["prod"] });
    expect(m.title).toBe("No pods match “web”");
  });
});
