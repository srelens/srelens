import { describe, it, expect, beforeEach } from "vitest";
import {
  MAX_RECENT_LOGS,
  RECENT_LOGS_KEY,
  forgetLogSubjects,
  loadRecentLogSubjects,
  parseStoredRecents,
  recentKey,
  recentLogSubjects,
  rememberLogSubject,
  reviewRecents,
  scanKey,
  type RecentLogSubject,
  type SubjectScan,
} from "./logRecents";

function fakeStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    m,
  };
}

/** A storage that refuses every accessor, the way a WebView with storage
 *  disabled does — it throws rather than answering null. */
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

const deployment = (name: string, cluster = "prod"): RecentLogSubject => ({
  cluster,
  kind: "Deployment",
  namespace: "checkout",
  name,
});

const pod = (name: string, cluster = "prod"): RecentLogSubject => ({
  cluster,
  kind: "Pod",
  namespace: "checkout",
  name,
});

const names = (cluster = "prod") => recentLogSubjects(cluster).map((e) => e.name);

describe("the subjects a bare /logs offers", () => {
  beforeEach(() => loadRecentLogSubjects(fakeStorage()));

  it("puts the subject just opened first, and never twice", () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject(deployment("checkout-api"), s);
    rememberLogSubject(deployment("payments"), s);
    // Asserted here as well as after the third call: re-opening what is
    // already at the head is a no-op, so the order the third call leaves
    // behind is the same whether the list grows from the front or the back.
    expect(names()).toEqual(["payments", "checkout-api"]);
    rememberLogSubject(deployment("checkout-api"), s);
    expect(names()).toEqual(["checkout-api", "payments"]);
  });

  it("does not rewrite the document for a subject already at the head", () => {
    // The screen re-resolves its subject on every remount, so this runs on
    // every tab switch back to a stream.
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject(deployment("checkout-api"), s);
    const written = s.m.get(RECENT_LOGS_KEY);
    s.m.delete(RECENT_LOGS_KEY);
    rememberLogSubject(deployment("checkout-api"), s);
    expect(s.m.get(RECENT_LOGS_KEY)).toBeUndefined();
    expect(written).toBeDefined();
  });

  it("keeps only the last few, and reads them back after a reload", () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    for (let i = 0; i < MAX_RECENT_LOGS + 3; i += 1) {
      rememberLogSubject(deployment(`api-${i}`), s);
    }
    expect(recentLogSubjects("prod")).toHaveLength(MAX_RECENT_LOGS);
    expect(names()[0]).toBe(`api-${MAX_RECENT_LOGS + 2}`);
    // Oldest three pushed out.
    expect(names()).not.toContain("api-0");

    loadRecentLogSubjects(fakeStorage()); // forget
    expect(recentLogSubjects("prod")).toHaveLength(0);
    loadRecentLogSubjects(s);
    expect(names()[0]).toBe(`api-${MAX_RECENT_LOGS + 2}`);
    expect(recentLogSubjects("prod")).toHaveLength(MAX_RECENT_LOGS);
  });

  it("offers one cluster's subjects to that cluster alone", () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject(deployment("checkout-api", "prod"), s);
    rememberLogSubject(deployment("checkout-api", "staging"), s);
    expect(names("prod")).toEqual(["checkout-api"]);
    expect(names("staging")).toEqual(["checkout-api"]);
    expect(recentLogSubjects("prod")[0].cluster).toBe("prod");
    expect(names("nowhere")).toEqual([]);
  });

  it("hands back the same array until something changes", () => {
    // `useSyncExternalStore` tears down and re-renders forever on a snapshot
    // that is a fresh object every read, and this one is composed by filtering.
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject(deployment("checkout-api"), s);
    const first = recentLogSubjects("prod");
    expect(recentLogSubjects("prod")).toBe(first);
    rememberLogSubject(deployment("payments"), s);
    expect(recentLogSubjects("prod")).not.toBe(first);
    const second = recentLogSubjects("prod");
    expect(recentLogSubjects("prod")).toBe(second);
  });

  it("drops one unreadable entry on its own, leaving the rest", () => {
    const s = fakeStorage();
    s.m.set(
      RECENT_LOGS_KEY,
      JSON.stringify([
        { cluster: "prod", kind: "Deployment", namespace: "checkout", name: 7 },
        deployment("payments"),
        "not an entry at all",
        { cluster: "prod", kind: "Deployment", namespace: "checkout", name: "" },
      ]),
    );
    loadRecentLogSubjects(s);
    expect(names()).toEqual(["payments"]);
  });

  it("reads anything but a list of subjects as no subjects", () => {
    expect(parseStoredRecents(null)).toEqual([]);
    expect(parseStoredRecents("{oh no")).toEqual([]);
    expect(parseStoredRecents(JSON.stringify({ prod: [] }))).toEqual([]);
  });

  it("costs the recents and nothing else when storage refuses", () => {
    const bad = refusingStorage();
    expect(() => loadRecentLogSubjects(bad)).not.toThrow();
    expect(() => rememberLogSubject(deployment("checkout-api"), bad)).not.toThrow();
    expect(() => forgetLogSubjects([recentKey(deployment("checkout-api"))], bad)).not.toThrow();
    // The write failed, but the session still has what it just opened.
    rememberLogSubject(deployment("payments"), bad);
    expect(names()).toEqual(["payments"]);
  });

  it("forgets the subjects it is told to, and keeps the others", () => {
    const s = fakeStorage();
    loadRecentLogSubjects(s);
    rememberLogSubject(deployment("checkout-api"), s);
    rememberLogSubject(pod("checkout-api-7d7-x2mzp"), s);
    forgetLogSubjects([recentKey(pod("checkout-api-7d7-x2mzp"))], s);
    expect(names()).toEqual(["checkout-api"]);
    loadRecentLogSubjects(s);
    expect(names()).toEqual(["checkout-api"]);
  });
});

