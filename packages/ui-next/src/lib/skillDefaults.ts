import { useSyncExternalStore } from "react";
import { refreshSetting, settingsStorage } from "@srelens/core";
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

/**
 * A stored document as a list of names; anything else is no names. For a short
 * while on this branch the choice was kept as `{ on, off }`, when preinstalled
 * skills started on; a document left in that shape is read for what it turned
 * on, so a build that wrote one does not lose the reader's choices.
 */
export function parseSkillDefaults(raw: string | null): string[] {
  if (!raw) return [];
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return [];
  }
  if (typeof doc === "object" && doc !== null && !Array.isArray(doc)) doc = (doc as { on?: unknown }).on;
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

function notify(): void {
  for (const listener of listeners) listener();
}

/**
 * What is kept, as the backend has it now — or, when it cannot be asked, as
 * this window's copy of the settings has it.
 */
async function latest(): Promise<readonly string[]> {
  try {
    await refreshSetting(SKILL_DEFAULTS_KEY);
  } catch {
    // Not readable again: this window's copy is the best there is.
  }
  try {
    return parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY));
  } catch {
    return [];
  }
}

/** Saves run one after another, each on top of what the one before it kept. */
let saving: Promise<void> = Promise.resolve();

/** A switch thrown and not yet kept. */
interface Change {
  name: string;
  on: boolean;
}

/** The switches thrown and not yet kept, oldest first. */
let waiting: Change[] = [];

function changed(list: readonly string[], { name, on }: Change): readonly string[] {
  if (list.includes(name) === on) return list;
  return on ? [...list, name] : list.filter((n) => n !== name);
}

/**
 * Show `kept` with every switch still waiting to be kept thrown on top of it.
 * A save or a re-read that finishes while later clicks are waiting must not
 * show the list as it was before them: the switch would flip back under the
 * reader's hand, and since it decides what a click means from what it shows,
 * the next click would keep the opposite of what they meant.
 */
function show(kept: readonly string[]): void {
  defaults = waiting.reduce(changed, kept);
  notify();
}

/**
 * Turn a skill on or off for every new conversation — and for this window's
 * next question too, so the switch does what it says without a restart.
 *
 * The switch moves at once. What is written is this one change made to the
 * list as it is kept now, not to the list this window loaded at start: another
 * window may have turned a different skill on since, and writing this window's
 * copy would silently turn that one off again.
 */
export function setSkillDefault(name: string, on: boolean): void {
  if (read().includes(name) === on) return;
  const change: Change = { name, on };
  waiting.push(change);
  defaults = changed(read(), change);
  setSkillActive(name, on);
  notify();
  saving = saving.then(async () => {
    const kept = await latest();
    const next = changed(kept, change);
    if (next !== kept) {
      try {
        settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(next));
      } catch {
        // Storage is unavailable: the choice holds for this session.
      }
    }
    waiting = waiting.filter((c) => c !== change);
    show(next);
  });
}

/**
 * Look again at what is kept, so a page that shows it shows what another
 * window chose since this one started. What is on in this window's
 * conversations does not move: that is this window's until it is restarted.
 */
export function refreshSkillDefaults(): Promise<void> {
  saving = saving.then(async () => show(await latest()));
  return saving;
}

/** Resolves when every change made so far has been kept. */
export function skillDefaultsSaved(): Promise<void> {
  return saving;
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
  saving = Promise.resolve();
  waiting = [];
  listeners.clear();
}
