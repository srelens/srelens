// @vitest-environment node
import { describe, it, expect } from "vitest";
import { detailRoute, parseDetailRoute } from "./detailRoute";

describe("detailRoute", () => {
  it("carries the kind, the namespace and the name", () => {
    expect(detailRoute("Pod", "kube-system", "web-0")).toBe("/k/Pod/kube-system/web-0");
  });

  it("stands a placeholder in for a cluster-scoped kind, so the arity never varies", () => {
    expect(detailRoute("Node", null, "worker-1")).toBe("/k/Node/-/worker-1");
  });

  it("encodes every segment, so a name cannot change the route's shape", () => {
    // A CRD's kind and a resource's name can both contain a slash in the wild.
    const route = detailRoute("Widget", "default", "a/b");
    expect(route.split("/")).toHaveLength(5);
    expect(parseDetailRoute(route)!.name).toBe("a/b");
  });

  it("round-trips", () => {
    for (const [k, ns, n] of [["Pod", "default", "web"], ["Node", null, "n1"]] as const) {
      expect(parseDetailRoute(detailRoute(k, ns, n))).toEqual({ kind: k, namespace: ns, name: n });
    }
  });

  it("refuses a segment that cannot be decoded, rather than throwing", () => {
    // The hazard `parseLogsRoute` documents at length, in the five-segment
    // parser next door: this one also runs DURING RENDER over persisted
    // routes — `describe` and `screenFor` both call it — so a `URIError` here
    // is the whole new-design window failing to boot off one corrupted string
    // in storage, not a bad tab. `null` is this function's existing answer
    // for a route it cannot make a subject of.
    expect(parseDetailRoute("/k/Pod/default/%zz")).toBeNull();
    expect(parseDetailRoute("/k/%e0%a4%a/default/web-1")).toBeNull();
    expect(parseDetailRoute("/k/Pod/%/web-1")).toBeNull();
    // Every other segment decodes fine; the route is still refused rather
    // than returning a half-decoded subject.
    expect(parseDetailRoute("/k/Pod/%zz/web-1")).toBeNull();
    // A cluster-scoped route never decodes its namespace segment, so the name
    // is the only one left that can break this one.
    expect(parseDetailRoute("/k/Node/-/%zz")).toBeNull();
  });

  it("refuses a route that is not a detail route", () => {
    expect(parseDetailRoute("/k/pods")).toBeNull(); // a LIST route
    expect(parseDetailRoute("/k/Pod/default")).toBeNull(); // too few segments
    expect(parseDetailRoute("/resources")).toBeNull();
  });
});
