import { invokeCapability, invokeCommand, subscribe, type Invoker } from "../transport/transport";

// Types mirror the Rust DTOs in crates/kube/src/toolbox.rs.

/** A managed CLI tool's inventory entry (Toolbox "Tools" section). */
export interface ToolStatus {
  name: string;
  installed: boolean;
  path?: string | null;
  version?: string | null;
  /** "managed" (srelens installed it) or "system". */
  source?: string | null;
}

export type RequirementKind = "kubectl" | "krew-plugin" | "external";
export type RequirementStatus = "found" | "not-on-app-path" | "missing";

/** One exec-auth requirement of a context, resolved. */
export interface RequirementResult {
  binary: string;
  kind: RequirementKind;
  plugin?: string | null;
  installable: boolean;
  status: RequirementStatus;
  path?: string | null;
  version?: string | null;
}

export interface DiagnosisReport {
  context: string;
  items: RequirementResult[];
}

/** Result of a tool install. */
export interface InstallResult {
  tool: string;
  version: string;
  path: string;
}

/** A krew index entry. */
export interface Plugin {
  name: string;
  description: string;
  installed: boolean;
}

export interface PluginActionResult {
  plugin: string;
  output: string;
}

/** Result wrappers keep the try/catch out of components (mirrors lib/helm.ts). */
type Result<T> = { data?: T; error?: string };

async function call<T>(run: () => Promise<T>): Promise<Result<T>> {
  try {
    return { data: await run() };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Inventory the managed toolchain (kubectl, krew, helm). */
export function toolboxStatus(invoke: Invoker = invokeCapability): Promise<Result<ToolStatus[]>> {
  return call(async () => (await invoke<{ tools: ToolStatus[] }>("toolbox.status", {})).tools);
}

/** Diagnose a context's exec-auth tool requirements. */
export function diagnoseContext(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<Result<DiagnosisReport>> {
  return call(() => invoke<DiagnosisReport>("toolbox.diagnoseContext", { context }));
}

/** Search the krew plugin index. */
export function searchPlugins(
  query: string,
  invoke: Invoker = invokeCapability,
): Promise<Result<Plugin[]>> {
  return call(async () => (await invoke<{ plugins: Plugin[] }>("toolbox.searchPlugins", { query })).plugins);
}

const installTool = (id: string, invoke: Invoker) =>
  call(() => invoke<InstallResult>(id, {}));

/** Install the latest stable kubectl into ~/.srelens/bin. */
export const installKubectl = (invoke: Invoker = invokeCapability) =>
  installTool("toolbox.installKubectl", invoke);

/** Install the latest helm into ~/.srelens/bin. */
export const installHelm = (invoke: Invoker = invokeCapability) =>
  installTool("toolbox.installHelm", invoke);

/** Bootstrap krew into ~/.krew. */
export const installKrew = (invoke: Invoker = invokeCapability) =>
  installTool("toolbox.installKrew", invoke);

interface RawInstallProgress {
  tool: string;
  received: number;
  total: number | null;
}

/**
 * Install a managed tool (kubectl / helm / krew) via the streaming
 * `start_tool_install` command, reporting download progress as a whole percent
 * (or null while the total size is unknown / during the post-download install).
 * The same verified install as the capability, with a progress bar.
 */
export async function startToolInstall(
  tool: "kubectl" | "helm" | "krew",
  onProgress?: (percent: number | null) => void,
): Promise<Result<InstallResult>> {
  const dispose = await subscribe("toolbox://progress", (payload) => {
    const p = payload as RawInstallProgress;
    if (p.tool !== tool) return;
    onProgress?.(p.total ? Math.min(100, Math.round((p.received / p.total) * 100)) : null);
  });
  try {
    return { data: await invokeCommand<InstallResult>("start_tool_install", { tool }) };
  } catch (e) {
    return { error: String(e) };
  } finally {
    dispose();
  }
}

const pluginAction = (id: string, plugin: string, invoke: Invoker) =>
  call(() => invoke<PluginActionResult>(id, { plugin }));

/** Install a krew plugin. */
export const installPlugin = (plugin: string, invoke: Invoker = invokeCapability) =>
  pluginAction("toolbox.installPlugin", plugin, invoke);

/** Upgrade an installed krew plugin. */
export const upgradePlugin = (plugin: string, invoke: Invoker = invokeCapability) =>
  pluginAction("toolbox.upgradePlugin", plugin, invoke);

/** Remove an installed krew plugin. */
export const removePlugin = (plugin: string, invoke: Invoker = invokeCapability) =>
  pluginAction("toolbox.removePlugin", plugin, invoke);
