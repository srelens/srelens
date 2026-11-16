import { describe, it, expect } from "vitest";
import type { CrdRef } from "@srelens/core";
import { GATEWAY_API_GROUP, isGatewayApi, splitGatewayCrds } from "./gatewayApi";

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
