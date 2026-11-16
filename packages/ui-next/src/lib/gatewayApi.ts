import {
  asArray,
  asRecord,
  str,
  type CrdRef,
  type CustomRow,
  type K8sObject,
  type PrinterColumn,
  type ResourceStatusLine,
} from "@srelens/core";

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

/** The `status` of the condition of this type, or `""` where there is none yet. */
function conditionStatus(conditions: unknown, type: string): string {
  const found = asArray(conditions).map(asRecord).find((c) => c.type === type);
  return found ? str(found.status) : "";
}

export interface GatewayListener {
  name: string;
  protocol: string;
  port: string;
  hostname: string;
  /** How many routes the controller has bound here; `null` before it has said. */
  attachedRoutes: number | null;
  /** The listener's own `Programmed` condition: `True`, `False`, or `""`. */
  programmed: string;
}

/** A Gateway's listeners, each joined by name to what the controller reports for it. */
export function gatewayListeners(object: K8sObject): GatewayListener[] {
  const reported = asArray(asRecord(object.status).listeners).map(asRecord);
  return asArray(asRecord(object.spec).listeners).map((l) => {
    const listener = asRecord(l);
    const status = reported.find((r) => r.name === listener.name);
    return {
      name: str(listener.name),
      protocol: str(listener.protocol),
      port: str(listener.port),
      hostname: str(listener.hostname),
      attachedRoutes: typeof status?.attachedRoutes === "number" ? status.attachedRoutes : null,
      programmed: conditionStatus(status?.conditions, "Programmed"),
    };
  });
}

export interface RouteParent {
  kind: string;
  namespace: string;
  name: string;
  sectionName: string;
  /** The listener port the reference names, or `""`: part of which parent it is. */
  port: string;
  /** The parent's `Accepted` and `ResolvedRefs` conditions: `True`, `False`, or `""`. */
  accepted: string;
  resolvedRefs: string;
}

/** A parent reference with the defaults the API applies: a Gateway, in the route's own namespace. */
function parentKey(ref: Record<string, unknown>, routeNamespace: string) {
  return {
    kind: str(ref.kind) || "Gateway",
    namespace: str(ref.namespace) || routeNamespace,
    name: str(ref.name),
    sectionName: str(ref.sectionName),
    port: ref.port == null ? "" : String(ref.port),
  };
}

/** What a route attaches to, each joined to the verdict that parent's controller gave. */
export function routeParents(object: K8sObject): RouteParent[] {
  const namespace = str(asRecord(object.metadata).namespace);
  const reported = asArray(asRecord(object.status).parents).map(asRecord);
  return asArray(asRecord(object.spec).parentRefs).map((r) => {
    const parent = parentKey(asRecord(r), namespace);
    const status = reported.find((s) => {
      const theirs = parentKey(asRecord(s.parentRef), namespace);
      return (
        theirs.kind === parent.kind &&
        theirs.namespace === parent.namespace &&
        theirs.name === parent.name &&
        theirs.sectionName === parent.sectionName &&
        theirs.port === parent.port
      );
    });
    return {
      ...parent,
      accepted: conditionStatus(status?.conditions, "Accepted"),
      resolvedRefs: conditionStatus(status?.conditions, "ResolvedRefs"),
    };
  });
}

export interface RouteBackend {
  kind: string;
  namespace: string;
  name: string;
  port: string;
  weight: string;
  /** A core Service, which the app can open. Anything else is named and no more. */
  isService: boolean;
}

export interface RouteRule {
  key: string;
  /** What the rule matches, in words; `*` for a rule that matches everything. */
  matches: string;
  backends: RouteBackend[];
}

const pairs = (label: string, items: unknown) =>
  asArray(items).map((item) => `${label} ${str(asRecord(item).name)}=${str(asRecord(item).value)}`);

