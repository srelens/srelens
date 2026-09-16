import { invokeCapability } from "../transport/transport";
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
  contributions: {
    pages: ExtensionPage[];
    detailTabs: ExtensionDetailTab[];
    detailLinks: ExtensionDetailLink[];
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
  schemaVersion: number;
  nextRevision: number;
  plugins: InstalledExtension[];
}
export type ExtensionChange =
  | { action: "install"; manifest: string; grants: string[]; signature?: number[] }
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
/** Checks a manifest exactly as installing it with these grants would, without installing. */
export const validateExtension = (manifest: string, grants: string[], signature?: number[]) =>
  invokeCapability<{ errors: ExtensionValidationError[] }>("extensions.validate", {
    manifest,
    grants,
    ...(signature ? { signature } : {}),
  });
export interface ExtensionResourceResult {
  printerColumns?: Array<{name:string;jsonPath:string;type?:string}>;
  columnsError?: string;
  items: Array<{
    name: string;
    namespace: string;
    age: string;
    created?: string | null;
    columns: string[];
  }>;
}
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
