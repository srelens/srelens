import { settingsStorage } from "./settingsStorage";

const CACHE_KEY = "srelens.releaseNotes";
const CACHE_LIMIT = 10;
interface CachedNotes { version: string; notes: string }
function readCache(): CachedNotes[] {
  try {
    const value: unknown = JSON.parse(settingsStorage.getItem(CACHE_KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((entry): entry is CachedNotes =>
      entry && typeof entry.version === "string" && typeof entry.notes === "string",
    ).slice(0, CACHE_LIMIT) : [];
  } catch { return []; }
}
function rememberNotes(version: string, notes: string): string {
  // An empty release body may still be awaiting publication; do not cache it.
  if (notes.trim()) {
    try {
      settingsStorage.setItem(CACHE_KEY, JSON.stringify([
        { version, notes }, ...readCache().filter((entry) => entry.version !== version),
      ].slice(0, CACHE_LIMIT)));
    } catch { /* Notes remain readable when preference storage is unavailable. */ }
  }
  return notes;
}

/** Cache by exact version; older dev manifests need the GitHub release body. */
export async function loadUpdateNotes(
  update: { version: string; notes: string },
  options: { signal?: AbortSignal; refresh?: boolean } = {},
): Promise<string> {
  if (update.notes.trim()) return rememberNotes(update.version, update.notes);
  if (!options.refresh) {
    const cached = readCache().find((entry) => entry.version === update.version);
    if (cached) return cached.notes;
  }
  const tag = encodeURIComponent(`srelens-v${update.version}`);
  const controller = options.signal ? null : new AbortController();
  const timeout = controller ? setTimeout(() => controller.abort(), 15_000) : null;
  let response: Response;
  try {
    response = await fetch(`https://api.github.com/repos/srelens/srelens/releases/tags/${tag}`, {
      headers: { Accept: "application/vnd.github+json" },
      signal: options.signal ?? controller!.signal,
      credentials: "omit",
    });
  } finally {
    if (timeout !== null) clearTimeout(timeout);
  }
  if (!response.ok) throw new Error(`Could not read the GitHub release (HTTP ${response.status}).`);
  const release: unknown = await response.json();
  if (!release || typeof release !== "object" || !("body" in release)) {
    throw new Error("GitHub returned an invalid release response.");
  }
  if (release.body === null) return "";
  if (typeof release.body !== "string") throw new Error("GitHub returned invalid release notes.");
  return rememberNotes(update.version, release.body);
}
