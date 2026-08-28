// Interface scale (#237): one persisted percentage applied as the webview's
// native zoom level.
//
// Why native zoom and not a CSS knob: the stylesheet is px-based (see
// ui/styles.css), so scaling the root font-size moves almost nothing, and CSS
// `zoom` on the root would multiply the app shell's `height: 100vh` and the
// drawer's fixed positioning past the window edge. Native zoom shrinks the
// layout viewport itself — exactly what a browser's Cmd/Ctrl +/- does — so px
// sizes, viewport units, and fixed overlays all stay consistent.
//
// Desktop only: in web mode the browser's own zoom already does this, which is
// why the shortcuts below must NOT be intercepted there (preventDefault would
// suppress the native zoom the user already has).

import { setWebviewZoom } from "../transport/transport";
import { settingsStorage } from "./settingsStorage";

const UI_SCALE_KEY = "srelens.uiScale";

/** Interface scale bounds, in percent of the default size. */
export const UI_SCALE = { MIN: 80, MAX: 150, DEFAULT: 100, STEP: 10 } as const;

/** Clamp any value to the supported scale range; fall back to the default. */
export function clampUiScale(value: unknown): number {
  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return UI_SCALE.DEFAULT;
  return Math.round(Math.max(UI_SCALE.MIN, Math.min(UI_SCALE.MAX, n)));
}

/** The persisted interface scale in percent, or 100 when unset/invalid. */
export function getUiScale(): number {
  try {
    const raw = settingsStorage.getItem(UI_SCALE_KEY);
    return raw === null ? UI_SCALE.DEFAULT : clampUiScale(JSON.parse(raw));
  } catch {
    return UI_SCALE.DEFAULT;
  }
}

/** Persist the interface scale (clamped). Returns the value stored. */
export function setUiScale(percent: number): number {
  const clamped = clampUiScale(percent);
  try {
    settingsStorage.setItem(UI_SCALE_KEY, JSON.stringify(clamped));
  } catch {
    // ignore unavailable/quota-exceeded storage
  }
  return clamped;
}

/**
 * Apply a scale to the webview. Fire-and-forget: a zoom failure (missing
 * permission, web mode) must never break the caller's render or keystroke.
 */
export function applyUiScale(percent: number): void {
  void setWebviewZoom(clampUiScale(percent) / 100).catch(() => {});
}

/**
 * Which scale action a keydown asks for, or null. Cmd/Ctrl with `+`/`=`
 * zooms in, `-` zooms out, `0` resets — the browser-zoom vocabulary the
 * reporter asked for. Alt/AltGr combos are left alone (they type characters
 * on some layouts).
 */
export function uiScaleShortcut(e: {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}): "in" | "out" | "reset" | null {
  if (!(e.metaKey || e.ctrlKey) || e.altKey) return null;
  if (e.key === "+" || e.key === "=") return "in";
  if (e.key === "-" || e.key === "_") return "out";
  if (e.key === "0") return "reset";
  return null;
}

/** The next scale after applying a shortcut action to `current`. */
export function stepUiScale(current: number, action: "in" | "out" | "reset"): number {
  if (action === "reset") return UI_SCALE.DEFAULT;
  return clampUiScale(current + (action === "in" ? UI_SCALE.STEP : -UI_SCALE.STEP));
}
