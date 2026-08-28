import { csrfHeader } from "../transport/webTransport";

export interface KubeconfigMeta {
  id: number;
  name: string;
  createdAt: number;
  updatedAt: number;
}

/** GET /api/kubeconfigs — the caller's uploaded kubeconfigs. */
export async function list(): Promise<KubeconfigMeta[]> {
  const res = await fetch("/api/kubeconfigs", { credentials: "include", headers: { ...csrfHeader() } });
  if (!res.ok) throw new Error(`list kubeconfigs failed: ${res.status}`);
  return res.json();
}

/** POST /api/kubeconfigs — upsert by name, sealed at rest server-side. Returns the row id. */
export async function upload(name: string, yaml: string): Promise<number> {
  const res = await fetch("/api/kubeconfigs", {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json", ...csrfHeader() },
    body: JSON.stringify({ name, yaml }),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error ?? `upload failed: ${res.status}`);
  return data.id;
}

/** DELETE /api/kubeconfigs/:id (a missing id is treated as already-removed). */
export async function remove(id: number): Promise<void> {
  const res = await fetch(`/api/kubeconfigs/${id}`, { method: "DELETE", credentials: "include", headers: { ...csrfHeader() } });
  if (!res.ok && res.status !== 404) throw new Error(`delete failed: ${res.status}`);
}
