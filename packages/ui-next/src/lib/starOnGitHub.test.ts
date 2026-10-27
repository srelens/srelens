import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { settingsStorage } from "@srelens/core";
import {
  NUDGE_AFTER_LAUNCHES,
  NUDGE_AFTER_MS,
  STAR_AFTER_VISIT_MS,
  STAR_FOCUS_MAX_AGE_MS,
  STAR_MIN_GAP_MS,
  STAR_REFRESH_MS,
  STAR_KEY,
  STAR_LIMITED_MS,
  __resetStarForTests,
  answerNudge,
  countLaunch,
  formatStarCount,
  getStarState,
  nudgeDue,
  parseStarState,
  refreshStarCount,
  setShowStarButton,
  useStarCountRefresh,
  visitedRepository,
  type StarState,
} from "./starOnGitHub";

const DAY = 24 * 60 * 60 * 1000;
const T0 = Date.UTC(2026, 9, 1);

function stored(): Partial<StarState> {
  return JSON.parse(settingsStorage.getItem(STAR_KEY) ?? "{}") as Partial<StarState>;
}

function seed(patch: Partial<StarState>) {
  settingsStorage.setItem(STAR_KEY, JSON.stringify(patch));
  __resetStarForTests();
}

/** A `fetch` that answers with `body`, and records what it was asked. */
function github(body: unknown, ok = true, etag: string | null = null) {
  return vi.fn(
    async (_url: RequestInfo | URL, _init?: RequestInit) =>
      ({ ok, json: async () => body, headers: new Headers(etag ? { etag } : {}) }) as Response,
  );
}

/** What a request was sent with. */
const sent = (fetcher: ReturnType<typeof github>, call = 0) =>
  (fetcher.mock.calls[call][1]?.headers ?? {}) as Record<string, string>;

beforeEach(() => {
  settingsStorage.removeItem(STAR_KEY);
  __resetStarForTests();
});

describe("parseStarState", () => {
  it("is the defaults for nothing, for junk, and for the wrong shape", () => {
    const bare = { show: true, count: null, checkedAt: 0, etag: "", retryAt: 0, launches: 0, firstLaunchAt: 0, nudged: false };
    expect(parseStarState(null)).toEqual(bare);
    expect(parseStarState("{not json")).toEqual(bare);
    expect(parseStarState("[1,2]")).toEqual({ ...bare });
    expect(parseStarState('"text"')).toEqual(bare);
  });

  it("keeps what is well formed and replaces what is not, field by field", () => {
    const parsed = parseStarState(
      JSON.stringify({ show: false, count: -3, checkedAt: "soon", launches: 4, firstLaunchAt: 9, nudged: true }),
    );
    expect(parsed).toEqual({ show: false, count: null, checkedAt: 0, etag: "", retryAt: 0, launches: 4, firstLaunchAt: 9, nudged: true });
  });

  it("shows the button unless it was turned off", () => {
    expect(parseStarState("{}").show).toBe(true);
    expect(parseStarState('{"show":"no"}').show).toBe(true);
    expect(parseStarState('{"show":false}').show).toBe(false);
  });
});

describe("formatStarCount", () => {
  it("is the number itself below a thousand", () => {
    expect(formatStarCount(0)).toBe("0");
    expect(formatStarCount(999)).toBe("999");
  });

  it("is rounded down, never up, above it", () => {
    expect(formatStarCount(1000)).toBe("1k");
    expect(formatStarCount(1234)).toBe("1.2k");
    // 1.99k is not yet 2k.
    expect(formatStarCount(1999)).toBe("1.9k");
    expect(formatStarCount(12_345)).toBe("12k");
    expect(formatStarCount(999_999)).toBe("999k");
    expect(formatStarCount(1_250_000)).toBe("1.2M");
  });
});

describe("the launch count", () => {
  it("counts a page load once, however often it is asked to", () => {
    countLaunch(T0);
    countLaunch(T0 + 5);
    expect(getStarState().launches).toBe(1);
    expect(stored().launches).toBe(1);
  });

  it("remembers the first launch and does not move it", () => {
    countLaunch(T0);
    __resetStarForTests();
    countLaunch(T0 + DAY);
    expect(getStarState()).toMatchObject({ launches: 2, firstLaunchAt: T0 });
  });
});

