// The blur behind a see-through window — the native half of the Appearance
// pane's window opacity, which is otherwise all stylesheet.
import { setWindowBlur } from "../transport/transport";

/**
 * Blur what is behind the window, or stop. Fire-and-forget, as `applyUiScale`
 * is: a refused effect (missing permission, a platform without one, web mode)
 * must never break the boot or the click that asked for it.
 */
export function applyWindowBlur(on: boolean): void {
  void setWindowBlur(on).catch(() => {});
}
