// Bridges the persisted request-timeout setting to the Rust backend.
//
// The backend keeps the timeout in a process-wide global that resets to its
// default on every launch, so the persisted value must be re-applied at
// startup and whenever the user changes it in Settings.

import { invokeCommand } from "../transport/transport";
import { clampTimeoutSecs, getRequestTimeoutSecs, setRequestTimeoutSecs } from "./settings";

/** Push the persisted timeout to the backend. Call once on startup. */
export async function applyPersistedTimeout(): Promise<number> {
  const secs = getRequestTimeoutSecs();
  try {
    return await invokeCommand<number>("set_request_timeout", { secs });
  } catch {
    // No backend (tests / web preview) — the persisted value still stands.
    return secs;
  }
}

/** Apply a timeout before persisting it, so a refusal cannot look like a saved change. */
export async function updateRequestTimeout(secs: number): Promise<number> {
  const applied = await invokeCommand<number>("set_request_timeout", { secs: clampTimeoutSecs(secs) });
  return setRequestTimeoutSecs(applied);
}