describe("nudgeDue", () => {
  const ready: StarState = {
    show: true, count: null, checkedAt: 0, etag: "", retryAt: 0, nudged: false,
    launches: NUDGE_AFTER_LAUNCHES, firstLaunchAt: T0,
  };
  const later = T0 + NUDGE_AFTER_MS;

  it("is due once the app has been opened enough times over enough days", () => {
    expect(nudgeDue(ready, later)).toBe(true);
  });

  it("waits for both, not either", () => {
    expect(nudgeDue({ ...ready, launches: NUDGE_AFTER_LAUNCHES - 1 }, later)).toBe(false);
    expect(nudgeDue(ready, later - 1)).toBe(false);
  });

  it("is never due again once answered", () => {
    expect(nudgeDue({ ...ready, nudged: true }, later + 100 * DAY)).toBe(false);
  });

  it("is not due when the reader has hidden the button it would point at", () => {
    expect(nudgeDue({ ...ready, show: false }, later)).toBe(false);
  });

  it("is not due before any launch was counted", () => {
    expect(nudgeDue({ ...ready, firstLaunchAt: 0 }, later)).toBe(false);
  });

  it("answering it is remembered across a restart", () => {
    answerNudge();
    __resetStarForTests();
    expect(getStarState().nudged).toBe(true);
  });
});

describe("the reader's choice to show the button", () => {
  it("is remembered across a restart", () => {
    setShowStarButton(false);
    __resetStarForTests();
    expect(getStarState().show).toBe(false);
  });

  it("holds for the session when storage refuses the write", () => {
    const refuse = vi.spyOn(settingsStorage, "setItem").mockImplementation(() => {
      throw new Error("Settings backend is unavailable");
    });
    setShowStarButton(false);
    refuse.mockRestore();
    expect(getStarState().show).toBe(false);
  });
});

/**
 * Two windows share one stored document and each holds its own copy of it. A
 * write from one must change the fields it names and leave the rest as storage
 * has them NOW, or a count arriving in one window undoes a choice made in the
 * other.
 */
describe("with a second window open", () => {
  /** What another window would do: change storage behind this one's back. */
  function elsewhere(patch: Partial<StarState>) {
    settingsStorage.setItem(STAR_KEY, JSON.stringify({ ...stored(), ...patch }));
  }

  it("a count arriving here does not undo a button hidden, or a question answered, there", async () => {
    let reply!: (r: Response) => void;
    const pending = vi.fn(() => new Promise<Response>((resolve) => (reply = resolve)));
    const asking = refreshStarCount(T0, pending as unknown as typeof fetch);
    elsewhere({ nudged: true });
    reply({ ok: true, json: async () => ({ stargazers_count: 55 }) } as Response);
    await asking;
    expect(stored()).toMatchObject({ count: 55, nudged: true });
    expect(getStarState()).toMatchObject({ count: 55, nudged: true });
  });

  it("answers again when storage no longer says so, though this window remembers answering", () => {
    answerNudge();
    expect(stored().nudged).toBe(true);
    // Another window wrote its older copy back over ours.
    elsewhere({ nudged: false });
    answerNudge();
    expect(stored().nudged).toBe(true);
  });

  it("writes nothing when both this window and storage already say answered", () => {
    answerNudge();
    const write = vi.spyOn(settingsStorage, "setItem");
    answerNudge();
    expect(write).not.toHaveBeenCalled();
    write.mockRestore();
  });

  it("counts its launch on top of the other window's", () => {
    getStarState();
    elsewhere({ launches: 7, firstLaunchAt: T0 - DAY });
    countLaunch(T0);
    expect(stored()).toMatchObject({ launches: 8, firstLaunchAt: T0 - DAY });
  });

  it("keeps a choice storage refused, rather than reading the old one back", () => {
    const refuse = vi.spyOn(settingsStorage, "setItem").mockImplementation(() => {
      throw new Error("Settings backend is unavailable");
    });
    setShowStarButton(false);
    // A later write in the same window, still refused.
    answerNudge();
    refuse.mockRestore();
    expect(getStarState()).toMatchObject({ show: false, nudged: true });
  });
});

