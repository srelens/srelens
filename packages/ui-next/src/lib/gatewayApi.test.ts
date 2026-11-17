import { describe, it, expect } from "vitest";
import type { CrdRef, K8sObject } from "@srelens/core";
import {
  GATEWAY_API_GROUP,
  PARENT_COLUMNS,
  gatewayListeners,
  gatewayStatusLine,
  isGatewayApi,
  routeParents,
  routeRules,
  routesAttachedTo,
  splitGatewayCrds,
} from "./gatewayApi";

const crd = (plural: string, group: string, kind: string): CrdRef => ({
  name: `${plural}.${group}`,
  group,
  version: "v1",
  kind,
  plural,
  namespaced: true,
});

const G = GATEWAY_API_GROUP;

describe("isGatewayApi", () => {
  it("is the standard channel's group and nothing else", () => {
    expect(isGatewayApi("gateway.networking.k8s.io")).toBe(true);
    expect(isGatewayApi("gateway.networking.x-k8s.io")).toBe(false);
    expect(isGatewayApi("networking.k8s.io")).toBe(false);
    expect(isGatewayApi(undefined)).toBe(false);
  });
});

describe("splitGatewayCrds", () => {
  it("takes the Gateway API kinds out, in the order traffic meets them", () => {
    const certs = crd("certificates", "cert-manager.io", "Certificate");
    const experimental = crd("xlistenersets", "gateway.networking.x-k8s.io", "XListenerSet");
    const { gateway, rest } = splitGatewayCrds([
      crd("referencegrants", G, "ReferenceGrant"),
      certs,
      crd("httproutes", G, "HTTPRoute"),
      crd("zebras", G, "Zebra"),
      crd("gateways", G, "Gateway"),
      experimental,
      crd("apples", G, "Apple"),
      crd("gatewayclasses", G, "GatewayClass"),
    ]);
    expect(gateway.map((c) => c.kind)).toEqual([
      "GatewayClass",
      "Gateway",
      "HTTPRoute",
      "ReferenceGrant",
      "Apple",
      "Zebra",
    ]);
    expect(rest).toEqual([certs, experimental]);
  });

  it("finds nothing on a cluster without the CRDs", () => {
    expect(splitGatewayCrds([crd("certificates", "cert-manager.io", "Certificate")]).gateway).toEqual([]);
  });
});

describe("reading a Gateway", () => {
  const gw: K8sObject = {
    kind: "Gateway",
    metadata: { name: "edge", namespace: "infra" },
    spec: {
      listeners: [
        { name: "http", protocol: "HTTP", port: 80 },
        { name: "https", protocol: "HTTPS", port: 443, hostname: "*.example.com" },
      ],
    },
    status: {
      conditions: [{ type: "Programmed", status: "True" }],
      listeners: [
        { name: "https", attachedRoutes: 3, conditions: [{ type: "Programmed", status: "False" }] },
      ],
    },
  };

  it("joins each listener to what the controller reports for it, by name", () => {
    expect(gatewayListeners(gw)).toEqual([
      { name: "http", protocol: "HTTP", port: "80", hostname: "", attachedRoutes: null, programmed: "" },
      { name: "https", protocol: "HTTPS", port: "443", hostname: "*.example.com", attachedRoutes: 3, programmed: "False" },
    ]);
  });

  it("has no listeners for a gateway with no spec", () => {
    expect(gatewayListeners({ kind: "Gateway", metadata: { name: "x" } })).toEqual([]);
  });

  it("is well when programmed, flagged when not, and silent before the controller answers", () => {
    expect(gatewayStatusLine("Gateway", gw)).toMatchObject({ status: "Programmed", flagged: false });
    const down = { ...gw, status: { conditions: [{ type: "Programmed", status: "False" }] } };
    expect(gatewayStatusLine("Gateway", down)).toMatchObject({ status: "Not programmed", health: "danger", flagged: true });
    expect(gatewayStatusLine("Gateway", { kind: "Gateway", metadata: { name: "x" } })).toBeNull();
  });
});

