import { invokeCapability } from "../transport/transport";
import type { ActionPredicate } from "./actionPredicates";
import type { CapabilityImpact } from "./capabilities";
// These mirror crates/plugin-host/src/manifest.rs and crates/registry/src/extensions.rs;
// extensionTypes.test.ts fails when a field name or its optionality differs.
interface ExtensionContributionBase {
  id: string;
  title: string;
  capability: string;
}
export interface ExtensionPage extends ExtensionContributionBase {
  group?: string;
  statusColumns?: { ready: number; suspended?: number; progressing?: number };
  dashboard?: {
    pages: string[];
    events?: { capability: string; apiGroups: string[] };
  };
}
export interface ExtensionDetailTab extends ExtensionContributionBase {
  /** Qualified Kubernetes kinds, e.g. `argoproj.io/Application`. */
  forKinds: string[];
}
/** Opens a read-only results panel from a resource detail view; never a cluster write. */
export interface ExtensionDetailLink extends ExtensionContributionBase {
  /** Qualified Kubernetes kinds, e.g. `argoproj.io/Application`. */
  forKinds: string[];
}
export interface ExtensionJoin {
  id: string;
  capability: string;
  match: { label?: string; kindLabel?: string; ownerReference?: boolean; annotation?: string; name?: boolean };
}
export interface ExtensionTableColumn {
  id: string;
  title: string;
  forKinds: string[];
  source: { join?: string; jsonPath: string };
  format: "text" | "number" | "status" | "badge" | "date" | "duration";
  sortable?: boolean;
  filterable?: boolean;
}
export type ExtensionPanelFormat = ExtensionTableColumn["format"];
/** The six statuses every surface draws (#541). */
export type NormalizedStatus = "healthy" | "warning" | "error" | "progressing" | "suspended" | "unknown";
/**
 * A predicate without its refusal sentence: the same operators and path
 * grammar, plus `selfReference` — the value must be a reference, in a
 * host-known format, to the very object the rule reads (an Argo CD tracking
 * id naming its own resource). The host evaluates it; the surface never does.
 */
export type ExtensionStatusCondition = Omit<ActionPredicate, "reason"> & { selfReference?: "argocd-tracking-id" };
/** One status rule; the first whose conditions all hold wins. */
export interface ExtensionStatusRule {
  when: ExtensionStatusCondition[];
  status: NormalizedStatus;
  /** The word shown. Required: colour is never the only signal. */
  label: string;
  /** Where in the object the reason is. */
  reason?: string;
}
export interface ExtensionStatusResolver {
  /** Qualified custom-resource kinds the app declares a reader for. */
  forKinds: string[];
  rules: ExtensionStatusRule[];
}
export interface ExtensionBadge {
  id: string;
  /** Qualified built-in kinds, e.g. `apps/Deployment`. */
  forKinds: string[];
  /** A declared join whose matched resource the rules read; without one they read the row's metadata. */
  join?: string;
  rules: ExtensionStatusRule[];
}
/**
 * What the host resolved an object to. `label` and `reason` are an app's and
 * a cluster's text: draw them through `plainText`.
 */
export interface ResolvedStatus {
  status: NormalizedStatus;
  label: string;
  reason?: string;
}
export interface ResolvedBadge extends ResolvedStatus {
  id: string;
}
export interface ExtensionDetailField {
  label: string;
  jsonPath: string;
  join?: string;
  format?: ExtensionPanelFormat;
}
export type ExtensionDetailSection =
  | { type: "fields"; fields: ExtensionDetailField[] }
  | { type: "conditions"; jsonPath: string; join?: string };
