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

export function toneColor(tone: Tone): string {
  return COLOR[tone];
}

export function toneWash(tone: Tone): string {
  return WASH[tone];
}
