import type { ReactNode } from "react";
import { toneColor, type Tone } from "./tone";

export type StatusKind = "success" | "warning" | "danger" | "info" | "neutral";

/**
 * The classic kit named its own colours here — `bg-emerald-500`, `bg-amber-500`,
 * `bg-sky-500` — which is exactly what the tone system exists to stop: a raw
 * palette value does not follow the theme, and the failure is invisible until
 * someone switches to light. The kinds are the caller's vocabulary and survive;
 * they resolve to tones. (#318)
 */
const TONE: Record<StatusKind, Tone> = {
  success: "ok",
  warning: "warn",
  danger: "sev",
  info: "info",
  neutral: "muted",
};

export interface StatusPillProps {
  status: ReactNode;
  kind?: StatusKind;
}

/**
 * A status indicator: a coloured dot followed by a label.
 *
 * The label always carries the meaning in words. The dot is a second channel,
 * never the only one — a status told in colour alone is unreadable to a
 * colour-blind user and silent to a screen reader.
 */
export function StatusPill({ status, kind = "neutral" }: StatusPillProps) {
  return (
    <span className="status" data-kind={kind}>
      <span className="dot" style={{ background: toneColor(TONE[kind]) }} />
      {status}
    </span>
  );
}
