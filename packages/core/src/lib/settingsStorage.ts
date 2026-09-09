import { readBackendSettings, writeBackendSettings, type SettingsSetInput } from "../transport/settingsTransport";

// Every desktop preference that existed before the file store. Keeping this
// list explicit prevents a broad localStorage sweep from importing unrelated
// WebView/application data.
const NEXT_MARKS_KEY = "srelens.next.marks";
const MIGRATION_KEYS = [
  "srelens.requestTimeoutSecs",
  "srelens.clusterNamespaces",
  "srelens.defaultNamespace",
  "srelens.workspaceLayout",
  "srelens.contextProfiles",
  NEXT_MARKS_KEY,
  "srelens.design",
  "srelens.restoreSession",
  "srelens.openTabs",
  "srelens.onboarded",
  "srelens.releaseNotes",
  "srelens.next.appearance",
  "srelens.next.workspaces",
  "srelens.next.columns",
  "srelens.next.recentLogs",
  "srelens.next.peekWidth",
  "srelens.next.sectionFolds",
  "srelens.next.namespaces",
  "srelens.next.ambiguousContextProfiles",
  "srelens.next.ambiguousContextOrder",
  "srelens.kubeconfigFiles",
  "srelens.hiddenColumns",
  "srelens.contextOrder",
  "srelens.updateChannel",
  "srelens.mcp",
  "srelens.recents",
  "srelens.uiScale",
  "srelens.savedForwards",
  "srelens.assistant.lastAgent",
  "fl-theme-v2",
] as const;

const LEGACY_ALIASES: Record<string, string[]> = {
  "fl-theme-v2": ["fl-theme"],
};

const values = new Map<string, unknown>();
let backendReady = false;
let initializationAttempted = false;
let lastWriteError: unknown;
let writes: Promise<void> = Promise.resolve();

function decode(raw: string): unknown {
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return raw;
  }
}

function encode(value: unknown): string {
  return typeof value === "string" ? value : (JSON.stringify(value) ?? "null");
}

function aliasesFor(key: string): string[] {
  const aliases = [...(LEGACY_ALIASES[key] ?? [])];
  if (key.startsWith("srelens.")) aliases.push(key.replace("srelens.", "freelens."));
  return aliases;
}

function enqueue(input: SettingsSetInput): void {
  writes = writes
    .then(async () => {
      await writeBackendSettings(input);
      lastWriteError = undefined;
    })
    .catch((error) => {
      lastWriteError = error;
      // Persistence remains best-effort at synchronous call sites, as it was
      // with localStorage, but failures are no longer silent.
      console.error("could not persist settings", error);
    });
}

/** Load the backend before React initializes synchronous settings state. */
export async function initializeSettingsStorage(): Promise<void> {
  initializationAttempted = true;
  backendReady = false;
  values.clear();
  try {
    const loaded = await readBackendSettings();
    Object.entries(loaded.values).forEach(([key, value]) => values.set(key, value));
    backendReady = true;

    // Scan the explicit allowlist on upgrades too: earlier releases omitted
    // new-design preferences. Backend values always win over old local copies.
    const migrated: Record<string, unknown> = {};
    let scanned = true;
    try {
      for (const key of MIGRATION_KEYS) {
        if (values.has(key)) continue;
        for (const candidate of [key, ...aliasesFor(key)]) {
          const raw = localStorage.getItem(candidate);
          if (raw === null) continue;
          migrated[key] = key === "fl-theme-v2" && candidate === "fl-theme" && (raw === "light" || raw === "dark")
            ? { name: "slate", mode: raw } : decode(raw);
          break;
        }
      }
    } catch (error) {
      scanned = false;
      console.warn("localStorage unreadable; deferring migration to a later launch", error);
    }
    if (!scanned) return;
    try {
      if (!loaded.localStorageMigrated || Object.keys(migrated).length > 0) {
        await writeBackendSettings({ values: migrated, ...(!loaded.localStorageMigrated ? { localStorageMigrated: true } : {}) });
        Object.entries(migrated).forEach(([key, value]) => values.set(key, value));
      }
    } catch (error) {
      console.error("could not migrate legacy settings; keeping the backend active and legacy copies intact", error);
      return;
    }
    // Remove copies only after the backend accepted the import. Clearing stale
    // copies of existing backend keys also prevents resurrection after a reset.
    try {
      for (const key of MIGRATION_KEYS) {
        localStorage.removeItem(key);
        for (const alias of aliasesFor(key)) localStorage.removeItem(alias);
      }
    } catch (error) {
      console.warn("durable settings migrated but old localStorage could not be cleared", error);
    }
  } catch (error) {
    console.error("could not initialize settings backend; settings writes are unavailable", error);
  }
}

/** Storage-shaped synchronous facade used by the existing settings helpers. */
export const settingsStorage = {
  getItem(key: string): string | null {
    if (!initializationAttempted) return localStorage.getItem(key);
    return values.has(key) ? encode(values.get(key)) : null;
  },

  setItem(key: string, raw: string): void {
    if (!initializationAttempted) {
      localStorage.setItem(key, raw);
      return;
    }
    if (!backendReady) throw new Error("Settings backend is unavailable");
    const value = decode(raw);
    values.set(key, value);
    enqueue({ values: { [key]: value } });
  },

  removeItem(key: string): void {
    if (!initializationAttempted) {
      localStorage.removeItem(key);
      return;
    }
    if (!backendReady) throw new Error("Settings backend is unavailable");
    values.delete(key);
    enqueue({ remove: [key] });
  },
};

/** Test seam for callers that need to observe queued write completion. */
export async function flushSettingsWrites(options: { throwOnError?: boolean } = {}): Promise<void> {
  await writes;
  if (options.throwOnError && lastWriteError) throw lastWriteError;
}
