import { invokeCapability } from "../transport/transport";
export interface ExtensionContribution {
  id: string;
  title: string;
  capability: string;
  forKinds?: string[];
  group?: string;
  statusColumns?: { ready: number; suspended?: number; progressing?: number };
  dashboard?: {
    pages: string[];
    events?: { capability: string; apiGroups: string[] };
  };
}
export interface ExtensionManifest {
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
    pages: ExtensionContribution[];
    detailTabs: ExtensionContribution[];
    rowActions: ExtensionContribution[];
  };
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
  | { action: "settings"; id: string; settings: Record<string, unknown> };
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
export function parseExtensionRoute(route: string) {
  const pieces = route.split("/");
  if ((pieces.length !== 6 && pieces.length !== 7) || pieces[1] !== "extensions") return null;
  try {
    const [context, id, page, namespace] = pieces
      .slice(2)
      .map(decodeURIComponent);
    const resourceName = pieces.length === 7 ? decodeURIComponent(pieces[6]) : undefined;
    if (pieces.length === 7 && !resourceName) return null;
    return context && id && page ? { context, id, page, namespace, ...(resourceName ? { resourceName } : {}) } : null;
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
  hostApiVersion: string;
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