describe("parseStarState and the ETag", () => {
  it("keeps an ETag as GitHub writes one, and nothing a header could not hold", () => {
    expect(parseStarState(JSON.stringify({ etag: 'W/"abc123"' })).etag).toBe('W/"abc123"');
    // It goes back to GitHub as a header: no spaces, no control characters, no novel.
    expect(parseStarState(JSON.stringify({ etag: "a b" })).etag).toBe("");
    expect(parseStarState(JSON.stringify({ etag: "a\nInjected: 1" })).etag).toBe("");
    expect(parseStarState(JSON.stringify({ etag: "x".repeat(201) })).etag).toBe("");
    expect(parseStarState(JSON.stringify({ etag: 7 })).etag).toBe("");
  });
});

describe("refreshStarCount", () => {
  it("asks GitHub for the repository, with no credentials, and keeps the count and its ETag", async () => {
    const fetcher = github({ stargazers_count: 1234 }, true, 'W/"v1"');
    await refreshStarCount(T0, fetcher);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(fetcher.mock.calls[0][0]).toBe("https://api.github.com/repos/srelens/srelens");
    expect(fetcher.mock.calls[0][1]).toMatchObject({ credentials: "omit" });
    expect(getStarState()).toMatchObject({ count: 1234, checkedAt: T0, etag: 'W/"v1"' });
    expect(stored().count).toBe(1234);
  });

  /**
   * #866: the count was asked for once a day, and a restart inside that day
   * reused the stored one — 194 in the app against 195 on GitHub, and no
   * restart would fix it.
   */
  it("asks again on a restart minutes later, not a day later", async () => {
    const first = github({ stargazers_count: 194 });
    await refreshStarCount(T0, first);
    // The app is closed and opened again.
    __resetStarForTests();
    const second = github({ stargazers_count: 195 });
    await refreshStarCount(T0 + 5 * 60 * 1000, second);
    expect(second).toHaveBeenCalledTimes(1);
    expect(getStarState().count).toBe(195);
  });

  it("does not ask twice within the floor, whoever is asking", async () => {
    const fetcher = github({ stargazers_count: 10 });
    await refreshStarCount(T0, fetcher);
    await refreshStarCount(T0 + STAR_MIN_GAP_MS - 1, fetcher);
    // A caller cannot talk it under the floor either.
    await refreshStarCount(T0 + STAR_MIN_GAP_MS - 1, fetcher, 0);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await refreshStarCount(T0 + STAR_MIN_GAP_MS, fetcher);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("holds off for as long as the caller is content with the count's age", async () => {
    const fetcher = github({ stargazers_count: 10 });
    await refreshStarCount(T0, fetcher);
    await refreshStarCount(T0 + STAR_FOCUS_MAX_AGE_MS - 1, fetcher, STAR_FOCUS_MAX_AGE_MS);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await refreshStarCount(T0 + STAR_FOCUS_MAX_AGE_MS, fetcher, STAR_FOCUS_MAX_AGE_MS);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("goes by when storage says the count was last asked for, which another window may have done", async () => {
    getStarState();
    settingsStorage.setItem(STAR_KEY, JSON.stringify({ count: 12, checkedAt: T0 }));
    const fetcher = github({ stargazers_count: 13 });
    await refreshStarCount(T0 + 1000, fetcher);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("asks whether the count has changed, and keeps it when GitHub says it has not", async () => {
    seed({ count: 194, etag: 'W/"v1"', checkedAt: T0 - DAY });
    const unchanged = vi.fn(async () => ({ ok: false, status: 304, headers: new Headers() }) as Response);
    await refreshStarCount(T0, unchanged as unknown as typeof fetch);
    expect((unchanged.mock.calls[0] as unknown as [string, RequestInit])[1].headers).toMatchObject({
      "If-None-Match": 'W/"v1"',
    });
    expect(getStarState()).toMatchObject({ count: 194, etag: 'W/"v1"', checkedAt: T0 });
  });

  it("replaces the count and the ETag together when it has changed", async () => {
    seed({ count: 194, etag: 'W/"v1"', checkedAt: T0 - DAY });
    await refreshStarCount(T0, github({ stargazers_count: 195 }, true, 'W/"v2"'));
    expect(getStarState()).toMatchObject({ count: 195, etag: 'W/"v2"' });
  });

  it("does not send an ETag it has no count for", async () => {
    // A 304 to it would leave the button with no number and nothing to put there.
    seed({ count: null, etag: 'W/"v1"' } as Partial<StarState>);
    const fetcher = github({ stargazers_count: 5 });
    await refreshStarCount(T0, fetcher);
    expect(sent(fetcher)["If-None-Match"]).toBeUndefined();
    expect(getStarState().count).toBe(5);
  });

  it("forgets the old ETag when the new answer carries none", async () => {
    seed({ count: 194, etag: 'W/"v1"', checkedAt: T0 - DAY });
    await refreshStarCount(T0, github({ stargazers_count: 195 }));
    expect(getStarState()).toMatchObject({ count: 195, etag: "" });
  });

  /**
   * The allowance is 60 an hour per address, shared with everyone behind it
   * and with the release notes in Settings. Once GitHub says it is spent,
   * asking again only keeps it spent.
   */
  describe("when GitHub says the limit is spent", () => {
    const limited = (status: number, reset?: number) =>
      vi.fn(async () => ({
        ok: false, status,
        headers: new Headers(reset === undefined ? {} : { "x-ratelimit-reset": String(Math.floor(reset / 1000)) }),
      }) as Response) as unknown as ReturnType<typeof github>;

    it("asks nothing until the reset time GitHub gives, then asks again", async () => {
      seed({ count: 194 });
      const reset = T0 + 20 * 60 * 1000;
      await refreshStarCount(T0, limited(403, reset));
      const fetcher = github({ stargazers_count: 195 });
      await refreshStarCount(reset - 1000, fetcher);
      expect(fetcher).not.toHaveBeenCalled();
      expect(getStarState().count).toBe(194);
      await refreshStarCount(reset, fetcher);
      expect(fetcher).toHaveBeenCalledTimes(1);
      expect(getStarState().count).toBe(195);
    });

    it("waits an hour when GitHub gives no time, on 429 as on 403", async () => {
      await refreshStarCount(T0, limited(429));
      const fetcher = github({ stargazers_count: 195 });
      await refreshStarCount(T0 + STAR_LIMITED_MS - 1, fetcher);
      expect(fetcher).not.toHaveBeenCalled();
      await refreshStarCount(T0 + STAR_LIMITED_MS, fetcher);
      expect(fetcher).toHaveBeenCalledTimes(1);
    });

    it("never waits longer than an hour, whatever time it is given", async () => {
      await refreshStarCount(T0, limited(403, T0 + 30 * DAY));
      const fetcher = github({ stargazers_count: 195 });
      await refreshStarCount(T0 + STAR_LIMITED_MS, fetcher);
      expect(fetcher).toHaveBeenCalledTimes(1);
    });

    it("is not silenced by a wait stamped by a clock that has since been put back", async () => {
      seed({ count: 194, retryAt: T0 + 30 * DAY });
      const fetcher = github({ stargazers_count: 195 });
      await refreshStarCount(T0, fetcher);
      expect(fetcher).toHaveBeenCalledTimes(1);
    });

    it("holds another window to the same wait", async () => {
      await refreshStarCount(T0, limited(403, T0 + 20 * 60 * 1000));
      __resetStarForTests();
      const fetcher = github({ stargazers_count: 195 });
      await refreshStarCount(T0 + 5 * 60 * 1000, fetcher);
      expect(fetcher).not.toHaveBeenCalled();
    });
  });

  it("keeps the last count, and does not retry within the floor, when GitHub refuses", async () => {
    seed({ count: 40, checkedAt: T0 - 2 * DAY });
    const refused = github({ message: "server error" }, false);
    await refreshStarCount(T0, refused);
    await refreshStarCount(T0 + 1000, refused);
    expect(refused).toHaveBeenCalledTimes(1);
    expect(getStarState().count).toBe(40);
  });

  it("says nothing and keeps the last count when the network fails", async () => {
    seed({ count: 40 });
    const offline = vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    });
    await expect(refreshStarCount(T0, offline as unknown as typeof fetch)).resolves.toBeUndefined();
    expect(getStarState().count).toBe(40);
  });

  it("ignores a reply that is not a star count", async () => {
    for (const body of [null, {}, { stargazers_count: "many" }, { stargazers_count: -1 }, { stargazers_count: 1.5 }]) {
      seed({ count: 7 });
      await refreshStarCount(T0, github(body));
      expect(getStarState().count).toBe(7);
    }
  });

  it("asks nothing at all when the reader has hidden the button, here or in another window", async () => {
    setShowStarButton(false);
    const fetcher = github({ stargazers_count: 10 });
    await refreshStarCount(T0, fetcher);
    expect(fetcher).not.toHaveBeenCalled();

    seed({});
    getStarState();
    settingsStorage.setItem(STAR_KEY, JSON.stringify({ show: false }));
    await refreshStarCount(T0, fetcher);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it("asks again when the clock has gone backwards past the last check", async () => {
    // A corrected clock must not leave the count frozen until the old date comes round.
    seed({ count: 7, checkedAt: T0 + 30 * DAY });
    const fetcher = github({ stargazers_count: 8 });
    await refreshStarCount(T0, fetcher);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
});

/**
 * Keeping the count current while a window is open (#866). The hook asks
 * through the real `fetch`, so that is what is replaced here.
 */
describe("useStarCountRefresh", () => {
  let fetcher: ReturnType<typeof github>;
  const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(T0);
    fetcher = github({ stargazers_count: 195 });
    vi.stubGlobal("fetch", fetcher);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("asks nothing until the window has booted, then asks at once", async () => {
    const view = renderHook(({ on }) => useStarCountRefresh(on), { initialProps: { on: false } });
    await flush();
    expect(fetcher).not.toHaveBeenCalled();
    view.rerender({ on: true });
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(getStarState().count).toBe(195);
  });

  it("asks again on its interval, for a window left open", async () => {
    renderHook(() => useStarCountRefresh(true));
    await flush();
    await act(async () => { await vi.advanceTimersByTimeAsync(STAR_REFRESH_MS); });
    expect(fetcher).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(STAR_REFRESH_MS); });
    expect(fetcher).toHaveBeenCalledTimes(3);
  });

  it("asks when the reader comes back to the window, if the count has aged", async () => {
    renderHook(() => useStarCountRefresh(true));
    await flush();
    // Back within moments: the count is fresh enough.
    vi.setSystemTime(T0 + STAR_FOCUS_MAX_AGE_MS - 1000);
    window.dispatchEvent(new Event("focus"));
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(1);
    vi.setSystemTime(T0 + STAR_FOCUS_MAX_AGE_MS);
    window.dispatchEvent(new Event("focus"));
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("asks at once on coming back from the repository, and once more a little later", async () => {
    renderHook(() => useStarCountRefresh(true));
    await flush();
    // Two minutes of use, then off to GitHub to star it and back.
    vi.setSystemTime(T0 + STAR_MIN_GAP_MS);
    visitedRepository();
    window.dispatchEvent(new Event("focus"));
    await flush();
    // Sooner than an ordinary return would ask.
    expect(STAR_MIN_GAP_MS).toBeLessThan(STAR_FOCUS_MAX_AGE_MS);
    expect(fetcher).toHaveBeenCalledTimes(2);
    // GitHub's answer may have been a minute old: one more look.
    await act(async () => { await vi.advanceTimersByTimeAsync(STAR_AFTER_VISIT_MS); });
    expect(fetcher).toHaveBeenCalledTimes(3);
    // And that is the end of it: the next return is an ordinary one.
    window.dispatchEvent(new Event("focus"));
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(3);
  });

  it("going to the repository answers the one-time question too", () => {
    visitedRepository();
    expect(getStarState().nudged).toBe(true);
  });

  it("does not ask for a window the reader cannot see", async () => {
    renderHook(() => useStarCountRefresh(true));
    await flush();
    vi.setSystemTime(T0 + DAY);
    const hidden = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(1);
    hidden.mockReturnValue("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    await flush();
    expect(fetcher).toHaveBeenCalledTimes(2);
    hidden.mockRestore();
  });

  it("stops asking when the window goes", async () => {
    const view = renderHook(() => useStarCountRefresh(true));
    await flush();
    view.unmount();
    vi.setSystemTime(T0 + DAY);
    window.dispatchEvent(new Event("focus"));
    await act(async () => { await vi.advanceTimersByTimeAsync(3 * STAR_REFRESH_MS); });
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
});
