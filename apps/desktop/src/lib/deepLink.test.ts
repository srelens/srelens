import { describe, expect, it } from "vitest";
import { parseDeepLink } from "./deepLink";

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
