// Bridges the persisted request-timeout setting to the Rust backend.
//
// The backend keeps the timeout in a process-wide global that resets to its
// default on every launch, so the persisted value must be re-applied at
// startup and whenever the user changes it in Settings.

import { invokeCommand } from "../transport/transport";
import { clampTimeoutSecs, getRequestTimeoutSecs, setRequestTimeoutSecs } from "./settings";

let updateQueue: Promise<void> = Promise.resolve();

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
export function updateRequestTimeout(secs: number): Promise<number> {
  const clamped = clampTimeoutSecs(secs);
  const update = updateQueue.then(async () => {
    const applied = await invokeCommand<number>("set_request_timeout", { secs: clamped });
    return setRequestTimeoutSecs(applied);
  });
  updateQueue = update.then(() => undefined, () => undefined);
  return update;
}
