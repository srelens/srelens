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

/**
 * GitHub allows an unauthenticated address 60 API requests an hour, shared
 * with the Star button's count and anyone else behind the same address (#926).
 * Once GitHub says they are spent, nothing is asked before its reset time, or
 * an hour when it gives none; never longer, as for the Star button.
 */
const LIMIT_MAX_MS = 60 * 60 * 1000;
let limitedUntil = 0;
function rateLimited(now: number): Error {
  const minutes = Math.max(1, Math.ceil((limitedUntil - now) / 60_000));
  return new Error(`GitHub's rate limit was reached; try again in ${minutes} minute${minutes === 1 ? "" : "s"}.`);
}

/**
 * The installed build's own release notes, bundled by the release workflow so
 * a release build never asks GitHub for them (#926). Empty in builds made
 * anywhere else, which fall back to the GitHub release.
 */
export function bundledReleaseNotes(): string {
  return (import.meta.env.VITE_SRELENS_RELEASE_NOTES as string | undefined) ?? "";
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
  const now = Date.now();
  // Bounded, so a clock put back since cannot silence the notes for longer.
  if (now < limitedUntil && limitedUntil - now <= LIMIT_MAX_MS) throw rateLimited(now);
  const tag = encodeURIComponent(`srelens-v${update.version}`);
  const controller = options.signal ? null : new AbortController();
  const timeout = controller ? setTimeout(() => controller.abort(), 15_000) : null;
  try {
    const response = await fetch(`https://api.github.com/repos/srelens/srelens/releases/tags/${tag}`, {
      headers: { Accept: "application/vnd.github+json" },
      signal: options.signal ?? controller!.signal,
      credentials: "omit",
    });
    const headers = response.headers;
    if (response.status === 429 || (response.status === 403 && (headers.get("x-ratelimit-remaining") === "0" || headers.has("retry-after")))) {
      const retryAfter = Number(headers.get("retry-after")) * 1000;
      const reset = Number(headers.get("x-ratelimit-reset")) * 1000;
      const until = retryAfter > 0 ? now + retryAfter : reset > now ? reset : now + LIMIT_MAX_MS;
      limitedUntil = Math.min(until, now + LIMIT_MAX_MS);
      throw rateLimited(now);
    }
    if (!response.ok) throw new Error(`Could not read the GitHub release (HTTP ${response.status}).`);
    const release: unknown = await response.json();
    if (!release || typeof release !== "object" || !("body" in release)) {
      throw new Error("GitHub returned an invalid release response.");
    }
    if (release.body === null) return "";
    if (typeof release.body !== "string") throw new Error("GitHub returned invalid release notes.");
    return rememberNotes(update.version, release.body);
  } finally {
    if (timeout !== null) clearTimeout(timeout);
  }
}
