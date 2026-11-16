import type { CrdRef } from "@srelens/core";

/**
 * What the app knows about the Gateway API (#1089).
 *
 * The Gateway API ships as CRDs, so a cluster has it or does not, and the
 * only way to tell is discovery. Everything here starts from the CRDs the
 * cluster serves: with none in this group there is no sidebar group, no
 * flagged row and no pane, and the app looks as it did before.
 *
 * The standard channel's group only. `gateway.networking.x-k8s.io` is the
 * experimental one, whose kinds come and go between releases; it stays under
 * Custom resources, where a kind nobody has heard of is the expected thing.
 */
export const GATEWAY_API_GROUP = "gateway.networking.k8s.io";

export function isGatewayApi(group: string | undefined): boolean {
  return group === GATEWAY_API_GROUP;
}

/** The kinds in the order traffic meets them, then what governs them. */
const KIND_ORDER = [
  "GatewayClass",
  "Gateway",
  "HTTPRoute",
  "GRPCRoute",
  "TLSRoute",
  "TCPRoute",
  "UDPRoute",
  "ReferenceGrant",
  "BackendTLSPolicy",
];

/**
 * The discovered CRDs as the Gateway API's and everyone else's.
 *
 * `gateway` is ordered by {@link KIND_ORDER}; a kind a later release adds is
 * not in that list and follows, by name, so it is listed rather than lost.
 */
export function splitGatewayCrds(crds: CrdRef[]): { gateway: CrdRef[]; rest: CrdRef[] } {
  const place = (crd: CrdRef) => {
    const at = KIND_ORDER.indexOf(crd.kind);
    return at < 0 ? KIND_ORDER.length : at;
  };
  return {
    gateway: crds
      .filter((crd) => isGatewayApi(crd.group))
      .sort((a, b) => place(a) - place(b) || a.kind.localeCompare(b.kind)),
    rest: crds.filter((crd) => !isGatewayApi(crd.group)),
  };
}
