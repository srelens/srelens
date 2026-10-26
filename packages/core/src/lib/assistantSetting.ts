import type { ClusterContext } from "./clusters";
import { settingsStorage } from "./settingsStorage";

/**
 * Whether the AI assistant is on, globally and per cluster (#968).
 *
 * One key in the durable settings document, read by both designs here and by
 * every srelens MCP server in Rust (`crates/registry/src/assistant.rs`), which
 * refuses what this says is off — so the UI hiding an entry point is never the
 * only thing standing between an agent and a cluster.
 *
 * The cluster switch can only turn the assistant OFF: global off wins, and a
 * cluster's entry is kept while global is off so switching back on restores it.
 */
export const ASSISTANT_KEY = "srelens.assistant";

export interface AssistantSetting {
  enabled: boolean;
  /** Stable IDs (`<kubeconfig path>#<context>`) of the clusters it is off for. */
  disabledClusters: string[];
}

/**
 * Anything but the documented shape reads as on, with nothing disabled: the
 * key is absent on every install that never touched it, and a value an older
 * or newer build wrote must not throw in render. Only the boolean `false`
 * turns it off, so a hand-edited `"false"` cannot.
 */
export function parseAssistantSetting(raw: string | null): AssistantSetting {
  let doc: unknown;
  try {
    doc = raw ? JSON.parse(raw) : null;
  } catch {
    doc = null;
  }
  if (typeof doc !== "object" || doc === null || Array.isArray(doc)) return { enabled: true, disabledClusters: [] };
  const { enabled, disabledClusters } = doc as Record<string, unknown>;
  return {
    enabled: enabled !== false,
    disabledClusters: Array.isArray(disabledClusters) ? disabledClusters.filter((id): id is string => typeof id === "string") : [],
  };
}

export function readAssistantSetting(storage: Pick<Storage, "getItem"> = settingsStorage): AssistantSetting {
  return parseAssistantSetting(storage.getItem(ASSISTANT_KEY));
}

export function writeAssistantSetting(next: AssistantSetting, storage: Pick<Storage, "setItem"> = settingsStorage): void {
  storage.setItem(ASSISTANT_KEY, JSON.stringify(next));
}

/**
 * Whether the assistant may act on `cluster`, named any way the app names one:
 * the context's name, its pinned ID, stable ID, key, or the kubeconfig's own
 * name for it. Every matching context is checked, not the first, as
 * `protectionFor` does — two files can name two clusters alike, and matching
 * more can only refuse more. No cluster at all is allowed while global is on.
 */
export function assistantAllowedFor(
  setting: AssistantSetting,
  contexts: readonly ClusterContext[],
  cluster: string | null | undefined,
): boolean {
  if (!setting.enabled) return false;
  if (!cluster) return true;
  const off = new Set(setting.disabledClusters);
  if (off.has(cluster)) return false;
  return !contexts.some((context) => {
    if (!off.has(context.stableId)) return false;
    const file = `${context.sourceFile}#`;
    const original = context.stableId.startsWith(file) ? context.stableId.slice(file.length) : undefined;
    return [context.name, context.pinnedId, context.key, original].includes(cluster);
  });
}