export interface ExtensionDetailPanel {
  id: string;
  title: string;
  forKinds: string[];
  sections: ExtensionDetailSection[];
}
export interface ExtensionManifest {
  /** Editor metadata naming the manifest's JSON Schema; the host ignores it. */
  $schema?: string;
  id: string;
  name: string;
  version: string;
  srelensApiVersion: string;
  kind: "declarative";
  permissions: string[];
  capabilities: Array<{
    name: string;
    title: string;
    target: string;
    arguments: Record<string, unknown>;
    inputs: string[];
  }>;
  /**
   * Declared mutations (#549). Absent in a manifest that only reads, and left
   * out of the stored form when empty. Each entry names a host action
   * primitive and the reader binding whose kind it acts on; the host fills in
   * that kind and fixes the inputs, so nothing here names a resource the app
   * does not already hold a granted reader for.
   */
  actions?: Array<{
    name: string;
    title: string;
    /** A host action primitive, e.g. `k8s.annotate`. */
    target: string;
    /** The `name` of a reader binding in `capabilities`. */
    resource: string;
    /** What the action writes, fixed at install time. */
    arguments: Record<string, unknown>;
    /**
     * What must be true of the resource for the host to send the write
     * (#550). Checked by the host against its own fresh read, so nothing
     * here is a check the surface is trusted to have made.
     */
    preconditions?: ActionPredicate[];
    /**
     * What must be true of the resource for the control to be offered. The
     * same predicates, asked here rather than of the cluster; a condition
     * that must be *enforced* belongs in `preconditions`.
     */
    availableWhen?: ActionPredicate[];
  }>;
  contributions: {
    pages: ExtensionPage[];
    detailTabs: ExtensionDetailTab[];
    detailLinks: ExtensionDetailLink[];
    joins?: ExtensionJoin[];
    tableColumns?: ExtensionTableColumn[];
    detailPanels?: ExtensionDetailPanel[];
    statusResolvers?: ExtensionStatusResolver[];
    badges?: ExtensionBadge[];
  };
}
/** Where a version came from: `catalog` is the exact bytes of a cached catalog release. */
export type ExtensionSource = "local" | "catalog";
/** A version an update replaced, kept so it can be restored. */
export interface ExtensionPreviousVersion {
  signatureProof?: {manifest:string;signature:number[]};
  manifest: ExtensionManifest;
  grants: string[];
  revision: number;
  source: ExtensionSource;
  /** Seconds since the Unix epoch. */
  installedAt: number;
}
export interface InstalledExtension {
  signatureProof?: {manifest:string;signature:number[]};
  /** Set by the host when a stored app failed re-verification; the app is disabled. */
  quarantined?: string;
  /** Host-computed unsigned-app policy denial; the affected app is disabled. */
  policyBlocked?: string;
  manifest: ExtensionManifest;
  enabled: boolean;
  revision: number;
  grants: string[];
  settings: Record<string, unknown>;
  source: ExtensionSource;
  /** When this version was installed, in seconds since the Unix epoch. */
  installedAt: number;
  /** The versions this one replaced, newest first; at most three. */
  history: ExtensionPreviousVersion[];
  /**
   * The keys (`ClusterContext.key`) of the kubeconfig contexts the app is enabled for; absent
   * means every cluster. A context's name is presentation only (#265), and its stable ID can
   * be shared by two contexts (#623), so neither is the identity here.
   */
  contexts?: string[];
}
export interface ExtensionInventory {
  /** Missing in older inventories means false. */
  allowUnsignedApps?: boolean;
  schemaVersion: number;
  nextRevision: number;
  plugins: InstalledExtension[];
}
export type ExtensionChange =
  | { action: "unsignedApps"; allowUnsignedApps: boolean }
  | { action: "install"; manifest: string; grants: string[]; signature?: number[]; reviewedRevision?: number }
  | { action: "enable"; id: string; enabled: boolean }
  | { action: "remove"; id: string }
  /** Restores a kept version; `grants` are what the user reviewed and grants again. */
  | { action: "rollback"; id: string; revision: number; grants: string[] }
  /** Limits the app to these stable context IDs, or with `null` allows every cluster. */
  | { action: "clusters"; id: string; contexts: string[] | null }
  | { action: "settings"; id: string; settings: Record<string, unknown> };
/**
 * Whether an installed app may be used on a context, given that context's key
 * (`ClusterContext.key`). A limited app is not enabled on a context whose key is not
 * known yet; the host enforces the same scope on every read and action.
 */
export const extensionEnabledFor = (
  plugin: Pick<InstalledExtension, "contexts">,
  contextKey: string | undefined,
) => !plugin.contexts || (contextKey !== undefined && plugin.contexts.includes(contextKey));
export const EXTENSIONS_CHANGED = "srelens:extensions-changed";
export const listExtensions = () =>
  invokeCapability<ExtensionInventory>("extensions.list", {});
export async function configureExtensions(change: ExtensionChange) {
  const state = await invokeCapability<ExtensionInventory>(
    "extensions.configure",
    change,
  );
  if (typeof window !== "undefined")
    window.dispatchEvent(new Event(EXTENSIONS_CHANGED));
  return state;
}
/**
 * One manifest problem. `code` is stable (docs/extensions/specification.md); `path` names
 * the value at fault, e.g. `contributions.pages[2].capability`, and is empty for the whole
 * manifest.
 */
export interface ExtensionValidationError {
  code: string;
  path: string;
  message: string;
}
/** Host-computed access changes for the exact manifest and installed revision reviewed. */
export interface ExtensionPermissionDiff {
  previousRevision: number | null;
  added: string[];
  removed: string[];
  unchanged: string[];
}
/** Checks a manifest exactly as installing it with these grants would, without installing. */
export const validateExtension = (manifest: string, grants: string[], signature?: number[]) =>
  invokeCapability<{ errors: ExtensionValidationError[]; permissionDiff?: ExtensionPermissionDiff }>("extensions.validate", {
    manifest,
    grants,
    ...(signature ? { signature } : {}),
  });
