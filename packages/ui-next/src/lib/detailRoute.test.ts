// @vitest-environment node
import { describe, it, expect } from "vitest";
import { detailRoute, editRoute, parseDetailRoute, parseEditRoute } from "./detailRoute";

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

// `Edit` used to mint `/edit/<name>` — the NAME alone, while every other route
// the row menu mints carries kind, namespace and name. `openTab` dedupes by
// route string, so `Edit` on `default/api` and on `staging/api` collapsed into
// ONE tab, and the second click focused the first resource's editor. So did a
// Pod `api` and a Deployment `api`.
const ON = { cluster: "prod" };

describe("editRoute", () => {
  it("carries the cluster, the kind, the namespace and the name", () => {
    expect(editRoute("Pod", "kube-system", "web-0", ON)).toBe("/edit/prod/Pod/kube-system/web-0");
  });

  it("stands the same placeholder in for a cluster-scoped kind, so the arity never varies", () => {
    expect(editRoute("Node", null, "worker-1", ON)).toBe("/edit/prod/Node/-/worker-1");
  });

  it("gives two namespaces' same-named resources two different routes", () => {
    expect(editRoute("Deployment", "default", "api", ON)).not.toBe(editRoute("Deployment", "staging", "api", ON));
  });

  it("gives two kinds' same-named resources two different routes", () => {
    expect(editRoute("Pod", "default", "api", ON)).not.toBe(editRoute("Deployment", "default", "api", ON));
  });

  it("gives two clusters' same-named resources two different routes", () => {
    // The editor pins the cluster it was opened on, and `openTab` dedupes by
    // route: without the cluster in it, Edit on staging's resource focused
    // prod's pinned editor for the same name, and staging's could not be
    // opened at all until that tab was closed.
    expect(editRoute("ConfigMap", "default", "web", { cluster: "prod" })).not.toBe(
      editRoute("ConfigMap", "default", "web", { cluster: "staging" }),
    );
    expect(parseEditRoute(editRoute("ConfigMap", "default", "web", { cluster: "staging" }))!.cluster).toBe(
      "staging",
    );
  });

  it("carries a custom kind's group in the kind segment, and tells it from the built-in of the same name", () => {
    // A CRD may legally reuse a built-in kind's name in its own group; by kind
    // alone the two are one route, and the editor would open the built-in.
    const custom = editRoute("Deployment", "default", "api", { cluster: "prod", group: "acme.io" });
    expect(custom).toBe("/edit/prod/acme.io%2FDeployment/default/api");
    expect(custom.split("/")).toHaveLength(6);
    expect(parseEditRoute(custom)).toEqual({
      cluster: "prod",
      group: "acme.io",
      kind: "Deployment",
      namespace: "default",
      name: "api",
    });
    expect(custom).not.toBe(editRoute("Deployment", "default", "api", ON));
    expect(parseEditRoute(editRoute("Deployment", "default", "api", ON))!.group).toBeUndefined();
  });

  it("encodes every segment, so a name cannot change the route's shape", () => {
    const route = editRoute("Widget", "default", "a/b", ON);
    expect(route.split("/")).toHaveLength(6);
    expect(parseEditRoute(route)!.name).toBe("a/b");
  });

  it("round-trips", () => {
    for (const [k, ns, n] of [["Pod", "default", "web"], ["Node", null, "n1"]] as const) {
      expect(parseEditRoute(editRoute(k, ns, n, ON))).toEqual({ cluster: "prod", kind: k, namespace: ns, name: n });
    }
  });

  it("still parses the five-segment shape an earlier build persisted, with no cluster", () => {
    // Such a tab opens an editor pinned to whatever the rail is on, rather
    // than the Placeholder.
    expect(parseEditRoute("/edit/Pod/default/web")).toEqual({ kind: "Pod", namespace: "default", name: "web" });
  });

  it("refuses a segment that cannot be decoded, rather than throwing", () => {
    // Same hazard, same answer as `parseDetailRoute`: this runs during render
    // over persisted routes, so a `URIError` is the window failing to boot.
    expect(parseEditRoute("/edit/Pod/default/%zz")).toBeNull();
    expect(parseEditRoute("/edit/%e0%a4%a/default/web-1")).toBeNull();
    expect(parseEditRoute("/edit/Pod/%/web-1")).toBeNull();
    expect(parseEditRoute("/edit/%zz/Pod/default/web-1")).toBeNull();
    expect(parseEditRoute("/edit/prod/Pod/default/%zz")).toBeNull();
  });

  it("refuses the wrong arity, so the legacy one-segment shape is not mistaken for a subject", () => {
    expect(parseEditRoute("/edit/web-1")).toBeNull();
    expect(parseEditRoute("/edit")).toBeNull();
    expect(parseEditRoute("/edit/Pod/default")).toBeNull();
    expect(parseEditRoute("/edit/prod/Pod/default/web-0/extra")).toBeNull();
    // And it does not answer for another prefix's route of the right arity.
    expect(parseEditRoute("/k/Pod/default/web-0")).toBeNull();
    expect(parseDetailRoute("/edit/Pod/default/web-0")).toBeNull();
  });
});
