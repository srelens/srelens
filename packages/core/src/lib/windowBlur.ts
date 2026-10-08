// The blur behind a see-through window — the native half of the Appearance
// pane's window opacity, which is otherwise all stylesheet.
import { setWindowBlur } from "../transport/transport";

/**
 * Blur what is behind the window, or stop, and say whether the host did it.
 *
 * Never rejects: this is called during boot and from a slider, and a refused
 * effect must not break either. But a refusal is not passed off as success.
 * It resolves `false` and is logged with its reason, so the caller — which
 * keeps a record of what the window is wearing — can leave that record
 * unchanged and ask again, and so a blur that never appears has something in
 * the log to explain it.
 */
export function applyWindowBlur(on: boolean): Promise<boolean> {
  return setWindowBlur(on).then(
    () => true,
    (error: unknown) => {
      console.warn(`srelens: could not turn the blur behind the window ${on ? "on" : "off"}`, error);
      return false;
    },
  );
}
