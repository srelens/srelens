import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";

/**
 * The "Star on GitHub" button's state (#850): whether the button is shown, how
 * many stars the repository has, and whether the one-time nudge is due.
 *
 * Everything here is the reader's own preference or a public number. Nothing
 * records whether they starred — the app cannot know without their GitHub
 * identity, and must not try.
 */

/** Where the button goes. Built here and nowhere else, so it is a constant
 *  `openExternal` is handed and never a string from anywhere. */
export const REPO_URL = "https://github.com/srelens/srelens";
export const ISSUES_URL = `${REPO_URL}/issues/new/choose`;
const REPO_API = "https://api.github.com/repos/srelens/srelens";

export const STAR_KEY = "srelens.next.starOnGitHub";

/** Asked at most once a day, whether or not the last asking worked: a network
 *  that refuses GitHub would otherwise be asked again at every launch. */
export const STAR_COUNT_MAX_AGE_MS = 24 * 60 * 60 * 1000;

/** The nudge waits until the app has been useful: this many launches AND this
 *  long since the first one. A prompt on first launch asks for praise from
 *  someone who has not used the thing yet. */
export const NUDGE_AFTER_LAUNCHES = 5;
export const NUDGE_AFTER_MS = 3 * 24 * 60 * 60 * 1000;

export interface StarState {
  /** The reader's choice to see the button in the top bar. On until turned off. */
  show: boolean;
  /** The last star count GitHub gave, or `null` if it never has. */
  count: number | null;
  /** When the count was last asked for, successfully or not. */
  checkedAt: number;
  launches: number;
  /** When this install first counted a launch. */
  firstLaunchAt: number;
  /** The nudge has been answered, either way. It is never shown again. */
  nudged: boolean;
}

const BARE: StarState = { show: true, count: null, checkedAt: 0, launches: 0, firstLaunchAt: 0, nudged: false };

const count = (v: unknown): number | null =>
  typeof v === "number" && Number.isInteger(v) && v >= 0 ? v : null;

/** A stored document as a state, with anything missing or malformed replaced by
 *  its default — a file from an older version, or one edited by hand. */
export function parseStarState(raw: string | null): StarState {
  if (!raw) return BARE;
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return BARE;
  }
  if (!doc || typeof doc !== "object") return BARE;
  const d = doc as Record<string, unknown>;
  return {
    show: d.show !== false,
    count: count(d.count),
    checkedAt: count(d.checkedAt) ?? 0,
    launches: count(d.launches) ?? 0,
    firstLaunchAt: count(d.firstLaunchAt) ?? 0,
    nudged: d.nudged === true,
  };
}

let state: StarState | null = null;
const listeners = new Set<() => void>();

function read(): StarState {
  if (state === null) {
    try {
      state = parseStarState(settingsStorage.getItem(STAR_KEY));
    } catch {
      state = BARE;
    }
  }
  return state;
}

/** The last write did not reach storage, so storage is behind this window. */
let unsaved = false;

/**
 * What storage holds now, which another window may have changed since this one
 * read it. Unless this window's own last write never landed: then this
 * window's copy is the newer one and storage is not consulted.
 */
function stored(): StarState {
  if (unsaved) return read();
  try {
    return parseStarState(settingsStorage.getItem(STAR_KEY));
  } catch {
    return read();
  }
}

/**
 * Change the named fields and nothing else. The patch is laid over what
 * storage holds NOW rather than over this window's copy: with two windows
 * open, a count arriving in one must not write back the `show` and `nudged`
 * the other has since changed.
 */
function write(patch: Partial<StarState>): void {
  state = { ...stored(), ...patch };
  try {
    settingsStorage.setItem(STAR_KEY, JSON.stringify(state));
    unsaved = false;
  } catch {
    // Preference storage is unavailable. The choice holds for this session,
    // which is better than a button that ignores being turned off.
    unsaved = true;
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getStarState(): StarState {
  return read();
}

export function useStarState(): StarState {
  return useSyncExternalStore(subscribe, read, read);
}

export function setShowStarButton(show: boolean): void {
  write({ show });
}

/** Whether the nudge is due. A pure function of the state and the clock. */
export function nudgeDue(s: StarState, now: number): boolean {
  return (
    s.show &&
    !s.nudged &&
    s.launches >= NUDGE_AFTER_LAUNCHES &&
    s.firstLaunchAt > 0 &&
    now - s.firstLaunchAt >= NUDGE_AFTER_MS
  );
}

let launchCounted = false;

/**
 * Count this launch, once per page load however many times the button mounts.
 * A launch is a page load: a second window is a second launch, which errs
 * toward asking a little sooner and never toward asking twice.
 */
export function countLaunch(now: number = Date.now()): void {
  if (launchCounted) return;
  launchCounted = true;
  const s = stored();
  write({ launches: s.launches + 1, firstLaunchAt: s.firstLaunchAt || now });
}

/**
 * The invitation was answered — by going to GitHub from any of the app's star
 * buttons, by "Not now", or by closing the callout. Going there before the
 * callout was ever due counts: someone who has already followed the button is
 * not asked later whether they would like to.
 */
export function answerNudge(): void {
  if (read().nudged && stored().nudged) return;
  write({ nudged: true });
}

/** `1234` as `1.2k`. Whole below a thousand, one decimal below ten thousand,
 *  none above: the figure is a sense of scale, not a count to check. */
export function formatStarCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(Math.floor(n / 100) / 10).toFixed(1).replace(/\.0$/, "")}k`;
  if (n < 1_000_000) return `${Math.floor(n / 1000)}k`;
  return `${(Math.floor(n / 100_000) / 10).toFixed(1).replace(/\.0$/, "")}M`;
}

/**
 * Ask GitHub how many stars the repository has, if it has not been asked today.
 *
 * Never throws and never reports a failure: offline, rate limited, or a network
 * that blocks GitHub all leave the button showing its label and the last count
 * it had, if any. The request is unauthenticated, carries no identifier, and is
 * the only one this feature makes.
 */
export async function refreshStarCount(
  now: number = Date.now(),
  fetcher: typeof fetch = fetch,
): Promise<void> {
  const s = read();
  if (!s.show) return;
  if (s.checkedAt > 0 && now - s.checkedAt < STAR_COUNT_MAX_AGE_MS && now >= s.checkedAt) return;
  // Stamped before the request, so a second window, or a reply that never
  // comes, does not ask again.
  write({ checkedAt: now });
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10_000);
  try {
    const response = await fetcher(REPO_API, {
      headers: { Accept: "application/vnd.github+json" },
      credentials: "omit",
      signal: controller.signal,
    });
    if (!response.ok) return;
    const body: unknown = await response.json();
    const stars = count((body as { stargazers_count?: unknown } | null)?.stargazers_count);
    if (stars !== null) write({ count: stars });
  } catch {
    // Said nowhere, by design: see above.
  } finally {
    clearTimeout(timeout);
  }
}

/** Forget the in-memory state so a test starts from storage. */
export function __resetStarForTests(): void {
  state = null;
  unsaved = false;
  launchCounted = false;
  listeners.clear();
}
