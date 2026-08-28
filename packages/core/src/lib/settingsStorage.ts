import { invokeCapability } from "../transport/transport";
import { isTauri } from "../transport/platform";

interface SettingsGetOutput {
  schemaVersion: number;
  localStorageMigrated: boolean;
  values: Record<string, unknown>;
}

interface SettingsSetInput {
  values?: Record<string, unknown>;
  remove?: string[];
  localStorageMigrated?: boolean;
}

// Every desktop preference that existed before the file store. Keeping this
// list explicit prevents a broad localStorage sweep from importing unrelated
// WebView/application data.
const MIGRATION_KEYS = [
  "srelens.requestTimeoutSecs",
  "srelens.clusterNamespaces",
  "srelens.defaultNamespace",
  "srelens.workspaceLayout",
  "srelens.contextProfiles",
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
let fileBacked = false;
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
      await invokeCapability("settings.set", input);
    })
    .catch((error) => {
      // Persistence remains best-effort at synchronous call sites, as it was
      // with localStorage, but failures are no longer silent.
      console.error("could not persist settings", error);
    });
}

/**
 * Load the desktop file before React initializes synchronous settings state.
 * On the first successful load, import known localStorage keys in one atomic
 * write and only then remove the old copies.
 */
export async function initializeSettingsStorage(): Promise<void> {
  if (!isTauri()) return;
  try {
    const loaded = await invokeCapability<SettingsGetOutput>("settings.get", {});
    values.clear();
    Object.entries(loaded.values).forEach(([key, value]) => values.set(key, value));

    if (!loaded.localStorageMigrated) {
      const migrated: Record<string, unknown> = {};
      // Reading the OLD store is optional work: a WebView with localStorage
      // disabled throws from getItem, and letting that escape would abandon
      // the file backend we just loaded successfully — falling back to the
      // very storage that is unavailable, so nothing could persist at all.
      let scanned = true;
      try {
        for (const key of MIGRATION_KEYS) {
          if (values.has(key)) continue;
          for (const candidate of [key, ...aliasesFor(key)]) {
            const raw = localStorage.getItem(candidate);
            if (raw === null) continue;
            migrated[key] =
              key === "fl-theme-v2" &&
              candidate === "fl-theme" &&
              (raw === "light" || raw === "dark")
                ? { name: "slate", mode: raw }
                : decode(raw);
            break;
          }
        }
      } catch (error) {
        // The scan is all-or-nothing on purpose. Committing the migrated flag
        // after a partial read would retire the one-time import for good, so a
        // transient storage failure would strand the legacy preferences
        // permanently. Leave the flag unset and retry on the next launch.
        scanned = false;
        console.warn("localStorage unreadable; deferring migration to a later launch", error);
      }

      if (scanned) {
        await invokeCapability("settings.set", {
          values: migrated,
          localStorageMigrated: true,
        } satisfies SettingsSetInput);
        Object.entries(migrated).forEach(([key, value]) => values.set(key, value));

        // Only reached once the import is committed, so a failed or deferred
        // migration always leaves its localStorage data intact for the retry.
        try {
          for (const key of MIGRATION_KEYS) {
            localStorage.removeItem(key);
            for (const alias of aliasesFor(key)) localStorage.removeItem(alias);
          }
        } catch (error) {
          console.warn(
            "durable settings migrated but old localStorage could not be cleared",
            error,
          );
        }
      }
    }
    fileBacked = true;
  } catch (error) {
    // A corrupt/unwritable file must not prevent the app from opening.
    fileBacked = false;
    console.error("could not initialize durable settings; using localStorage", error);
  }
}

/** Storage-shaped synchronous facade used by the existing settings helpers. */
export const settingsStorage = {
  getItem(key: string): string | null {
    if (!fileBacked) return localStorage.getItem(key);
    return values.has(key) ? encode(values.get(key)) : null;
  },

  setItem(key: string, raw: string): void {
    if (!fileBacked) {
      localStorage.setItem(key, raw);
      return;
    }
    const value = decode(raw);
    values.set(key, value);
    enqueue({ values: { [key]: value } });
  },

  removeItem(key: string): void {
    if (!fileBacked) {
      localStorage.removeItem(key);
      return;
    }
    values.delete(key);
    enqueue({ remove: [key] });
  },
};

/** Test seam for callers that need to observe queued write completion. */
export async function flushSettingsWrites(): Promise<void> {
  await writes;
}