describe("what a scan says about a remembered subject", () => {
  const present: SubjectScan = { names: ["checkout-api", "payments"] };
  const podScan: SubjectScan = { names: ["checkout-api-7d7-q7v4t"] };
  const unreadable: SubjectScan = { error: true };

  it("offers nothing it has not checked yet", () => {
    const entries = [deployment("checkout-api")];
    const review = reviewRecents(entries, new Map());
    expect(review.offered).toEqual([]);
    expect(review.forget).toEqual([]);
  });

  it("offers what the cluster still has", () => {
    const entries = [deployment("checkout-api")];
    const review = reviewRecents(entries, new Map([[scanKey(entries[0]), present]]));
    expect(review.offered).toEqual([{ entry: entries[0], presence: "present" }]);
    expect(review.forget).toEqual([]);
  });

  it("keeps a workload that is gone, and says so rather than offering it", () => {
    // A Deployment's name outlives its pods: scaled to zero, moved, or
    // re-applied, it comes back under the same name — so it is worth saying
    // it is missing rather than quietly dropping it.
    const entries = [deployment("checkout-api"), deployment("payments-gone")];
    const key = scanKey(entries[0]);
    const review = reviewRecents(entries, new Map([[key, present]]));
    expect(review.offered).toEqual([
      { entry: entries[0], presence: "present" },
      { entry: entries[1], presence: "gone" },
    ]);
    expect(review.forget).toEqual([]);
  });

  it("drops a pod that is gone, and forgets it", () => {
    // A pod name carries its replica-set hash and its random suffix: once the
    // cluster has replaced it, that exact name is never coming back, and a
    // corpse left in the list would push a live workload out of the cap.
    const dead = pod("checkout-api-7d7-x2mzp");
    const alive = pod("checkout-api-7d7-q7v4t");
    const review = reviewRecents([dead, alive], new Map([[scanKey(dead), podScan]]));
    expect(review.offered).toEqual([{ entry: alive, presence: "present" }]);
    expect(review.forget).toEqual([dead]);
  });

  it("offers what it could not check rather than calling it gone", () => {
    // A list that failed is not evidence of a deletion — the same rule the
    // namespace selection's stale-check follows for a namespace list that has
    // not answered yet. Nothing is forgotten on a failure either.
    const entries = [deployment("checkout-api"), pod("checkout-api-7d7-x2mzp")];
    const scans = new Map([
      [scanKey(entries[0]), unreadable],
      [scanKey(entries[1]), unreadable],
    ]);
    const review = reviewRecents(entries, scans);
    expect(review.offered).toEqual([
      { entry: entries[0], presence: "unverified" },
      { entry: entries[1], presence: "unverified" },
    ]);
    expect(review.forget).toEqual([]);
  });

  it("scans one list per kind and namespace", () => {
    expect(scanKey(deployment("a"))).toBe(scanKey(deployment("b")));
    expect(scanKey(deployment("a"))).not.toBe(scanKey(pod("a")));
    expect(scanKey({ ...deployment("a"), namespace: "payments" })).not.toBe(scanKey(deployment("a")));
  });
});
