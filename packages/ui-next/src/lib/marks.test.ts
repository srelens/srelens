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
    expect(defaultMark("a-b-c-d").short).toBe("ABC");
  });
  it("persists a set mark and reads it back after a reload", () => {
    const s = fakeStorage();
    loadMarks(s);
    setMark("prod", { ...defaultMark("prod-eu"), color: "var(--ok)" }, s);
    expect(JSON.parse(s.m.get("srelens.contextProfiles")!).prod.color).toBe("var(--ok)");
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
      expect(getMark("prod", "prod-eu").color).toBe(defaultMark("prod-eu").color);
    }
  });

  it("re-renders a subscriber when its mark is set and reset", async () => {
    const { renderHook, act } = await import("@testing-library/react");
    const s = fakeStorage();
    loadMarks(s);
    const { result } = renderHook(() => useMark("prod", "prod-eu"));
    expect(result.current.color).toBe(defaultMark("prod-eu").color);
    act(() => setMark("prod", { ...defaultMark("prod-eu"), color: "var(--ok)" }, s));
    expect(result.current.color).toBe("var(--ok)");
    act(() => resetMark("prod", s));
    expect(result.current.color).toBe(defaultMark("prod-eu").color);
  });
});

describe("classic context identity parity", () => {
  it("uses the saved classic profile, including an image and short name", () => {
    const s = fakeStorage();
    s.m.set("srelens.contextProfiles", JSON.stringify({ prod: { displayName: "Production Europe", shortName: "PEU", color: "#123456", logo: "custom", logoUrl: "https://example.test/logo.png" } }));
    s.m.set(MARKS_KEY, JSON.stringify({ id: defaultMark("Old name") }));
    loadMarks(s);
    expect(getMark("id", "prod")).toMatchObject({ name: "Production Europe", short: "PEU", color: "#123456", mark: "image", imageSrc: "https://example.test/logo.png" });
  });
  it("keeps the legacy logo choices instead of substituting a fallback glyph", () => {
    const s = fakeStorage();
    s.m.set("srelens.contextProfiles", JSON.stringify(Object.fromEntries(["cluster", "cloud", "shield", "database", "globe"].map(logo => [logo, { logo, shortName: "C" }]))));
    loadMarks(s);
    for (const logo of ["cluster", "cloud", "shield", "database", "globe"]) {
      expect(getMark(logo, logo)).toMatchObject({ mark: "icon", icon: logo });
    }
  });
  it("writes edits back to the shared classic profile and resets both designs", () => {
    const s = fakeStorage(); loadMarks(s); getMark("id", "prod");
    setMark("id", { ...defaultMark("prod"), name: "Production", short: "PRD", color: "#abcdef", mark: "icon", icon: "cloud" }, s);
    expect(JSON.parse(s.m.get("srelens.contextProfiles")!).id).toMatchObject({ displayName: "Production", shortName: "PRD", color: "#abcdef", logo: "cloud" });
    resetMark("id", s);
    expect(JSON.parse(s.m.get("srelens.contextProfiles")!).id).toBeUndefined();
    expect(getMark("id", "prod")).toEqual(defaultMark("prod"));
  });
  it("keeps palette tokens and does not pin derived labels during appearance-only edits", () => {
    const s = fakeStorage();
    loadMarks(s);
    const original = getMark("id", "prod");
    setMark("id", { ...original, color: "var(--mark-teal)" }, s);

    expect(JSON.parse(s.m.get("srelens.contextProfiles")!).id).toMatchObject({ color: "var(--mark-teal)" });
    expect(JSON.parse(s.m.get("srelens.contextProfiles")!).id).not.toHaveProperty("displayName");
    expect(JSON.parse(s.m.get("srelens.contextProfiles")!).id).not.toHaveProperty("shortName");
    loadMarks(s);
    expect(getMark("id", "renamed-prod")).toMatchObject({
      name: "renamed-prod",
      short: defaultMark("renamed-prod").short,
      color: "var(--mark-teal)",
    });
  });
  it("uses the same generated initials as classic for long context names", () => {
    expect(defaultMark("dev-lon-nrtc-6bcb8b63").short).toBe("DLN");
  });
});

it("honours a profile reset in classic after importing it by stable ID", async () => {
  const { rememberContextMarks } = await import("./marks");
  const s = fakeStorage();
  s.m.set("srelens.contextProfiles", JSON.stringify({ prod: { displayName: "Production", shortName: "P" } }));
  loadMarks(s);
  rememberContextMarks([{ name: "prod", stableId: "id" } as import("@srelens/core").ClusterContext], s);
  s.m.set("srelens.contextProfiles", "{}");
  loadMarks(s);
  expect(getMark("id", "prod")).toEqual(defaultMark("prod"));
});

it("reads classic profiles stored by stable ID, including after a context rename", () => {
  const s = fakeStorage();
  s.m.set("srelens.contextProfiles", JSON.stringify({ "stable-id": { displayName: "Production", shortName: "PRD", logo: "cloud" } }));
  loadMarks(s);
  expect(getMark("stable-id", "config/prod")).toMatchObject({ name: "Production", short: "PRD", icon: "cloud" });
});

it.each(["", "   ", "  Production Europe  "])("matches classic display labels for %j", displayName => {
  const s = fakeStorage();
  s.m.set("srelens.contextProfiles", JSON.stringify({ id: { displayName } }));
  loadMarks(s);
  expect(getMark("id", "prod").name).toBe(displayName.trim() || "prod");
});
