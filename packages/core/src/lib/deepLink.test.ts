import { describe, expect, it } from "vitest";
import { checkDeepLink, dedupeDeepLinkTargets, parseDeepLink, type DeepLinkTarget } from "./deepLink";

describe("parseDeepLink", () => {
  it("parses a cluster link", () => {
    expect(parseDeepLink("srelens://cluster/kind-dev")).toEqual({
      route: "cluster",
      context: "kind-dev",
    });
  });

  it("parses a namespaced resource link", () => {
    expect(parseDeepLink("srelens://resource/prod/kube-system/Pod/coredns-abc")).toEqual({
      route: "resource",
      context: "prod",
      namespace: "kube-system",
      kind: "Pod",
      name: "coredns-abc",
    });
  });

  it("treats '-' as no namespace, for cluster-scoped kinds", () => {
    // An empty segment would collapse when the path is split, so the
    // placeholder has to be a real character.
    expect(parseDeepLink("srelens://resource/prod/-/Node/worker-1")).toMatchObject({
      namespace: null,
      kind: "Node",
      name: "worker-1",
    });
  });

  it("accepts the extra slashes different platforms produce", () => {
    // A link with no authority component is normalized differently by each OS
    // handler; both spellings must land in the same place.
    const expected = { route: "cluster", context: "prod" };
    expect(parseDeepLink("srelens://cluster/prod")).toEqual(expected);
    expect(parseDeepLink("srelens:///cluster/prod")).toEqual(expected);
    expect(parseDeepLink("SRELENS://cluster/prod")).toEqual(expected);
    expect(parseDeepLink("  srelens://cluster/prod/  ")).toEqual(expected);
  });

  it("keeps an encoded slash inside a context instead of splitting the route", () => {
    // OpenShift contexts genuinely look like this; splitting here would turn
    // one context into three path segments and break the link.
    expect(parseDeepLink("srelens://cluster/default%2Fapi-example-com%3A6443%2Fdev")).toEqual({
      route: "cluster",
      context: "default/api-example-com:6443/dev",
    });
  });

  it("ignores a query string or fragment", () => {
    expect(parseDeepLink("srelens://cluster/prod?utm=x")).toEqual({
      route: "cluster",
      context: "prod",
    });
    expect(parseDeepLink("srelens://cluster/prod#frag")).toEqual({
      route: "cluster",
      context: "prod",
    });
  });

  it("refuses anything that is not an srelens link", () => {
    for (const url of ["", "https://example.com/cluster/prod", "srelen://cluster/prod", "cluster/prod"]) {
      expect(parseDeepLink(url)).toBeNull();
    }
  });

  it("refuses unknown routes and wrong segment counts", () => {
    // A malformed link must be inert, never partially honoured.
    for (const url of [
      "srelens://",
      "srelens://cluster",
      "srelens://cluster/a/b",
      "srelens://resource/prod/ns/Pod",
      "srelens://resource/prod/ns/Pod/name/extra",
      "srelens://evil/prod",
    ]) {
      expect(parseDeepLink(url)).toBeNull();
    }
  });

  it("refuses control characters and malformed encoding", () => {
    expect(parseDeepLink("srelens://cluster/pro%00d")).toBeNull();
    expect(parseDeepLink("srelens://cluster/bad%ZZ")).toBeNull();
  });
});

describe("dedupeDeepLinkTargets", () => {
  const resource = (context: string, kind: string, name: string): DeepLinkTarget => ({
    route: "resource",
    context,
    namespace: "default",
    kind,
    name,
  });

  it("collapses links that open the same view, keeping the last", () => {
    // A double-click would otherwise append two Pods tabs against the same
    // render's state, duplicating the tab and its resource watch.
    const result = dedupeDeepLinkTargets([
      resource("prod", "Pod", "first"),
      resource("prod", "Pod", "second"),
    ]);
    expect(result).toHaveLength(1);
    expect(result[0]).toMatchObject({ name: "second" });
  });

  it("keeps distinct views separate", () => {
    const result = dedupeDeepLinkTargets([
      resource("prod", "Pod", "a"),
      resource("prod", "Service", "b"),
      resource("staging", "Pod", "c"),
      { route: "cluster", context: "prod" },
    ]);
    expect(result).toHaveLength(4);
  });

  it("collapses repeated cluster links", () => {
    const result = dedupeDeepLinkTargets([
      { route: "cluster", context: "prod" },
      { route: "cluster", context: "prod" },
    ]);
    expect(result).toEqual([{ route: "cluster", context: "prod" }]);
  });

  it("orders by LAST occurrence, so the final link is routed last", () => {
    // Routing order decides which tab ends up active. A replaced entry must
    // move to the end, or an earlier link would be applied after a later one
    // and steal focus from the link the user actually clicked most recently.
    const result = dedupeDeepLinkTargets([
      resource("prod", "Pod", "a"),
      { route: "cluster", context: "prod" },
      resource("prod", "Pod", "b"),
    ]);
    expect(result).toHaveLength(2);
    expect(result[0]).toMatchObject({ route: "cluster" });
    expect(result[1]).toMatchObject({ route: "resource", name: "b" });
  });
});

// The one rule set both designs judge a link by. The reasons are the sentences
// classic has always shown, written out here by hand rather than read back from
// the module, so a reworded or dropped check fails a test instead of quietly
// changing what a reader is told.
describe("checkDeepLink", () => {
  const contexts = ["prod", "staging"];

  it("passes a cluster link to a context that exists, as its parsed target", () => {
    expect(checkDeepLink("srelens://cluster/staging", contexts)).toEqual({
      ok: true,
      target: { route: "cluster", context: "staging" },
    });
  });

  it("passes a namespaced resource link that names its namespace", () => {
    expect(checkDeepLink("srelens://resource/prod/kube-system/Pod/coredns-abc", contexts)).toEqual({
      ok: true,
      target: {
        route: "resource",
        context: "prod",
        namespace: "kube-system",
        kind: "Pod",
        name: "coredns-abc",
      },
    });
  });

  it("passes '-' for a cluster-scoped kind, which has no namespace to name", () => {
    expect(checkDeepLink("srelens://resource/prod/-/Node/worker-1", contexts)).toMatchObject({
      ok: true,
      target: { kind: "Node", namespace: null, name: "worker-1" },
    });
  });

  it("refuses a link srelens cannot parse", () => {
    expect(checkDeepLink("srelens://evil/prod", contexts)).toEqual({
      ok: false,
      reason: "It isn't a link srelens understands.",
    });
  });

  it("refuses a context that is not listed, matching the name exactly", () => {
    expect(checkDeepLink("srelens://cluster/dev", contexts)).toEqual({
      ok: false,
      reason: 'No kube context named "dev".',
    });
    // Context names are case-sensitive in a kubeconfig, so `Prod` is not `prod`.
    expect(checkDeepLink("srelens://resource/Prod/default/Pod/web", contexts)).toEqual({
      ok: false,
      reason: 'No kube context named "Prod".',
    });
  });

  it("refuses a kind with no detail view", () => {
    // Events have a list and no detail, so such a link would land on the list
    // instead of the object it named.
    expect(checkDeepLink("srelens://resource/prod/default/Event/web.17f", contexts)).toEqual({
      ok: false,
      reason: "srelens can't open a Event directly.",
    });
  });

  it("refuses a namespaced kind given '-' in place of its namespace", () => {
    expect(checkDeepLink("srelens://resource/prod/-/Pod/web", contexts)).toEqual({
      ok: false,
      reason: "Pod is namespaced, so the link needs a namespace.",
    });
  });
});
