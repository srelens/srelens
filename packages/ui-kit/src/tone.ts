/**
 * Semantic colour, shared by every component that shows severity.
 *
 * A tone says what something means — a failing pod, a warning, a healthy node —
 * and the tokens decide what that looks like in each theme. Components never
 * name a colour themselves.
 */
export type Tone = "sev" | "warn" | "ok" | "info" | "accent" | "muted";

const COLOR: Record<Tone, string> = {
  sev: "var(--sev)",
  warn: "var(--warn)",
  ok: "var(--ok)",
  info: "var(--info)",
  accent: "var(--accent)",
  muted: "var(--ink-muted)",
};

const WASH: Record<Tone, string> = {
  sev: "var(--sev-wash)",
  warn: "var(--warn-wash)",
  ok: "var(--ok-wash)",
  info: "var(--info-wash)",
  accent: "var(--accent-wash)",
  // Muted has no wash: a neutral badge sits on whatever surface it lands on.
  muted: "transparent",
};

/**
 * The tone a load reads at — the thresholds a percentage is coloured by.
 *
 * Here rather than inside {@link Meter} because two things draw the same
 * reading: the meter on a node's row, and the figure a screen puts above it
 * (the cluster overview's CPU and Memory tiles are toned by the cluster's own
 * share). A second copy of `> 80` / `> 65` would let a tile call a cluster
 * amber while the meters under it called it red — the same drift the status
 * vocabulary in `@srelens/core` is kept in one place to avoid.
 */
export function loadTone(percent: number): Tone {
  return percent > 80 ? "sev" : percent > 65 ? "warn" : "ok";
}

export function toneColor(tone: Tone): string {
  return COLOR[tone];
}

export function toneWash(tone: Tone): string {
  return WASH[tone];
}
