import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage } from "@srelens/core";

const setSkillActive = vi.hoisted(() => vi.fn());
vi.mock("./agentRun", () => ({ setSkillActive }));
const refreshSetting = vi.hoisted(() => vi.fn());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), refreshSetting }));

import {
  SKILL_DEFAULTS_KEY,
  __resetSkillDefaultsForTests,
  applySkillDefaults,
  estimateTokens,
  formatTokens,
  getSkillDefaults,
  parseSkillDefaults,
  refreshSkillDefaults,
  setSkillDefault,
  skillDefaultsSaved,
} from "./skillDefaults";

beforeEach(() => {
  settingsStorage.removeItem(SKILL_DEFAULTS_KEY);
  __resetSkillDefaultsForTests();
  setSkillActive.mockReset();
  refreshSetting.mockReset().mockResolvedValue(undefined);
});

/** Another window keeping its own choice, seen when this one looks again. */
function anotherWindowKeeps(names: string[]): void {
  refreshSetting.mockImplementationOnce(async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(names));
  });
}

describe("parseSkillDefaults", () => {
  it("is no names for nothing, for junk, and for the wrong shape", () => {
    for (const raw of [null, "", "{not json", '{"a":1}', '"text"', "7"]) {
      expect(parseSkillDefaults(raw)).toEqual([]);
    }
  });

  it("keeps the names, once each, and drops what is not a name", () => {
    expect(parseSkillDefaults(JSON.stringify(["oomkilled", 3, "", null, "oomkilled", "pending-pod"]))).toEqual([
      "oomkilled",
      "pending-pod",
    ]);
  });
});

describe("parseSkillDefaults, for a document kept as on and off", () => {
  it("reads the names that were turned on, and nothing else", () => {
    expect(parseSkillDefaults(JSON.stringify({ on: ["team-runbook", 3, "team-runbook"], off: ["oomkilled"] }))).toEqual([
      "team-runbook",
    ]);
    expect(parseSkillDefaults(JSON.stringify({ off: ["oomkilled"] }))).toEqual([]);
  });
});

describe("the kept skills", () => {
  it("start as none", () => {
    expect(getSkillDefaults()).toEqual([]);
  });

  it("keep a skill turned on, across a restart", async () => {
    setSkillDefault("oomkilled", true);
    expect(getSkillDefaults()).toEqual(["oomkilled"]);
    await skillDefaultsSaved();
    __resetSkillDefaultsForTests();
    expect(getSkillDefaults()).toEqual(["oomkilled"]);
  });

  it("forget a skill turned off, and only that one", async () => {
    setSkillDefault("oomkilled", true);
    setSkillDefault("pending-pod", true);
    setSkillDefault("oomkilled", false);
    await skillDefaultsSaved();
    __resetSkillDefaultsForTests();
    expect(getSkillDefaults()).toEqual(["pending-pod"]);
  });

  it("apply to this window's next question at once, not only after a restart", () => {
    setSkillDefault("oomkilled", true);
    expect(setSkillActive).toHaveBeenCalledExactlyOnceWith("oomkilled", true);
    setSkillDefault("oomkilled", false);
    expect(setSkillActive).toHaveBeenLastCalledWith("oomkilled", false);
  });

  it("do nothing when asked for the state they are already in", () => {
    setSkillDefault("oomkilled", false);
    setSkillDefault("oomkilled", true);
    setSkillActive.mockClear();
    setSkillDefault("oomkilled", true);
    expect(setSkillActive).not.toHaveBeenCalled();
  });

  it("hold for the session when storage refuses the write", async () => {
    const refuse = vi.spyOn(settingsStorage, "setItem").mockImplementation(() => {
      throw new Error("Settings backend is unavailable");
    });
    setSkillDefault("oomkilled", true);
    await skillDefaultsSaved();
    refuse.mockRestore();
    expect(getSkillDefaults()).toEqual(["oomkilled"]);
    expect(setSkillActive).toHaveBeenCalledWith("oomkilled", true);
  });
});

/**
 * Each window loads what is kept once and used to write its whole copy back.
 * With `oomkilled` turned on in one window and `pending-pod` then turned on in
 * another, the second write carried only `pending-pod` and the first choice
 * was gone at the next launch.
 */