describe("reading a route", () => {
  const route: K8sObject = {
    kind: "HTTPRoute",
    metadata: { name: "shop", namespace: "store" },
    spec: {
      parentRefs: [{ name: "edge", namespace: "infra", sectionName: "https" }, { name: "local" }],
      rules: [
        {
          matches: [
            { path: { type: "PathPrefix", value: "/api" }, method: "GET", headers: [{ name: "x-env", value: "canary" }] },
            { path: { type: "Exact", value: "/health" } },
          ],
          backendRefs: [
            { name: "api", port: 8080, weight: 90 },
            { name: "api-canary", namespace: "canary", port: 8080, weight: 10 },
            { group: "multicluster.x-k8s.io", kind: "ServiceImport", name: "far" },
          ],
        },
        { backendRefs: [{ name: "web", port: 80 }] },
      ],
    },
    status: {
      parents: [
        {
          parentRef: { name: "edge", namespace: "infra", sectionName: "https" },
          conditions: [{ type: "Accepted", status: "True" }, { type: "ResolvedRefs", status: "False" }],
        },
      ],
    },
  };

  it("joins each parent to its verdict, defaulting the namespace to the route's", () => {
    expect(routeParents(route)).toEqual([
      { kind: "Gateway", group: G, namespace: "infra", name: "edge", sectionName: "https", port: "", accepted: "True", resolvedRefs: "False" },
      { kind: "Gateway", group: G, namespace: "store", name: "local", sectionName: "", port: "", accepted: "", resolvedRefs: "" },
    ]);
  });

  it("keeps a parent's API group, and joins it only to its own verdict", () => {
    const route: K8sObject = {
      kind: "HTTPRoute",
      metadata: { name: "own", namespace: "store" },
      spec: { parentRefs: [{ name: "edge" }, { group: "example.com", kind: "Gateway", name: "edge" }] },
      status: {
        parents: [
          { parentRef: { group: "example.com", kind: "Gateway", name: "edge" }, conditions: [{ type: "Accepted", status: "False" }] },
          { parentRef: { name: "edge" }, conditions: [{ type: "Accepted", status: "True" }] },
        ],
      },
    };
    expect(routeParents(route).map((p) => [p.group, p.accepted])).toEqual([
      [G, "True"],
      ["example.com", "False"],
    ]);
  });

  it("tells two references to one gateway apart by port", () => {
    const route: K8sObject = {
      kind: "HTTPRoute",
      metadata: { name: "split", namespace: "store" },
      spec: { parentRefs: [{ name: "edge", port: 80 }, { name: "edge", port: 443 }] },
      status: {
        parents: [
          { parentRef: { name: "edge", port: 80 }, conditions: [{ type: "Accepted", status: "True" }] },
          { parentRef: { name: "edge", port: 443 }, conditions: [{ type: "Accepted", status: "False" }] },
        ],
      },
    };
    expect(routeParents(route).map((p) => [p.port, p.accepted])).toEqual([
      ["80", "True"],
      ["443", "False"],
    ]);
  });

  it("writes a rule's matches the way a reader would say them", () => {
    const rules = routeRules(route);
    expect(rules.map((r) => r.matches)).toEqual(["GET PathPrefix /api header x-env=canary, Exact /health", "*"]);
  });

  it("tells a regular-expression header or query match from an exact one", () => {
    const regex: K8sObject = {
      kind: "HTTPRoute",
      metadata: { name: "re", namespace: "store" },
      spec: {
        rules: [
          {
            matches: [
              {
                headers: [
                  { type: "RegularExpression", name: "x-tenant", value: "acme.*" },
                  { type: "Exact", name: "x-env", value: "acme.*" },
                ],
                queryParams: [{ type: "RegularExpression", name: "v", value: "2.*" }],
              },
            ],
          },
        ],
      },
    };
    expect(routeRules(regex)[0].matches).toBe("header x-tenant=~acme.* header x-env=acme.* query v=~2.*");
  });

  it("reads a gRPC rule's service and method", () => {
    const grpc: K8sObject = {
      kind: "GRPCRoute",
      metadata: { name: "g", namespace: "store" },
      spec: { rules: [{ matches: [{ method: { service: "cart.Cart", method: "Add" } }] }] },
    };
    expect(routeRules(grpc)[0].matches).toBe("cart.Cart/Add");
  });

  it("tells a regular-expression gRPC method from an exact one", () => {
    const grpc: K8sObject = {
      kind: "GRPCRoute",
      metadata: { name: "g", namespace: "store" },
      spec: { rules: [{ matches: [{ method: { type: "RegularExpression", service: "cart\\..*", method: "Add" } }] }] },
    };
    expect(routeRules(grpc)[0].matches).toBe("RegularExpression cart\\..*/Add");
  });

  it("reads a rule with an empty match among others as matching everything", () => {
    // Matches are alternatives; an empty one matches every request.
    const any: K8sObject = {
      kind: "HTTPRoute",
      metadata: { name: "any", namespace: "store" },
      spec: { rules: [{ matches: [{}, { path: { type: "Exact", value: "/health" } }] }] },
    };
    expect(routeRules(any)[0].matches).toBe("*");
  });

  it("names each backend, and knows which are Services", () => {
    expect(routeRules(route)[0].backends).toEqual([
      { kind: "Service", namespace: "store", name: "api", port: "8080", weight: "90", isService: true },
      { kind: "Service", namespace: "canary", name: "api-canary", port: "8080", weight: "10", isService: true },
      { kind: "ServiceImport", namespace: "store", name: "far", port: "", weight: "", isService: false },
    ]);
  });

  it("is flagged when any parent refused it or could not resolve its backends", () => {
    expect(gatewayStatusLine("HTTPRoute", route)).toMatchObject({ status: "Unresolved refs", flagged: true });
    const refused = { ...route, status: { parents: [{ parentRef: { name: "edge" }, conditions: [{ type: "Accepted", status: "False" }] }] } };
    expect(gatewayStatusLine("HTTPRoute", refused)).toMatchObject({ status: "Not accepted", flagged: true });
    const fine = { ...route, status: { parents: [{ parentRef: { name: "edge" }, conditions: [{ type: "Accepted", status: "True" }] }] } };
    expect(gatewayStatusLine("HTTPRoute", fine)).toMatchObject({ status: "Accepted", flagged: false });
  });

  it("says nothing of a route no controller has answered, or of a kind with no verdict", () => {
    expect(gatewayStatusLine("HTTPRoute", { kind: "HTTPRoute", metadata: { name: "x" } })).toBeNull();
    expect(gatewayStatusLine("ReferenceGrant", { kind: "ReferenceGrant", metadata: { name: "x" } })).toBeNull();
    expect(routeParents({ kind: "HTTPRoute", metadata: { name: "x" } })).toEqual([]);
    expect(routeRules({ kind: "HTTPRoute", metadata: { name: "x" } })).toEqual([]);
  });
});

