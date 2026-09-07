import { describe, it, expect, beforeEach } from "vitest";
import {
  SECTION_FOLDS_KEY,
  isSectionOpen,
  loadSectionFolds,
  parseStoredSectionFolds,
  setSectionOpen,
} from "./sectionFolds";

function fakeStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    m,
  };
}

const refusing = {
  getItem: () => {
    throw new Error("storage is disabled in this WebView");
  },
  setItem: () => {
    throw new Error("storage is disabled in this WebView");
  },
  removeItem: () => {},
};

describe("which detail sections a reader has opened", () => {
  beforeEach(() => loadSectionFolds(fakeStorage()));

  it("has everything shut before anyone has opened anything", () => {
    // The reader's own words: "first open should keep everything collapsed".
    // Absence is the closed state, so a kind nobody has touched, and a block
    // nobody has opened on a kind they have, both read shut.
    expect(isSectionOpen("Pod", "Conditions")).toBe(false);
    expect(isSectionOpen("Secret", "Annotations")).toBe(false);
  });

  it("remembers an opened block for the kind, and hands it back after a reload", () => {
    const s = fakeStorage();
    loadSectionFolds(s);
    setSectionOpen("Pod", "Conditions", true, { storage: s });
    loadSectionFolds(fakeStorage()); // a launch that reads a different disk
    expect(isSectionOpen("Pod", "Conditions")).toBe(false);
    loadSectionFolds(s);
    expect(isSectionOpen("Pod", "Conditions")).toBe(true);
  });

  it("remembers per kind and never across kinds", () => {
    // THE SECURITY PROPERTY. A Secret's annotations are gated because a
    // `kubectl apply`-managed Secret carries its whole data map in an
    // annotation. Opening Annotations on a Deployment says something about
    // Deployments and must say nothing at all about Secrets.
    const s = fakeStorage();
    loadSectionFolds(s);
    setSectionOpen("Deployment", "Annotations", true, { storage: s });
    expect(isSectionOpen("Deployment", "Annotations")).toBe(true);
    expect(isSectionOpen("Secret", "Annotations")).toBe(false);
    loadSectionFolds(s);
    expect(isSectionOpen("Secret", "Annotations")).toBe(false);
  });

  it("shuts a block again, and forgets a kind with nothing left open", () => {
    const s = fakeStorage();
    loadSectionFolds(s);
    setSectionOpen("Pod", "Conditions", true, { storage: s });
    setSectionOpen("Pod", "Labels", true, { storage: s });
    setSectionOpen("Pod", "Conditions", false, { storage: s });
    expect(isSectionOpen("Pod", "Conditions")).toBe(false);
    expect(isSectionOpen("Pod", "Labels")).toBe(true);
    setSectionOpen("Pod", "Labels", false, { storage: s });
    // Not an empty array left behind for every kind ever peeked at: the
    // document is the reader's choices, and a reader who has undone all of
    // theirs has none.
    expect(JSON.parse(s.m.get(SECTION_FOLDS_KEY)!)).toEqual({});
  });

  /**
   * THE RULING: a titled section that is the only content of its pane opens
   * OPEN, because a pane that opens showing nothing at all is hostile — the
   * same argument the unheaded lead fact list was exempted under. It stays
   * collapsible and stays remembered; only the starting point differs.
   *
   * Which means the document has to tell "nobody has touched this" from "the
   * reader shut it", and it could not: it recorded opened ids and nothing
   * else, so absence meant both. It records the reader's DEVIATION FROM THE
   * DEFAULT instead — the id when they open a block that starts shut, and
   * `!id` when they shut one that starts open. Absence is still the default
   * and the only default, which is the property a Secret's annotations rest
   * on: nothing in a stored document can open a block whose caller did not
   * ask for it to start open, and no caller asks that for a Secret.
   */
  describe("a block its pane cannot do without", () => {
    it("starts open when nothing is remembered about it", () => {
      expect(isSectionOpen("Pod", "Containers", true)).toBe(true);
      // And only that block: the default is the caller's, per block.
      expect(isSectionOpen("Pod", "Annotations")).toBe(false);
    });

    it("remembers the reader shutting it, across a launch", () => {
      const s = fakeStorage();
      loadSectionFolds(s);
      setSectionOpen("Pod", "Containers", false, { defaultOpen: true, storage: s });
      expect(isSectionOpen("Pod", "Containers", true)).toBe(false);
      loadSectionFolds(s);
      expect(isSectionOpen("Pod", "Containers", true)).toBe(false);
    });

    it("forgets the choice again once it matches the default", () => {
      // The document stays the size of the reader's actual deviations, the
      // same rule an opened-then-shut block already followed.
      const s = fakeStorage();
      loadSectionFolds(s);
      setSectionOpen("Pod", "Containers", false, { defaultOpen: true, storage: s });
      setSectionOpen("Pod", "Containers", true, { defaultOpen: true, storage: s });
      expect(JSON.parse(s.m.get(SECTION_FOLDS_KEY)!)).toEqual({});
      expect(isSectionOpen("Pod", "Containers", true)).toBe(true);
    });

    it("records a shut block distinctly from an opened one, so the two cannot collide", () => {
      const s = fakeStorage();
      loadSectionFolds(s);
      setSectionOpen("Pod", "Containers", false, { defaultOpen: true, storage: s });
      setSectionOpen("Pod", "Conditions", true, { storage: s });
      expect(JSON.parse(s.m.get(SECTION_FOLDS_KEY)!)).toEqual({ Pod: ["!Containers", "Conditions"] });
    });

    it("never opens a block from a record that only says shut", () => {
      // THE DIRECTION THAT MATTERS. A stored deviation can close a block that
      // starts open; nothing it can say opens a block that starts shut, so no
      // document — however old, however malformed — can disclose a Secret's
      // annotations to a reader who never opened them.
      const s = fakeStorage();
      loadSectionFolds(s);
      setSectionOpen("Secret", "Annotations", false, { defaultOpen: true, storage: s });
      expect(isSectionOpen("Secret", "Annotations")).toBe(false);
      loadSectionFolds(s);
      expect(isSectionOpen("Secret", "Annotations")).toBe(false);
    });
  });

  it("is a snapshot that cannot tear", () => {
    // A boolean, so `useSyncExternalStore` compares it by value: a composed
    // object would be a fresh one on every read and re-render forever.
    expect(typeof isSectionOpen("Pod", "Conditions")).toBe("boolean");
  });

  it("costs the memory and nothing else when storage refuses", () => {
    // `settingsStorage` falls back to raw `localStorage`, which throws
    // outright in a WebView with storage disabled. Boot must still finish.
    expect(() => loadSectionFolds(refusing)).not.toThrow();
    expect(isSectionOpen("Pod", "Conditions")).toBe(false);
    expect(() => setSectionOpen("Pod", "Conditions", true, { storage: refusing })).not.toThrow();
    // Still true for this session, even though nothing could be written.
    expect(isSectionOpen("Pod", "Conditions")).toBe(true);
  });
});

