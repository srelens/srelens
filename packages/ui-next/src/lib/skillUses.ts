import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * How often each skill has been used (#387) — what the Skills rail draws as
 * `used N×`.
 *
 * One use is one question actually sent while the skill is active for its run,
 * counted by `askAgent` at the moment the question leaves; a question refused or
 * abandoned before that never counts. Keyed by skill name.
 *
 * Same shape as `logRecents.ts`, and for the same reasons: `settingsStorage` by
 * default, so the desktop writes the backend's settings file and the web writes
 * `localStorage`; injectable, so tests need a Map and no platform; and every
 * accessor wrapped, because `localStorage` in a WebView with storage disabled
 * does not return null — it throws.
 */
export const SKILL_USES_KEY = "srelens.next.skillUses";

/** Anything but a map of whole, non-negative counts reads as no counts; one
 *  count this build cannot read is dropped on its own. */
export function parseSkillUses(raw: string | null): Record<string, number> {
  if (!raw) return {};
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof doc !== "object" || doc === null || Array.isArray(doc)) return {};
  const out: Record<string, number> = {};
  for (const [name, count] of Object.entries(doc)) {
    if (typeof count === "number" && Number.isSafeInteger(count) && count >= 0) out[name] = count;
  }
  return out;
}

let uses: Readonly<Record<string, number>> | null = null;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The kept counts, read without telling anyone — safe inside a render. A
 *  refusing storage costs the counts and nothing else. */
function readUses(storage: Storage): Record<string, number> {
  try {
    return parseSkillUses(storage.getItem(SKILL_USES_KEY));
  } catch (error) {
    console.error("could not read how often skills were used", error);
    return {};
  }
}

/** Read the kept counts again — in tests, as often as they like. Lazily read
 *  on first use otherwise, so nothing at boot has to remember to call this. */
export function loadSkillUses(storage: Storage = settingsStorage): void {
  uses = readUses(storage);
  emit();
}

/** These skills went with a question just sent. Each counted once per
 *  question, however often it was named. */
export function recordSkillUses(names: readonly string[], storage: Storage = settingsStorage): void {
  if (names.length === 0) return;
  if (uses === null) uses = readUses(storage);
  const next = { ...uses };
  for (const name of new Set(names)) next[name] = (next[name] ?? 0) + 1;
  uses = next;
  emit();
  try {
    storage.setItem(SKILL_USES_KEY, JSON.stringify(next));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: a count that does not
    // survive the session is better than a question that fails to send.
    console.error("could not keep how often skills were used", error);
  }
}

export function getSkillUses(): Readonly<Record<string, number>> {
  if (uses === null) uses = readUses(settingsStorage);
  return uses;
}

export function useSkillUses(): Readonly<Record<string, number>> {
  return useSyncExternalStore(subscribe, getSkillUses);
}
