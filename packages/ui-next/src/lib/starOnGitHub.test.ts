import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage } from "@srelens/core";
import {
  NUDGE_AFTER_LAUNCHES,
  NUDGE_AFTER_MS,
  STAR_COUNT_MAX_AGE_MS,
  STAR_KEY,
  __resetStarForTests,
  answerNudge,
  countLaunch,
  formatStarCount,
  getStarState,
  nudgeDue,
  parseStarState,
  refreshStarCount,
  setShowStarButton,
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
function github(body: unknown, ok = true) {
  return vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => ({ ok, json: async () => body }) as Response);
}

beforeEach(() => {
  settingsStorage.removeItem(STAR_KEY);
  __resetStarForTests();
});

describe("parseStarState", () => {
  it("is the defaults for nothing, for junk, and for the wrong shape", () => {
    const bare = { show: true, count: null, checkedAt: 0, launches: 0, firstLaunchAt: 0, nudged: false };
    expect(parseStarState(null)).toEqual(bare);
    expect(parseStarState("{not json")).toEqual(bare);
    expect(parseStarState("[1,2]")).toEqual({ ...bare });
    expect(parseStarState('"text"')).toEqual(bare);
  });

  it("keeps what is well formed and replaces what is not, field by field", () => {
    const parsed = parseStarState(
      JSON.stringify({ show: false, count: -3, checkedAt: "soon", launches: 4, firstLaunchAt: 9, nudged: true }),
    );
    expect(parsed).toEqual({ show: false, count: null, checkedAt: 0, launches: 4, firstLaunchAt: 9, nudged: true });
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
    show: true, count: null, checkedAt: 0, nudged: false,
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

describe("refreshStarCount", () => {
  it("asks GitHub for the repository, with no credentials, and keeps the count", async () => {
    const fetcher = github({ stargazers_count: 1234 });
    await refreshStarCount(T0, fetcher);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(fetcher.mock.calls[0][0]).toBe("https://api.github.com/repos/srelens/srelens");
    expect(fetcher.mock.calls[0][1]).toMatchObject({ credentials: "omit" });
    expect(getStarState()).toMatchObject({ count: 1234, checkedAt: T0 });
    expect(stored().count).toBe(1234);
  });

  it("does not ask again the same day, and does the day after", async () => {
    const fetcher = github({ stargazers_count: 10 });
    await refreshStarCount(T0, fetcher);
    await refreshStarCount(T0 + STAR_COUNT_MAX_AGE_MS - 1, fetcher);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await refreshStarCount(T0 + STAR_COUNT_MAX_AGE_MS, fetcher);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("keeps the last count, and does not retry that day, when GitHub refuses", async () => {
    seed({ count: 40, checkedAt: T0 - 2 * DAY });
    const refused = github({ message: "rate limited" }, false);
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

  it("asks nothing at all when the reader has hidden the button", async () => {
    setShowStarButton(false);
    const fetcher = github({ stargazers_count: 10 });
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
