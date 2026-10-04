import { describe, it, expect } from "vitest";
import { SKILL_USES_KEY, getSkillUses, loadSkillUses, parseSkillUses, recordSkillUses } from "./skillUses";

function fakeStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    m,
  };
}

/** Refuses every accessor, the way a WebView with storage disabled does. */
function refusingStorage() {
  return {
    getItem: (): string | null => {
      throw new Error("storage is disabled");
    },
    setItem: () => {
      throw new Error("storage is disabled");
    },
    removeItem: () => {
      throw new Error("storage is disabled");
    },
  };
}

describe("skill uses (#387)", () => {
  it("counts each use and keeps the count", () => {
    const storage = fakeStorage();
    loadSkillUses(storage);
    recordSkillUses(["triage"], storage);
    recordSkillUses(["triage"], storage);
    expect(getSkillUses()).toEqual({ triage: 2 });
    expect(storage.m.get(SKILL_USES_KEY)).toBe(JSON.stringify({ triage: 2 }));
  });

  it("reads the counts back from where they were kept", () => {
    const storage = fakeStorage();
    storage.setItem(SKILL_USES_KEY, JSON.stringify({ oom: 4 }));
    loadSkillUses(storage);
    expect(getSkillUses()).toEqual({ oom: 4 });
  });

  it("counts a skill named twice in one question once", () => {
    const storage = fakeStorage();
    loadSkillUses(storage);
    recordSkillUses(["a", "a", "b"], storage);
    expect(getSkillUses()).toEqual({ a: 1, b: 1 });
  });

  it("reads anything but a map of whole, non-negative counts as no counts", () => {
    expect(parseSkillUses(null)).toEqual({});
    expect(parseSkillUses("not json")).toEqual({});
    expect(parseSkillUses("[1]")).toEqual({});
    expect(parseSkillUses('{"a":-1,"b":1.5,"c":2,"d":"3"}')).toEqual({ c: 2 });
  });

  it("still counts in memory when storage refuses, and never throws out", () => {
    const storage = refusingStorage();
    expect(() => loadSkillUses(storage)).not.toThrow();
    expect(() => recordSkillUses(["triage"], storage)).not.toThrow();
    expect(getSkillUses()).toEqual({ triage: 1 });
  });
});
