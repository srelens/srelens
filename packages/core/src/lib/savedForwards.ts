// Saved port-forwards: user-defined shortcuts to re-establish a forward
// later, without re-picking the namespace/kind/target/port each time.
// Persisted per kube-context. Web mode stores rows server-side, one row per
// context, via the per-user settings API (`GET`/`PUT /api/settings/:key`,
// see crates/server/src/api_settings.rs); desktop persists the equivalent
// state through the durable settings mirror.
import { isWeb } from "../transport/platform";
import { csrfHeader } from "../transport/webTransport";
import { settingsStorage } from "./settingsStorage";

export interface SavedForward {
  id: string;
  name: string;
  namespace: string;
  kind: string;
  target: string;
  remotePort: number;
  localPort?: number;
}

const STORAGE_KEY = "srelens.savedForwards";

function settingsKey(context: string): string {
  return `savedForwards:${context}`;
}

async function parseError(res: Response): Promise<never> {
  const data = await res.json().catch(() => ({}));
  throw new Error((data as { error?: string }).error ?? `request failed: ${res.status}`);
}

async function webList(context: string): Promise<SavedForward[]> {
  const res = await fetch(`/api/settings/${encodeURIComponent(settingsKey(context))}`, {
    credentials: "include",
    headers: { ...csrfHeader() },
  });
  if (!res.ok) await parseError(res);
  const data = (await res.json()) as { value: SavedForward[] | null };
  return Array.isArray(data.value) ? data.value : [];
}

async function webPutAll(context: string, list: SavedForward[]): Promise<void> {
  const res = await fetch(`/api/settings/${encodeURIComponent(settingsKey(context))}`, {
    method: "PUT",
    credentials: "include",
    headers: { "Content-Type": "application/json", ...csrfHeader() },
    body: JSON.stringify(list),
  });
  if (!res.ok) await parseError(res);
}

function loadAll(): Record<string, SavedForward[]> {
  try {
    const raw = settingsStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as Record<string, SavedForward[]>) : {};
  } catch {
    return {};
  }
}

function saveAll(map: Record<string, SavedForward[]>): void {
  try {
    settingsStorage.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {
    // ignore unavailable/quota-exceeded storage
  }
}

/** Saved port-forward shortcuts for a kube-context. */
export async function listSavedForwards(context: string): Promise<SavedForward[]> {
  if (isWeb) return webList(context);
  return loadAll()[context] ?? [];
}

/** Upsert a saved forward (matched by id) for a kube-context. */
export async function saveForward(context: string, sf: SavedForward): Promise<void> {
  if (isWeb) {
    const current = await webList(context);
    await webPutAll(context, [...current.filter((f) => f.id !== sf.id), sf]);
    return;
  }
  const all = loadAll();
  const current = all[context] ?? [];
  all[context] = [...current.filter((f) => f.id !== sf.id), sf];
  saveAll(all);
}

/** Remove a saved forward by id for a kube-context. */
export async function deleteSavedForward(context: string, id: string): Promise<void> {
  if (isWeb) {
    const current = await webList(context);
    await webPutAll(
      context,
      current.filter((f) => f.id !== id),
    );
    return;
  }
  const all = loadAll();
  const current = all[context] ?? [];
  all[context] = current.filter((f) => f.id !== id);
  saveAll(all);
}