export interface ExtensionResourceResult {
  printerColumns?: Array<{name:string;jsonPath:string;type?:string}>;
  columnsError?: string;
  /** True when the backend stopped at its row cap and more remain (#609). */
  truncated?: boolean;
  items: Array<{
    name: string;
    namespace: string;
    age: string;
    created?: string | null;
    columns: string[];
    /** The row's status, when the app declares a status resolver for its kind (#541). */
    status?: ResolvedStatus;
  }>;
}
type ExtensionResourceItem = ExtensionResourceResult["items"][number];
/**
 * One listed resource's normalized status (#541): the host's, resolved from
 * the app's `statusResolvers` on the whole object, or — for a page still on
 * the deprecated `statusColumns` — read from its printer columns and mapped
 * onto the same six statuses. `unknown` when neither says anything.
 *
 * Per item on purpose, so a count by status (#540's `countByStatus`, an app
 * dashboard) maps it over the rows it already holds.
 */
export function itemStatus(item: ExtensionResourceItem, statusColumns?: ExtensionPage["statusColumns"]): NormalizedStatus {
  if (item.status) return item.status.status;
  if (!statusColumns) return "unknown";
  const truth = (index?: number) => index !== undefined && item.columns[index]?.toLowerCase() === "true";
  if (truth(statusColumns.suspended)) return "suspended";
  if (truth(statusColumns.progressing)) return "progressing";
  const ready = item.columns[statusColumns.ready]?.toLowerCase();
  return ready === "true" ? "healthy" : ready === "false" ? "error" : "unknown";
}
/** {@link itemStatus} for each item, in order. */
export const itemStatuses = (items: ExtensionResourceItem[], statusColumns?: ExtensionPage["statusColumns"]) =>
  items.map((item) => itemStatus(item, statusColumns));
export const readExtension = <T = ExtensionResourceResult>(
  id: string,
  revision: number,
  capability: string,
  context: string,
  namespace = "",
  useCrdColumns = false,
) =>
  invokeCapability<T>("extensions.read", {
    id,
    revision,
    capability,
    context,
    namespace,
    ...(useCrdColumns ? {useCrdColumns:true} : {}),
  });
export interface ExtensionColumnRow {
  uid?: string;
  name: string;
  namespace: string;
  row: Record<string, unknown>;
}
export interface ExtensionColumnResult {
  columns: ExtensionTableColumn[];
  /** The badges the app declares for the requested kind (#541). */
  badges?: ExtensionBadge[];
  cells: Array<{ uid?: string | null; name: string; namespace: string;
    values: Record<string, string | null>; errors?: Record<string, string>;
    badges?: ResolvedBadge[]; badgeErrors?: Record<string, string> }>;
}
export const resolveExtensionColumns = (
  id: string, revision: number, context: string, namespace: string, kind: string,
  uids: ExtensionColumnRow[],
) => invokeCapability<ExtensionColumnResult>("extensions.resolveColumns", {
  id, revision, context, namespace, kind, uids,
});
export type ExtensionResolvedPanel = {
  id: string;
  title: string;
  sections: Array<
    { type: "fields"; fields: Array<{ label: string; value: string | null; format?: ExtensionPanelFormat; error?: string }> }
    | { type: "conditions"; items: Array<{ type: string; status: "True" | "False" | "Unknown"; reason?: string; message?: string; observedGeneration?: number; lastTransitionTime?: string }>; error?: string }
  >;
};
export const resolveExtensionPanels = (
  id: string, revision: number, context: string, namespace: string, kind: string, resource: object,
) => invokeCapability<{ panels: ExtensionResolvedPanel[] }>("extensions.resolvePanels", {
  id, revision, context, namespace, kind, resource,
});
export function extensionRoute(
  context: string,
  id: string,
  page: string,
  namespace = "",
) {
  return `/extensions/${[context, id, page, namespace].map(encodeURIComponent).join("/")}`;
}
/** A cluster identity route; legacy `/extensions/` routes still carry display names. */
export function extensionClusterRoute(clusterId: string, id: string, page: string, namespace = "") {
  return extensionRoute(clusterId, id, page, namespace).replace("/extensions/", "/extension-clusters/");
}
export function extensionClusterResourceRoute(clusterId: string, id: string, page: string, namespace: string, name: string) {
  return `${extensionClusterRoute(clusterId, id, page, namespace)}/${encodeURIComponent(name)}`;
}
export function parseExtensionRoute(route: string) {
  const pieces = route.split("/");
  if ((pieces.length !== 6 && pieces.length !== 7) || !["extensions", "extension-clusters"].includes(pieces[1])) return null;
  try {
    const [context, id, page, namespace] = pieces
      .slice(2)
      .map(decodeURIComponent);
    const resourceName = pieces.length === 7 ? decodeURIComponent(pieces[6]) : undefined;
    if (pieces.length === 7 && !resourceName) return null;
    return context && id && page ? { context, id, page, namespace, ...(pieces[1] === "extension-clusters" ? { clusterId: context } : {}), ...(resourceName ? { resourceName } : {}) } : null;
  } catch {
    return null;
  }
}
export function contributionKind(kind: string, group = "") {
  return kind.includes("/") ? kind : `${group}/${kind}`;
}

