import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage } from "@srelens/core";

const setSkillActive = vi.hoisted(() => vi.fn());
vi.mock("./agentRun", () => ({ setSkillActive }));

import {
  SKILL_DEFAULTS_KEY,
  __resetSkillDefaultsForTests,
  applySkillDefaults,
  estimateTokens,
  formatTokens,
  getSkillDefaults,
  parseSkillDefaults,
  setSkillDefault,
} from "./skillDefaults";

beforeEach(() => {
  settingsStorage.removeItem(SKILL_DEFAULTS_KEY);
  __resetSkillDefaultsForTests();
  setSkillActive.mockReset();
});

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

describe("the kept skills", () => {
  it("start as none", () => {
    expect(getSkillDefaults()).toEqual([]);
  });

  it("keep a skill turned on, across a restart", () => {
    setSkillDefault("oomkilled", true);
    __resetSkillDefaultsForTests();
    expect(getSkillDefaults()).toEqual(["oomkilled"]);
  });

  it("forget a skill turned off, and only that one", () => {
    setSkillDefault("oomkilled", true);
    setSkillDefault("pending-pod", true);
    setSkillDefault("oomkilled", false);
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

  it("hold for the session when storage refuses the write", () => {
    const refuse = vi.spyOn(settingsStorage, "setItem").mockImplementation(() => {
      throw new Error("Settings backend is unavailable");
    });
    setSkillDefault("oomkilled", true);
    refuse.mockRestore();
    expect(getSkillDefaults()).toEqual(["oomkilled"]);
    expect(setSkillActive).toHaveBeenCalledWith("oomkilled", true);
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
