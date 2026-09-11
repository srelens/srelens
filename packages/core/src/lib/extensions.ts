import { kindToResource } from "./kinds";
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
  freelens?: string;
  manifest: ExtensionManifest;
  enabled: boolean;
  revision: number;
  grants: string[];
  settings: Record<string, unknown>;
}
export interface ExtensionInventory {
  schemaVersion: number;
  developerMode: boolean;
  nextRevision: number;
  plugins: InstalledExtension[];
}
export type ExtensionChange =
  | { action: "developerMode"; enabled: boolean }
  | { action: "install"; manifest: string; grants: string[] }
  | { action: "installArchive"; archive: string; grants: string[] }
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
  items: Array<{
    name: string;
    namespace: string;
    age: string;
    columns: string[];
  }>;
}
export const readExtension = <T = ExtensionResourceResult>(
  id: string,
  revision: number,
  capability: string,
  context: string,
  namespace = "",
) =>
  invokeCapability<T>("extensions.read", {
    id,
    revision,
    capability,
    context,
    namespace,
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
  if (pieces.length !== 6 || pieces[1] !== "extensions") return null;
  try {
    const [context, id, page, namespace] = pieces
      .slice(2)
      .map(decodeURIComponent);
    return context && id && page ? { context, id, page, namespace } : null;
  } catch {
    return null;
  }
}
export function contributionKind(kind: string) {
  if (kind.includes("/")) return kind;
  const resource = kindToResource(kind);
  return resource ? `${resource.group}/${kind}` : "";
}

export interface FreelensBootstrap {
  source: string;
  crds: Array<Record<string, unknown>>;
  namespaces: string[];
}
export const readFreelensExtension = <T = unknown>(
  id: string,
  revision: number,
  context: string,
  request: Record<string, unknown>,
) =>
  invokeCapability<T>("extensions.freelensRead", {
    ...request,
    id,
    revision,
    context,
  });
