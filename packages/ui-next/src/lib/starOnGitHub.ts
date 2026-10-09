import { useEffect, useSyncExternalStore } from "react";
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

/**
 * How current the count is kept (#866). It was asked for once a day, and a
 * restart inside that day reused the stored figure — so someone who starred
 * from the button came back to a count without their own star in it, and
 * restarting did not help.
 *
 * - `STAR_MIN_GAP_MS` is the floor between two requests, whoever wants one:
 *   a launch, a second window, a burst of focus changes. GitHub's own answer
 *   for a repository is cached for about a minute, so asking sooner buys
 *   nothing.
 * - `STAR_REFRESH_MS` is how often a window left open asks again.
 * - `STAR_FOCUS_MAX_AGE_MS` is how old the count may be when the reader comes
 *   back to the window before it is asked for again.
 * - `STAR_AFTER_VISIT_MS` is the second look after the reader has been to the
 *   repository: their star may not be in GitHub's cached answer yet.
 * - `STAR_LIMITED_MS` is how long nothing is asked after GitHub says the
 *   limit is spent, when it does not say until when.
 *
 * The intervals are long on purpose. An unauthenticated address is allowed 60
 * requests an hour, an office shares one address, and — measured, not assumed
 * — a `304` to the stored `ETag` still spends one of the 60 when the request
 * carries no token. The same allowance serves the release notes in Settings,
 * whose failure IS shown to the reader. So a count a few minutes old is the
 * price of not spending it: a launch, two a hour for an open window, and a
 * return after a quarter of an hour away. The `ETag` is still sent; it saves
 * the body, not the allowance.
 */
export const STAR_MIN_GAP_MS = 60 * 1000;
export const STAR_REFRESH_MS = 30 * 60 * 1000;
export const STAR_FOCUS_MAX_AGE_MS = 15 * 60 * 1000;
export const STAR_AFTER_VISIT_MS = 75 * 1000;
export const STAR_LIMITED_MS = 60 * 60 * 1000;

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
  /** The `ETag` GitHub sent with `count`, to ask "has it changed?" with. */
  etag: string;
  /** GitHub said the limit is spent: ask nothing before this time. */
  retryAt: number;
  launches: number;
  /** When this install first counted a launch. */
  firstLaunchAt: number;
  /** The nudge has been answered, either way. It is never shown again. */
  nudged: boolean;
}

const BARE: StarState = { show: true, count: null, checkedAt: 0, etag: "", retryAt: 0, launches: 0, firstLaunchAt: 0, nudged: false };

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
    // Only what a header may hold: this is sent back to GitHub as one.
    etag: typeof d.etag === "string" && /^[\x21-\x7e]{1,200}$/.test(d.etag) ? d.etag : "",
    retryAt: count(d.retryAt) ?? 0,
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
 * Ask GitHub how many stars the repository has, unless it was asked within
 * `maxAgeMs` — and never within {@link STAR_MIN_GAP_MS}, whatever is passed.
 *
 * Never throws and never reports a failure: offline, rate limited, or a network
 * that blocks GitHub all leave the button showing the last count it had, if
 * any. The request is unauthenticated, carries no identifier, and is the only
 * kind this feature makes.
 */
export async function refreshStarCount(
  now: number = Date.now(),
  fetcher: typeof fetch = fetch,
  maxAgeMs: number = STAR_MIN_GAP_MS,
): Promise<void> {
  if (!read().show) return;
  // Storage's stamp, not this window's: another window may have just asked.
  const s = stored();
  if (!s.show) return;
  const gap = Math.max(maxAgeMs, STAR_MIN_GAP_MS);
  // `now >= checkedAt`: a clock that was corrected backwards must not leave
  // the count frozen until the old date comes round.
  if (s.checkedAt > 0 && now >= s.checkedAt && now - s.checkedAt < gap) return;
  // Told to wait. Bounded, so a stamp left by a clock that has since been put
  // back cannot silence the count for longer than GitHub ever asks.
  if (now < s.retryAt && s.retryAt - now <= STAR_LIMITED_MS) return;
  // Stamped before the request, so a second window, or a reply that never
  // comes, does not ask again.
  write({ checkedAt: now });
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10_000);
  try {
    const headers: Record<string, string> = { Accept: "application/vnd.github+json" };
    // Only with a count to keep: a 304 to an ETag whose count was lost would
    // leave the button with no number and nothing to replace it.
    if (s.etag !== "" && s.count !== null) headers["If-None-Match"] = s.etag;
    const response = await fetcher(REPO_API, { headers, credentials: "omit", signal: controller.signal });
    if (response.status === 403 || response.status === 429) {
      // The address's allowance is spent — by this app, another one, or a
      // colleague behind the same address. Until GitHub's own reset time when
      // it gives one, an hour when it does not; never longer.
      const reset = Number(response.headers?.get("x-ratelimit-reset")) * 1000;
      const until = Number.isFinite(reset) && reset > now ? reset : now + STAR_LIMITED_MS;
      write({ retryAt: Math.min(until, now + STAR_LIMITED_MS) });
      return;
    }
    // 304: unchanged. The count and its ETag stand.
    if (!response.ok) return;
    const body: unknown = await response.json();
    const stars = count((body as { stargazers_count?: unknown } | null)?.stargazers_count);
    if (stars === null) return;
    write({ count: stars, etag: parseStarState(JSON.stringify({ etag: response.headers?.get("etag") ?? "" })).etag });
  } catch {
    // Said nowhere, by design: see above.
  } finally {
    clearTimeout(timeout);
  }
}

/** The reader has been sent to the repository and not yet come back. */
let visiting = false;

/**
 * The reader went to the repository from one of the app's Star buttons.
 *
 * That answers the one-time question, and it is also the moment the count is
 * most likely to be about to change by exactly one — theirs — so the next
 * return to the window asks GitHub again rather than waiting its turn.
 */
export function visitedRepository(): void {
  answerNudge();
  visiting = true;
}

/**
 * Keep the count current while a window is open (#866): asked at start, on an
 * interval, and when the reader comes back to the window — at once, and once
 * more a little later, if they have just been to the repository.
 *
 * The window's to run, once, after boot; not the button's, so that drawing the
 * button in a test or a gallery never reaches the network.
 */
export function useStarCountRefresh(active: boolean): void {
  useEffect(() => {
    if (!active) return undefined;
    void refreshStarCount();
    const every = setInterval(() => void refreshStarCount(), STAR_REFRESH_MS);
    let later: ReturnType<typeof setTimeout> | undefined;
    const back = () => {
      if (document.visibilityState === "hidden") return;
      if (!visiting) {
        void refreshStarCount(Date.now(), fetch, STAR_FOCUS_MAX_AGE_MS);
        return;
      }
      visiting = false;
      void refreshStarCount();
      // GitHub's answer is cached for about a minute, so the star just given
      // may not be in the one above. One more look, and no more than one.
      clearTimeout(later);
      later = setTimeout(() => void refreshStarCount(), STAR_AFTER_VISIT_MS);
    };
    window.addEventListener("focus", back);
    document.addEventListener("visibilitychange", back);
    return () => {
      clearInterval(every);
      clearTimeout(later);
      window.removeEventListener("focus", back);
      document.removeEventListener("visibilitychange", back);
    };
  }, [active]);
}

/** Forget the in-memory state so a test starts from storage. */
export function __resetStarForTests(): void {
  state = null;
  unsaved = false;
  visiting = false;
  launchCounted = false;
  listeners.clear();
}