describe("finding a gateway's routes", () => {
  /** Columns as PARENT_COLUMNS asks for them: name, namespace, kind, group per parent, four parents. */
  const parents = (...refs: [string, string?, string?, string?][]) =>
    Array.from({ length: 4 }, (_, i) => refs[i] ?? ["", "", "", ""]).flatMap(([name, ns = "", kind = "", group = ""]) => [name, ns, kind, group]);

  it("asks for the name, namespace, kind and group of the first four parents of every route", () => {
    expect(PARENT_COLUMNS.map((c) => c.jsonPath).slice(0, 4)).toEqual([
      ".spec.parentRefs[0].name",
      ".spec.parentRefs[0].namespace",
      ".spec.parentRefs[0].kind",
      ".spec.parentRefs[0].group",
    ]);
    expect(PARENT_COLUMNS).toHaveLength(16);
  });

  it("keeps the rows that name the gateway, in its namespace or by default their own", () => {
    const rows = [
      { name: "a", namespace: "infra", age: "", columns: parents(["edge"]) },
      { name: "b", namespace: "store", age: "", columns: parents(["other"], ["edge", "infra"]) },
      { name: "c", namespace: "store", age: "", columns: parents(["edge"]) },
      { name: "d", namespace: "store", age: "" },
    ];
    expect(routesAttachedTo(rows, { namespace: "infra", name: "edge" }).map((r) => r.name)).toEqual(["a", "b"]);
  });

  it("does not take a mesh route's Service parent for a gateway of the same name", () => {
    const rows = [
      { name: "mesh", namespace: "infra", age: "", columns: parents(["edge", "", "Service", ""]) },
      { name: "named", namespace: "infra", age: "", columns: parents(["edge", "", "Gateway", "gateway.networking.k8s.io"]) },
    ];
    expect(routesAttachedTo(rows, { namespace: "infra", name: "edge" }).map((r) => r.name)).toEqual(["named"]);
  });
});
