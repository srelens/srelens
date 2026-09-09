import { invokeCapability } from "./transport";
import { isTauri } from "./platform";

export interface SettingsGetOutput {
  schemaVersion: number;
  localStorageMigrated: boolean;
  values: Record<string, unknown>;
}
export interface SettingsSetInput {
  values?: Record<string, unknown>;
  remove?: string[];
  localStorageMigrated?: boolean;
}
const MIGRATED_KEY = "srelens.settingsMigrated";

async function request(path: string, init: RequestInit = {}): Promise<Response> {
  const response = await fetch(path, {
    credentials: "same-origin",
    ...init,
    headers: { "Content-Type": "application/json", "x-srelens-csrf": "1" },
  });
  if (!response.ok) throw new Error(`Settings backend request failed (HTTP ${response.status})`);
  return response;
}

export async function readBackendSettings(): Promise<SettingsGetOutput> {
  if (isTauri()) return invokeCapability("settings.get", {});
  const response = await request("/api/settings");
  const { values } = await response.json() as { values: Record<string, unknown> };
  return { schemaVersion: 1, localStorageMigrated: values[MIGRATED_KEY] === true, values };
}

export async function writeBackendSettings(input: SettingsSetInput): Promise<void> {
  if (isTauri()) { await invokeCapability("settings.set", input); return; }
  // Per-key writes preserve changes from other windows. Mark migration complete
  // only after every import has succeeded; retries keep existing backend values.
  for (const [key, value] of Object.entries(input.values ?? {})) {
    await request(`/api/settings/${encodeURIComponent(key)}`, { method: "PUT", body: JSON.stringify(value) });
  }
  for (const key of input.remove ?? []) {
    await request(`/api/settings/${encodeURIComponent(key)}`, { method: "DELETE" });
  }
  if (input.localStorageMigrated !== undefined) {
    await request(`/api/settings/${MIGRATED_KEY}`, { method: "PUT", body: JSON.stringify(input.localStorageMigrated) });
  }
}
