import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import { setSkillActive } from "./agentRun";

/**
 * Which skills are on for every new conversation (#851).
 *
 * A skill could only be switched on for the run open right now — from the
 * agent screen's rail or the composer's `/` menu — and that choice is never
 * kept: each launch started with none. A reader who always wants the same two
 * skills picked them again every morning. This is the kept choice. It seeds
 * the window's active set when the window starts ({@link applySkillDefaults}),
 * and the per-run switches still turn a skill off or on for the moment without
 * touching it.
 */
export const SKILL_DEFAULTS_KEY = "srelens.next.skillDefaults";

/** A stored document as a list of names; anything else is no names. */
export function parseSkillDefaults(raw: string | null): string[] {
  if (!raw) return [];
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(doc)) return [];
  return [...new Set(doc.filter((name): name is string => typeof name === "string" && name !== ""))];
}

let defaults: readonly string[] | null = null;
const listeners = new Set<() => void>();

function read(): readonly string[] {
  if (defaults === null) {
    try {
      defaults = parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY));
    } catch {
      defaults = [];
    }
  }
  return defaults;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getSkillDefaults(): readonly string[] {
  return read();
}

export function useSkillDefaults(): readonly string[] {
  return useSyncExternalStore(subscribe, read, read);
}

/**
 * Turn a skill on or off for every new conversation — and for this window's
 * next question too, so the switch does what it says without a restart.
 */
export function setSkillDefault(name: string, on: boolean): void {
  const current = read();
  if (current.includes(name) === on) return;
  defaults = on ? [...current, name] : current.filter((n) => n !== name);
  try {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(defaults));
  } catch {
    // Storage is unavailable: the choice holds for this session.
  }
  setSkillActive(name, on);
  for (const listener of listeners) listener();
}

/**
 * Start the window with its kept skills on. Once, after boot, when settings
 * are readable. A name whose skill has since been deleted is harmless: the
 * agent run skips a skill it cannot load.
 */
export function applySkillDefaults(): void {
  for (const name of read()) setSkillActive(name, true);
}

/**
 * About how many tokens a text costs a model: a quarter of its characters,
 * rounded up. An estimate for sizing a choice, not a bill — every figure drawn
 * from it is marked as approximate.
 */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

/** `412`, `3.1k`, `14k` — a figure for scale, rounded down. */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(Math.floor(n / 100) / 10).toFixed(1).replace(/\.0$/, "")}k`;
  return `${Math.floor(n / 1000)}k`;
}

/** Forget the in-memory copy so a test starts from storage. */
export function __resetSkillDefaultsForTests(): void {
  defaults = null;
  listeners.clear();
}
