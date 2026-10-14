import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * How much of each log stream's history srelens keeps on disk while it is
 * open (#914), in megabytes. Read when a Logs screen opens its stream, so a
 * change applies to streams opened from then on and never restarts one.
 *
 * Persisted through `settingsStorage` like `peekWidth`, and injectable, so
 * tests need a Map. Every accessor is guarded: a size that does not survive
 * the session is better than a Logs screen that cannot open.
 */
export const LOG_HISTORY_KEY = "srelens.next.logHistoryMb";

/** The sizes offered. The desktop backend refuses more than the largest. */
export const LOG_HISTORY_CHOICES = [64, 256, 1024, 4096] as const;

export const DEFAULT_LOG_HISTORY_MB = 256;

/** The chosen size: anything stored that is not one of the choices reads as the default. */
export function logHistoryMb(storage: Storage = settingsStorage): number {
  try {
    const raw = storage.getItem(LOG_HISTORY_KEY);
    const value: unknown = raw === null ? null : JSON.parse(raw);
    if (typeof value === "number" && (LOG_HISTORY_CHOICES as readonly number[]).includes(value)) return value;
  } catch (error) {
    console.error("could not read the log history size", error);
  }
  return DEFAULT_LOG_HISTORY_MB;
}

export function saveLogHistoryMb(mb: number, storage: Storage = settingsStorage): void {
  try {
    storage.setItem(LOG_HISTORY_KEY, JSON.stringify(mb));
  } catch (error) {
    console.error("could not save the log history size", error);
  }
}

/** Megabytes as bytes, for `spoolMaxBytes`. */
export const megabytes = (mb: number): number => mb * 1024 * 1024;