export interface ExtensionCatalogEntry {
  id: string;
  name: string;
  description: string;
  repository: string;
  license: string;
  release: { version: string; manifestUrl: string; sha256: string; srelensApiVersion: string; prerelease: boolean };
  testedHost: { repository: string; revision: string };
}
export interface ExtensionCatalogSnapshot {
  catalog: { schemaVersion: number; extensions: ExtensionCatalogEntry[] };
  fetchedAt: number;
  stale: boolean;
  error: string | null;
  /** The newest supported extension API version. Deprecated: read `hostApiVersions`. */
  hostApiVersion: string;
  /** Every extension API version this host supports, oldest first. */
  hostApiVersions: string[];
  incompatible: string[];
}
export const listExtensionCatalog = (refresh = false) =>
  invokeCapability<ExtensionCatalogSnapshot>("extensions.catalog", { refresh });
/** Returns the exact checksum-verified bytes for explicit permission review. */
export const reviewCatalogExtension = (id: string, sha256: string) =>
  invokeCapability<{ manifest: string; signature?: number[] | null }>("extensions.catalogManifest", { id, sha256 });

/** Host-selected resource identity; API group/kind are resolved from the installed app. */
export interface ExtensionResourceSelection {
  id: string; revision: number; capability: string; context: string; namespace: string; name: string;
}
export interface ExtensionResourceDetail {
  resource: { apiVersion?: string; kind?: string; metadata: { name: string; namespace?: string; uid: string; resourceVersion: string; creationTimestamp?: string; labels?: Record<string,string>; annotations?: Record<string,string>; [key:string]: unknown }; spec?: Record<string, any>; status?: Record<string, any>; [key:string]: unknown };
  actions: string[];
  /** Titles and display predicates come from the installed manifest; impact
   * and confirmation wording come exclusively from the host primitive. */
  actionMeta?: Record<string, { title: string; availableWhen?: ActionPredicate[]; impact: CapabilityImpact; confirm: string | null }>;
  /** Newest first. */
  events?: Array<{type?:string;reason?:string;message?:string;count?:number;time?:string|null}>;
  /** True when the host returned only the newest events. */
  eventsTruncated?: boolean;
  /** True when the host stopped at its page bound: these are the newest of the events it read. */
  eventsPartial?: boolean;
  /** How many events the host read before choosing these. */
  eventsRead?: number;
  eventsError?: string | null;
}
export const inspectExtensionResource = (resource: ExtensionResourceSelection) => invokeCapability<ExtensionResourceDetail>("extensions.resource", resource);
/** Dispatched on `window` after the host accepts an action; `detail` is the acted-on resource. */
export const EXTENSION_RESOURCE_CHANGED = "srelens:extension-resource-changed";
export async function actOnExtensionResource(resource: ExtensionResourceSelection, action: string, uid: string, resourceVersion: string) {
  const result = await invokeCapability<{requested: boolean}>("extensions.action", {resource, action, uid, resourceVersion});
  // Lists, dashboards and details of the same resource may be open in other tabs.
  if (result.requested && typeof window !== "undefined")
    window.dispatchEvent(new CustomEvent<ExtensionResourceSelection>(EXTENSION_RESOURCE_CHANGED, {detail: resource}));
  return result;
}
export function onExtensionResourceChanged(listener: (resource: ExtensionResourceSelection) => void) {
  const handle = (event: Event) => listener((event as CustomEvent<ExtensionResourceSelection>).detail);
  window.addEventListener(EXTENSION_RESOURCE_CHANGED, handle);
  return () => window.removeEventListener(EXTENSION_RESOURCE_CHANGED, handle);
}

export function extensionResourceRoute(context:string,id:string,page:string,namespace:string,name:string) {
  return `${extensionRoute(context,id,page,namespace)}/${encodeURIComponent(name)}`;
}
