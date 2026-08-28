/** True when running inside the Tauri desktop shell. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** True in a plain browser (web mode). Evaluated once at load. */
export const isWeb = !isTauri();