describe("reading a stored fold document", () => {
  it("reads anything unparseable as nothing stored", () => {
    expect(parseStoredSectionFolds(null)).toEqual({});
    expect(parseStoredSectionFolds("")).toEqual({});
    expect(parseStoredSectionFolds("{oh no")).toEqual({});
    expect(parseStoredSectionFolds("[]")).toEqual({});
    expect(parseStoredSectionFolds('"Pod"')).toEqual({});
  });

  it("drops one unreadable kind on its own rather than taking the others with it", () => {
    // Losing one kind's folds is a nuisance; losing every kind's is not.
    expect(
      parseStoredSectionFolds('{"Pod":["Conditions"],"Deployment":42,"Node":["Info",7,null]}'),
    ).toEqual({ Pod: ["Conditions"], Node: ["Info"] });
  });

  it("never reads a value that is not a list of ids as \"everything is open\"", () => {
    // The one migration that must never exist. A `true` — or an object, or a
    // number — is an entry this build cannot read, and the answer to that is
    // closed, which is the answer that keeps a Secret's annotations out of
    // the document.
    const folds = parseStoredSectionFolds('{"Secret":true,"ConfigMap":{"Annotations":true}}');
    expect(folds).toEqual({});
    expect(folds.Secret).toBeUndefined();
  });

  it("drops a kind whose list is empty, so the shut state has exactly one spelling", () => {
    expect(parseStoredSectionFolds('{"Pod":[]}')).toEqual({});
  });
});