describe("a choice made in another window", () => {
  it("is not undone by turning a different skill on here", async () => {
    anotherWindowKeeps(["oomkilled"]);
    setSkillDefault("pending-pod", true);
    await skillDefaultsSaved();
    expect(parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY))).toEqual(["oomkilled", "pending-pod"]);
    expect(getSkillDefaults()).toEqual(["oomkilled", "pending-pod"]);
  });

  it("is not brought back by turning a different skill off here", async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["oomkilled", "pending-pod"]));
    anotherWindowKeeps(["pending-pod"]);
    setSkillDefault("pending-pod", false);
    await skillDefaultsSaved();
    expect(parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY))).toEqual([]);
  });

  it("is shown here once this window looks again", async () => {
    anotherWindowKeeps(["oomkilled"]);
    await refreshSkillDefaults();
    expect(getSkillDefaults()).toEqual(["oomkilled"]);
    // Shown, not applied: what is on in this window's conversations is its own.
    expect(setSkillActive).not.toHaveBeenCalled();
  });

  it("does not cost this window its own choice when the backend cannot be read again", async () => {
    refreshSetting.mockRejectedValueOnce(new Error("backend gone"));
    setSkillDefault("oomkilled", true);
    await skillDefaultsSaved();
    expect(parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY))).toEqual(["oomkilled"]);
  });
});

/**
 * Keeping a change waits on the backend. A save that finishes while later
 * clicks are still waiting used to show the list as it stood before them: on,
 * then off, and the first save put the switch back on until the second landed.
 * The switch decides what a click means from what it shows, so a click in
 * that moment kept the opposite of what was meant.
 */
describe("a switch thrown while an earlier one is still being kept", () => {
  /** Hold each re-read until released, so the test decides when a save lands. */
  function heldReads() {
    const release: Array<() => void> = [];
    refreshSetting.mockImplementation(() => new Promise<void>((resolve) => release.push(resolve)));
    const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
    return {
      async land() {
        while (release.length === 0) await tick();
        release.shift()!();
        await tick();
      },
    };
  }

  it("stays as the reader left it when the earlier save lands", async () => {
    const reads = heldReads();
    setSkillDefault("oomkilled", true);
    setSkillDefault("oomkilled", false);
    expect(getSkillDefaults()).toEqual([]);

    await reads.land();
    // The first save kept it on; the reader has since turned it off.
    expect(getSkillDefaults()).toEqual([]);

    await reads.land();
    await skillDefaultsSaved();
    expect(getSkillDefaults()).toEqual([]);
    expect(parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY))).toEqual([]);
  });

  it("stays thrown when a re-read lands before it is kept", async () => {
    const reads = heldReads();
    const looked = refreshSkillDefaults();
    setSkillDefault("oomkilled", true);

    await reads.land();
    await looked;
    expect(getSkillDefaults()).toEqual(["oomkilled"]);

    await reads.land();
    await skillDefaultsSaved();
    expect(parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY))).toEqual(["oomkilled"]);
  });

  it("keeps two different switches thrown together, in the order they were thrown", async () => {
    const reads = heldReads();
    setSkillDefault("oomkilled", true);
    setSkillDefault("pending-pod", true);
    await reads.land();
    expect(getSkillDefaults()).toEqual(["oomkilled", "pending-pod"]);
    await reads.land();
    await skillDefaultsSaved();
    expect(parseSkillDefaults(settingsStorage.getItem(SKILL_DEFAULTS_KEY))).toEqual(["oomkilled", "pending-pod"]);
  });
});

describe("applySkillDefaults", () => {
  it("turns each kept skill on for the window", () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["oomkilled", "pending-pod"]));
    applySkillDefaults();
    expect(setSkillActive.mock.calls).toEqual([
      ["oomkilled", true],
      ["pending-pod", true],
    ]);
  });

  it("turns nothing on when nothing is kept", () => {
    applySkillDefaults();
    expect(setSkillActive).not.toHaveBeenCalled();
  });
});

describe("estimateTokens and formatTokens", () => {
  it("counts a quarter of the characters, rounded up", () => {
    expect(estimateTokens("")).toBe(0);
    expect(estimateTokens("abcd")).toBe(1);
    expect(estimateTokens("abcde")).toBe(2);
    expect(estimateTokens("x".repeat(4000))).toBe(1000);
  });

  it("prints a figure for scale, rounded down", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
    expect(formatTokens(1000)).toBe("1k");
    expect(formatTokens(3199)).toBe("3.1k");
    expect(formatTokens(14_783)).toBe("14k");
  });
});
