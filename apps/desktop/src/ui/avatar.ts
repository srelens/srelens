/**
 * Deterministic avatar colour + initials for a cluster name — used by the
 * far-left srelens cluster hotbar.
 */

// A pleasant, evenly-spread palette for cluster avatars.
const AVATAR_COLORS = [
  "#3d90ce",
  "#5bb85b",
  "#e8a33d",
  "#cd6bd0",
  "#e85555",
  "#3bb6a8",
  "#7c83ff",
  "#d4795b",
];

/** Stable hash of a string (djb2). */
function hash(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) {
    h = (h * 33) ^ s.charCodeAt(i);
  }
  return h >>> 0;
}

/** Pick a stable colour for a cluster name. */
export function avatarColor(name: string): string {
  return AVATAR_COLORS[hash(name) % AVATAR_COLORS.length];
}

/** A segment that carries no identity: a generated id (hex with at least one
 * digit, 6+ chars — `6bcb8b63`) or a bare number. Cluster names commonly end
 * in these, and their initial tells the user nothing. The digit requirement
 * keeps real words that happen to be hex-alphabet (`decade`, `facade`) out
 * of the noise class. */
function isNoiseSegment(segment: string): boolean {
  return /^(?=.*\d)[0-9a-f]{6,}$/i.test(segment) || /^\d+$/.test(segment);
}

/** Up-to-3-char initials from a cluster name (splits on - _ space /),
 * skipping generated-id segments so similarly named contexts stay
 * distinguishable: `dev-lon-nrtc-6bcb8b63` → `DLN` and
 * `dev-lon-workload-15d9c530` → `DLW`, where two initials gave `DL` for
 * both (issue #209). Falls back to the raw segments when everything looks
 * generated (`k3d-7f3a9b21` still gets a label). */
export function avatarInitials(name: string): string {
  const parts = name.split(/[-_\s/]+/).filter(Boolean);
  if (parts.length === 0) return "?";
  const meaningful = parts.filter((p) => !isNoiseSegment(p));
  const chosen = meaningful.length > 0 ? meaningful : parts;
  if (chosen.length === 1) return chosen[0].slice(0, 2).toUpperCase();
  return chosen
    .slice(0, 3)
    .map((p) => p[0])
    .join("")
    .toUpperCase();
}
