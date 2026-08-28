import { describe, it, expect, beforeEach } from "vitest";
import { defaultMark, getMark, setMark, resetMark, loadMarks, useMark, MARKS_KEY } from "./marks";

function fakeStorage() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k), m };
}

describe("marks", () => {
  it("derives initials", () => {
    expect(defaultMark("prod-eu").short).toBe("PE");
    expect(defaultMark("staging").short).toBe("ST");
    expect(defaultMark("a-b-c-d").short).toBe("AB");
  });
  it("persists a set mark and reads it back after a reload", () => {
    const s = fakeStorage();
    loadMarks(s);
    setMark("prod", { ...defaultMark("prod-eu"), color: "var(--ok)" }, s);
    expect(JSON.parse(s.m.get(MARKS_KEY)!).prod.color).toBe("var(--ok)");
    loadMarks(fakeStorage()); // forget
    loadMarks(s);
    expect(getMark("prod", "prod-eu").color).toBe("var(--ok)");
  });
  it("keeps which symbol a symbol mark chose", () => {
    // The glyph is an id out of the app's own catalogue, not a component, so
    // it is the one part of a symbol mark that has to survive the round trip.
    const s = fakeStorage();
    loadMarks(s);
    setMark("prod", { ...defaultMark("prod-eu"), mark: "icon", icon: "server" }, s);
    loadMarks(fakeStorage()); // forget
    loadMarks(s);
    expect(getMark("prod", "prod-eu")).toMatchObject({ mark: "icon", icon: "server" });
  });
  it("drops one unreadable mark on its own, leaving the rest", () => {
    // Losing one cluster's colour is a nuisance; losing every cluster's is not.
    const s = fakeStorage();
    const good = { ...defaultMark("staging"), color: "var(--mark-teal)" };
    s.m.set(MARKS_KEY, JSON.stringify({ prod: { ...good, mark: "hologram" }, staging: good }));
    loadMarks(s);
    expect(getMark("prod", "prod-eu")).toEqual(defaultMark("prod-eu"));
    expect(getMark("staging", "staging").color).toBe("var(--mark-teal)");
  });
  it("resets to the default and survives a throwing storage", () => {
    const s = fakeStorage();
    loadMarks(s);
    setMark("prod", { ...defaultMark("prod-eu"), short: "ZZ" }, s);
    resetMark("prod", s);
    expect(getMark("prod", "prod-eu").short).toBe("PE");
    const bad = { getItem: () => { throw new Error("no"); }, setItem: () => { throw new Error("no"); }, removeItem: () => {} };
    expect(() => loadMarks(bad)).not.toThrow();
    expect(() => setMark("x", defaultMark("x"), bad)).not.toThrow();
  });
});

describe("marks the shell reads", () => {
  beforeEach(() => loadMarks(fakeStorage()));

  it("lets a customised name outlive a rename, and follows the kubeconfig until there is one", () => {
    const s = fakeStorage();
    loadMarks(s);
    // Nothing stored: the mark is called whatever the kubeconfig calls it, and
    // it keeps up when that changes.
    expect(getMark("prod", "prod-eu").name).toBe("prod-eu");
    expect(getMark("prod", "prod-eu-1").name).toBe("prod-eu-1");

    // Once the operator has typed a display name it is theirs, not a cache of
    // the context's — the editor's name field would otherwise revert every
    // keystroke to the kubeconfig's.
    setMark("prod", { ...defaultMark("prod-eu"), name: "Production EU", color: "var(--ok)" }, s);
    const after = getMark("prod", "prod-eu-1");
    expect(after.name).toBe("Production EU");
    expect(after.color).toBe("var(--ok)");
    expect(after.short).toBe("PE");
  });

  it("returns the same object until the mark changes", () => {
    // `useSyncExternalStore` re-renders forever on a snapshot that is a fresh
    // object every read, and every unstored cluster reads a fresh default.
    const s = fakeStorage();
    loadMarks(s);
    expect(getMark("prod", "prod-eu")).toBe(getMark("prod", "prod-eu"));
    setMark("prod", { ...defaultMark("prod-eu"), color: "var(--ok)" }, s);
    expect(getMark("prod", "prod-eu")).toBe(getMark("prod", "prod-eu"));
  });

  it("ignores a document that is not a map of marks", () => {
    const s = fakeStorage();
    for (const raw of ["[]", "null", "7", "{oops", '{"prod":3}']) {
      s.m.set(MARKS_KEY, raw);
      loadMarks(s);
      expect(getMark("prod", "prod-eu").color).toBe("var(--mark-indigo)");
    }
  });

  it("re-renders a subscriber when its mark is set and reset", async () => {
    const { renderHook, act } = await import("@testing-library/react");
    const s = fakeStorage();
    loadMarks(s);
    const { result } = renderHook(() => useMark("prod", "prod-eu"));
    expect(result.current.color).toBe("var(--mark-indigo)");
    act(() => setMark("prod", { ...defaultMark("prod-eu"), color: "var(--ok)" }, s));
    expect(result.current.color).toBe("var(--ok)");
    act(() => resetMark("prod", s));
    expect(result.current.color).toBe("var(--mark-indigo)");
  });
});
