/** Relative age from an ISO timestamp, e.g. "5d", "3h", "10m". */
export function ageFromTimestamp(iso?: string, now: number = Date.now()): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  const secs = Math.max(0, Math.floor((now - then) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

/** Human-readable duration between two ISO timestamps, e.g. "2m 30s". */
export function durationBetween(startIso?: string, endIso?: string): string {
  if (!startIso || !endIso) return "—";
  const secs = Math.max(0, Math.floor((new Date(endIso).getTime() - new Date(startIso).getTime()) / 1000));
  if (Number.isNaN(secs)) return "—";
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  if (mins < 60) return remSecs ? `${mins}m ${remSecs}s` : `${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  return remMins ? `${hours}h ${remMins}m` : `${hours}h`;
}

/** Absolute, human-readable timestamp, e.g. "Jun 10, 2026, 12:52:33 PM". */
export function absoluteTimestamp(iso?: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function timestampWithAge(iso: string, now: number): string {
  return iso ? `${ageFromTimestamp(iso, now)} ago (${absoluteTimestamp(iso)})` : "";
}
