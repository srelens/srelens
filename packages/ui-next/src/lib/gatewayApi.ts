import type { CrdRef, CustomRow, PrinterColumn } from "@srelens/core";

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

/** The kinds that attach to a Gateway and send traffic on to a backend. */
export const ROUTE_KINDS = ["HTTPRoute", "GRPCRoute", "TLSRoute", "TCPRoute", "UDPRoute"];

/**
 * Whether a route's gateway took it. The route CRDs print hostnames and an
 * age and nothing about this, which is the one thing a list of routes is
 * read for; the host evaluates whatever columns a caller names, so the list
 * asks for it.
 *
 * ponytail: the FIRST parent's verdict only — the host's JSONPath subset has
 * no wildcard. A route refused by its second gateway alone is not flagged in
 * the list (its detail pane shows every parent). Lift when `[*]` is evaluated.
 */
export const ACCEPTED_COLUMN: PrinterColumn = {
  name: "Accepted",
  type: "string",
  jsonPath: '.status.parents[0].conditions[?(@.type=="Accepted")].status',
};

const inGroup = (crd: CrdRef, kinds: string[]) => isGatewayApi(crd.group) && kinds.includes(crd.kind);

/** `crd` with the columns its list needs and its CRD does not print. */
export function withGatewayColumns(crd: CrdRef): CrdRef {
  const printed = crd.printerColumns ?? [];
  if (!inGroup(crd, ROUTE_KINDS) || printed.some((c) => c.name === ACCEPTED_COLUMN.name)) return crd;
  return { ...crd, printerColumns: [...printed, ACCEPTED_COLUMN] };
}

/**
 * Which of a Gateway API kind's rows need a second look, or `undefined` for a
 * kind with no verdict of its own (a ReferenceGrant) and for every kind in
 * another group. Read off the row's printed columns: `False` is a controller
 * saying no. Empty is a controller that has not answered, which is not a
 * failure — it is every route in the minute after it is applied.
 */
export function gatewayFlagged(crd: CrdRef): ((row: CustomRow) => boolean) | undefined {
  const column = inGroup(crd, ["Gateway"]) ? "Programmed" : inGroup(crd, ROUTE_KINDS) ? ACCEPTED_COLUMN.name : null;
  if (!column) return undefined;
  const at = (crd.printerColumns ?? []).findIndex((c) => c.name === column);
  if (at < 0) return undefined;
  return (row) => row.columns?.[at] === "False";
}