/** One match: an HTTP method, path, headers and query, or a gRPC service and method. */
function matchText(match: Record<string, unknown>): string {
  const parts: string[] = [];
  if (typeof match.method === "string") parts.push(match.method);
  else if (match.method) {
    const method = asRecord(match.method);
    parts.push([str(method.service), str(method.method)].filter(Boolean).join("/"));
  }
  const path = asRecord(match.path);
  if (path.value !== undefined) parts.push(`${str(path.type) || "PathPrefix"} ${str(path.value)}`);
  parts.push(...pairs("header", match.headers), ...pairs("query", match.queryParams));
  return parts.filter(Boolean).join(" ");
}

/** A route's rules: what each matches and where it sends it. */
export function routeRules(object: K8sObject): RouteRule[] {
  const namespace = str(asRecord(object.metadata).namespace);
  return asArray(asRecord(object.spec).rules).map((r, index) => {
    const rule = asRecord(r);
    const matches = asArray(rule.matches)
      .map((m) => matchText(asRecord(m)))
      .filter(Boolean);
    return {
      key: String(index),
      matches: matches.join(", ") || "*",
      backends: asArray(rule.backendRefs).map((b) => {
        const ref = asRecord(b);
        const kind = str(ref.kind) || "Service";
        return {
          kind,
          namespace: str(ref.namespace) || namespace,
          name: str(ref.name),
          port: str(ref.port),
          weight: str(ref.weight),
          isService: kind === "Service" && !str(ref.group),
        };
      }),
    };
  });
}

/**
 * The header's verdict on a Gateway or a route, in the shape every built-in
 * kind's is. `null` for a kind with none, and for an object no controller has
 * answered: a route applied a second ago is not a failed one.
 */
export function gatewayStatusLine(kind: string, object: K8sObject): ResourceStatusLine | null {
  const well = (status: string): ResourceStatusLine => ({ status, health: "success", flagged: false, readyText: null });
  const unwell = (status: string): ResourceStatusLine => ({ status, health: "danger", flagged: true, readyText: null });
  if (kind === "Gateway") {
    const programmed = conditionStatus(asRecord(object.status).conditions, "Programmed");
    return programmed === "True" ? well("Programmed") : programmed === "False" ? unwell("Not programmed") : null;
  }
  if (!ROUTE_KINDS.includes(kind)) return null;
  const parents = asArray(asRecord(object.status).parents).map((p) => asRecord(p).conditions);
  if (parents.some((c) => conditionStatus(c, "Accepted") === "False")) return unwell("Not accepted");
  if (parents.some((c) => conditionStatus(c, "ResolvedRefs") === "False")) return unwell("Unresolved refs");
  return parents.some((c) => conditionStatus(c, "Accepted") === "True") ? well("Accepted") : null;
}

/** How many of a route's parents a Gateway's route list looks at. */
const PARENTS_READ = 4;

/**
 * The columns that let a list of routes say which gateways each attaches to:
 * a name and a namespace for each of the first {@link PARENTS_READ} parents.
 *
 * ponytail: a route whose only reference to a gateway is its fifth parent or
 * later is not found. The host's JSONPath has no wildcard; lift with it.
 */
export const PARENT_COLUMNS: PrinterColumn[] = Array.from({ length: PARENTS_READ }, (_, i) => [
  { name: `parent${i}`, type: "string", jsonPath: `.spec.parentRefs[${i}].name` },
  { name: `parentNamespace${i}`, type: "string", jsonPath: `.spec.parentRefs[${i}].namespace` },
]).flat();

/** The rows, listed with {@link PARENT_COLUMNS}, whose route attaches to `gateway`. */
export function routesAttachedTo<Row extends CustomRow>(
  rows: Row[],
  gateway: { namespace: string; name: string },
): Row[] {
  return rows.filter((row) =>
    Array.from({ length: PARENTS_READ }, (_, i) => i).some(
      (i) =>
        row.columns?.[2 * i] === gateway.name &&
        (row.columns?.[2 * i + 1] || row.namespace) === gateway.namespace,
    ),
  );
}
