import { describe, it, expect } from "vitest";
import { clauseState, discoverFields, facetCounts, highlightRanges, inlineFields, inRange, isQueryableKey, logLineWords, matchLine, parseLogFields, parseLogQuery, removeClause, toggleClause, volumeBuckets, type QueryClause, type QueryLine } from "./logQuery";
import { healthRank, logLineHealth, logLineLevel } from "./logTerms";

describe("parseLogFields", () => {
  it("takes a JSON logger's msg as the message and keeps every key as a field", () => {
    expect(
      parseLogFields('{"level":"info","msg":"Wrote batch","system":"camlog","internal_use":true,"n":3,"x":null}'),
    ).toEqual({
      message: "Wrote batch",
      fields: { level: "info", msg: "Wrote batch", system: "camlog", internal_use: "true", n: "3", x: "null" },
      structured: true,
    });
  });

  it("falls back to `message` when there is no `msg`", () => {
    expect(parseLogFields('{"message":"hi","a":"b"}').message).toBe("hi");
  });

  it("flattens nested objects to dotted keys and keeps arrays whole", () => {
    expect(
      parseLogFields('{"msg":"m","http":{"status":502,"route":{"name":"pay"}},"tags":["a","b"]}').fields,
    ).toEqual({ msg: "m", "http.status": "502", "http.route.name": "pay", tags: '["a","b"]' });
  });

  it("still structures a JSON line with no message, so the row can show its fields", () => {
    expect(parseLogFields('{"level":"warn","system":"camlog"}')).toEqual({
      message: "",
      fields: { level: "warn", system: "camlog" },
      structured: true,
    });
  });

  it("reads whole-line logfmt and takes its msg", () => {
    expect(parseLogFields('time=2026-10-12T10:33:03Z level=info msg="Wrote batch" system=camlog')).toEqual({
      message: "Wrote batch",
      fields: { time: "2026-10-12T10:33:03Z", level: "info", msg: "Wrote batch", system: "camlog" },
      structured: true,
    });
  });

  it("reads klog's structured tail and keeps the line as written", () => {
    const text = 'I1012 10:33:03.123456       1 sync.go:42] "Starting sync" node="worker-1" pod="kube-system/coredns-7d"';
    expect(parseLogFields(text)).toEqual({
      message: text,
      fields: { node: "worker-1", pod: "kube-system/coredns-7d" },
      structured: false,
    });
  });

  it("unescapes a quoted logfmt value", () => {
    expect(parseLogFields('a=1 msg="said \\"hi\\" twice"').fields.msg).toBe('said "hi" twice');
  });

  it("keeps a prose line with a logfmt tail as written, and still reads its fields", () => {
    expect(parseLogFields("error pool timeout waited=30.0s in_use=5")).toEqual({
      message: "error pool timeout waited=30.0s in_use=5",
      fields: { waited: "30.0s", in_use: "5" },
      structured: false,
    });
  });

  it("treats a single pair as prose, not logfmt", () => {
    expect(parseLogFields("info starting checkout-api build=4f2a1c")).toEqual({
      message: "info starting checkout-api build=4f2a1c",
      fields: {},
      structured: false,
    });
  });

  it("treats malformed JSON as plain text", () => {
    expect(parseLogFields('{"level":"info","msg":')).toEqual({
      message: '{"level":"info","msg":',
      fields: {},
      structured: false,
    });
  });

  it("keeps a __proto__ key as a field instead of losing it to the prototype", () => {
    expect(parseLogFields('{"msg":"m","__proto__":"x"}').fields["__proto__"]).toBe("x");
  });

  it("does not throw on a JSON object nested 5000 deep, and still structures it", () => {
    const text = '{"a":'.repeat(5000) + "1" + "}".repeat(5000);
    expect(parseLogFields(text).structured).toBe(true);
  });

  it("does not throw on a JSON array nested 5000 deep", () => {
    const text = '{"a":' + "[".repeat(5000) + "]".repeat(5000) + "}";
    expect(parseLogFields(text).structured).toBe(true);
  });

  it("stores an object past the depth cap as its JSON text", () => {
    expect(parseLogFields('{"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":{"j":1}}}}}}}}}}').fields).toEqual({
      "a.b.c.d.e.f.g.h": '{"i":{"j":1}}',
    });
  });

  it("reads the pairs before an unterminated quote without rescanning the line", () => {
    expect(parseLogFields('a=1 b="' + '\\"'.repeat(20000)).fields.a).toBe("1");
  });

  it("keeps a logfmt __proto__ key as a field", () => {
    expect(parseLogFields("__proto__=x a=1").fields["__proto__"]).toBe("x");
  });

  it("takes the first non-blank message key", () => {
    expect(parseLogFields('{"msg":"  ","message":"hi"}').message).toBe("hi");
  });

  it("reads only the trailing run of pairs, so a pair before a word is not a field", () => {
    expect(parseLogFields("a=1 foo b=2")).toEqual({
      message: "a=1 foo b=2",
      fields: {},
      structured: false,
    });
  });
});

describe("logLineWords", () => {
  it("normalizes a message the way the term tally reads it", () => {
    expect(logLineWords('Success: Wrote "batch", 12 rows.')).toBe("success wrote batch 12 rows");
  });
});

describe("inlineFields", () => {
  it("lists every field except those with a column of their own, quoting spaced values", () => {
    expect(
      inlineFields({
        level: "info",
        msg: "m",
        time: "t",
        "@timestamp": "t",
        system: "camlog",
        note: "two words",
        x_ref_number: "",
      }),
    ).toBe('system=camlog note="two words" x_ref_number=');
  });
});

describe("healthRank", () => {
  it("ranks danger above warning above info above neutral", () => {
    expect(healthRank("danger")).toBeGreaterThan(healthRank("warning"));
    expect(healthRank("warning")).toBeGreaterThan(healthRank("info"));
    expect(healthRank("info")).toBeGreaterThan(healthRank("neutral"));
  });

  it("ranks success between info and neutral", () => {
    expect(healthRank("info")).toBeGreaterThan(healthRank("success"));
    expect(healthRank("success")).toBeGreaterThan(healthRank("neutral"));
  });
});

/** A QueryLine the way the Logs screen builds one from a stamp-stripped line. */
function ql(text: string, at?: number): QueryLine {
  const { message, fields } = parseLogFields(text);
  const health = logLineHealth(text);
  return {
    text,
    haystack: `${text} ${health}`.toLowerCase(),
    words: logLineWords(message),
    fields,
    level: logLineLevel(text) ?? "",
    health,
    ...(at === undefined ? {} : { at }),
  };
}

function clauses(query: string): readonly QueryClause[] {
  const parsed = parseLogQuery(query);
  if (!parsed.ok) throw new Error(`did not parse: ${parsed.error}`);
  return parsed.clauses;
}

const matches = (query: string, ...texts: string[]) =>
  texts
    .map((t) => ql(t))
    .filter((l) => matchLine(l, clauses(query)))
    .map((l) => l.text);

describe("parseLogQuery", () => {
  it("reads every clause form, with the span each one occupies", () => {
    const q = 'timeout "wrote batch" /err(or)?/ system:camlog note:"two words" ref:"" app:/^cid/ -level:debug';
    expect(
      clauses(q).map((c) => [c.field ?? null, c.match, c.value, c.negated, q.slice(c.span[0], c.span[1])]),
    ).toEqual([
      [null, "text", "timeout", false, "timeout"],
      [null, "phrase", "wrote batch", false, '"wrote batch"'],
      [null, "regex", "err(or)?", false, "/err(or)?/"],
      ["system", "equals", "camlog", false, "system:camlog"],
      ["note", "equals", "two words", false, 'note:"two words"'],
      ["ref", "equals", "", false, 'ref:""'],
      ["app", "regex", "^cid", false, "app:/^cid/"],
      ["level", "equals", "debug", true, "-level:debug"],
    ]);
  });

  it("lowercases what it compares, but keeps a field name and a regex as typed", () => {
    expect(clauses("Timeout System:CamLog /ABC/").map((c) => [c.field ?? null, c.value])).toEqual([
      [null, "timeout"],
      ["System", "camlog"],
      [null, "ABC"],
    ]);
  });

  it("reads a colon that does not follow a field name as text", () => {
    expect(clauses("10:33:03").map((c) => [c.field ?? null, c.match])).toEqual([[null, "text"]]);
  });

  it.each([
    ['"wrote batch', "This quote is never closed", 0],
    ["pool /err(", "This regex is never closed", 5],
    ["/(/", "Invalid regex", 0],
    ["a -", "A '-' needs something to exclude", 2],
    ["system: x", "A field needs a value after ':'", 7],
    ['"a"b', "Put a space after a quote or a regex", 3],
    ["//", "An empty regex", 0],
    ['""', "An empty phrase", 0],
  ] as const)("refuses %j", (query, error, at) => {
    const parsed = parseLogQuery(query);
    expect(parsed.ok).toBe(false);
    if (parsed.ok) return;
    expect(parsed.error).toContain(error);
    expect(parsed.at).toBe(at);
  });
});

describe("matchLine", () => {
  const A = '{"level":"info","msg":"Wrote batch","system":"camlog"}';
  const B = '{"level":"warn","msg":"Skipping checkpoint","system":"schwarz"}';
  const C = '{"level":"error","msg":"Write failed: timeout","system":"camlog"}';
  const D = "GET /healthz 200 1ms";

  it("ANDs free text and different fields", () => {
    expect(matches("system:camlog timeout", A, B, C, D)).toEqual([C]);
  });

  it("ORs values of one field, so two level clicks show both", () => {
    expect(matches("level:warn level:error", A, B, C, D)).toEqual([B, C]);
  });

  it("excludes with '-', and a line without the field is not excluded", () => {
    expect(matches("-system:camlog", A, B, C, D)).toEqual([B, D]);
  });

  it("compares field names and values case-insensitively", () => {
    expect(matches("SYSTEM:CamLog", A, B)).toEqual([A]);
  });

  it("reads level off a plain-text line as well as a JSON one", () => {
    const P = "error pool timeout waited=30s in_use=5";
    expect(matches("level:error", P, "info all good", C)).toEqual([P, C]);
  });

  it("still finds core's severity word in free text, as the filter always has", () => {
    expect(matches("danger", "error pool timeout", "info fine")).toEqual(["error pool timeout"]);
  });

  it("finds a phrase across the punctuation the term tally strips", () => {
    const S = '{"msg":"Success: Wrote batch","n":1}';
    expect(matches('"success wrote batch"', S, "nothing here")).toEqual([S]);
  });

  it("anchors a regex at the line, not after the severity word appended for search", () => {
    expect(matches("/timeout$/", "error pool timeout")).toEqual(["error pool timeout"]);
  });

  it("matches a field regex against that field's value", () => {
    expect(matches("system:/^schw/", A, B)).toEqual([B]);
  });

  it("skips the ignored field's clauses, which is what a facet's own counts need", () => {
    const line = ql(B);
    expect(matchLine(line, clauses("level:error system:schwarz"))).toBe(false);
    expect(matchLine(line, clauses("level:error system:schwarz"), "level")).toBe(true);
  });

  it("never reads a field off the object prototype", () => {
    expect(matches("constructor:x", A)).toEqual([]);
  });
});

describe("isQueryableKey", () => {
  it("accepts what `field:` parses as, and nothing else", () => {
    expect(["system", "http.status", "@timestamp", "app.kubernetes.io/name", "x_ref"].map(isQueryableKey)).toEqual([true, true, true, true, true]);
    expect(["bad key", "1st", "", "a:b"].map(isQueryableKey)).toEqual([false, false, false, false]);
  });
});

describe("toggleClause", () => {
  const sys = { field: "system", value: "camlog" };

  it.each([
    ["", "include", "system:camlog"],
    ["", "exclude", "-system:camlog"],
    ["pool system:camlog", "include", "pool"],
    ["pool system:camlog", "exclude", "pool -system:camlog"],
    ["pool -system:camlog", "include", "pool system:camlog"],
    ["pool -system:camlog x", "exclude", "pool x"],
    ["pool", "include", "pool system:camlog"],
    ["pool  ", "include", "pool system:camlog"],
  ] as const)("%j, %s", (query, mode, expected) => {
    expect(toggleClause(query, sys, mode)).toBe(expected);
  });

  it("finds the clause however it was typed", () => {
    expect(toggleClause("System:CamLog", sys, "include")).toBe("");
  });

  it("quotes a value the query would otherwise misread", () => {
    expect(toggleClause("", { field: "note", value: "two words" }, "include")).toBe('note:"two words"');
    expect(toggleClause("", { field: "ref", value: "" }, "include")).toBe('ref:""');
    expect(toggleClause("", { field: "path", value: "/healthz" }, "include")).toBe('path:"/healthz"');
    expect(toggleClause("", { field: "addr", value: "10.0.0.1:443" }, "include")).toBe('addr:"10.0.0.1:443"');
  });

  it("writes a value holding a quote as an exact regex, and finds it again", () => {
    const target = { field: "q", value: 'say "hi"' };
    const once = toggleClause("", target, "include");
    expect(once).toBe('q:/^say "hi"$/');
    expect(matches(once, '{"q":"say \\"hi\\""}', '{"q":"say hi"}')).toHaveLength(1);
    expect(toggleClause(once, target, "include")).toBe("");
  });

  it("toggles a phrase for a Top term", () => {
    expect(toggleClause("x", { phrase: "Wrote batch" }, "include")).toBe('x "Wrote batch"');
    expect(toggleClause('x "wrote batch"', { phrase: "Wrote batch" }, "exclude")).toBe('x -"Wrote batch"');
  });

  it("appends to a query that does not parse rather than losing what was typed", () => {
    expect(toggleClause('"open', sys, "include")).toBe('"open system:camlog');
  });
});

describe("removeClause", () => {
  it("removes exactly one clause and closes the gap it leaves", () => {
    const q = 'a  "b c"   -d:e f';
    const [, phrase, negated] = clauses(q);
    expect(removeClause(q, phrase)).toBe("a -d:e f");
    expect(removeClause(q, negated)).toBe('a  "b c" f');
  });
});

describe("clauseState", () => {
  it("says whether a value is included, excluded or neither", () => {
    const cs = clauses("system:camlog -level:debug");
    expect(clauseState(cs, { field: "system", value: "CamLog" })).toBe("include");
    expect(clauseState(cs, { field: "level", value: "debug" })).toBe("exclude");
    expect(clauseState(cs, { field: "level", value: "info" })).toBeUndefined();
  });
});

describe("discoverFields", () => {
  const json = (o: Record<string, unknown>) => ql(JSON.stringify(o));

  it("offers keys most lines carry with a handful of values, most common first", () => {
    const lines = Array.from({ length: 60 }, (_, i) =>
      json({
        level: i % 3 ? "info" : "warn",
        msg: `m${i}`,
        system: `s${i % 3}`,
        app: "syncer",
        id: `req-${i}`,
        ...(i % 4 === 0 ? { region: i % 8 ? "eu" : "us" } : {}),
        ...(i === 0 ? { rare: "x" } : {}),
        "bad key": i % 2 ? "a" : "b",
      }),
    );
    lines.push(ql("a plain line"));
    // system: every line, 3 values. region: 15 of 60 lines (≥ 20%), 2 values.
    // app has one value, id has 60, rare is on one line, `bad key` can't be
    // typed, and level/msg have columns of their own.
    expect(discoverFields(lines)).toEqual(["system", "region"]);
  });

  it("offers at most six, ties broken by name", () => {
    const keys = ["g", "f", "e", "d", "c", "b", "a"];
    const lines = Array.from({ length: 10 }, (_, i) => json(Object.fromEntries(keys.map((k) => [k, `${k}${i % 2}`]))));
    expect(discoverFields(lines)).toEqual(["a", "b", "c", "d", "e", "f"]);
  });

  it("folds a field's spellings into one, offered under the first one seen", () => {
    const lines = Array.from({ length: 10 }, (_, i) =>
      json(i < 5 ? { System: i % 2 ? "a" : "b" } : { system: i % 2 ? "a" : "b" }),
    );
    expect(discoverFields(lines)).toEqual(["System"]);
  });

  it("counts a field's spellings together toward the coverage cutoff", () => {
    const lines = Array.from({ length: 25 }, (_, i) =>
      json({
        pad: "p",
        ...(i < 3 ? { System: i % 2 ? "a" : "b" } : i < 6 ? { system: i % 2 ? "b" : "a" } : {}),
      }),
    );
    expect(discoverFields(lines)).toEqual(["System"]);
  });

  it("counts a row that carries both spellings once toward coverage", () => {
    const lines = [json({ pad: "p", Zone: "a", zone: "b" }), ...Array.from({ length: 9 }, () => json({ pad: "p" }))];
    expect(discoverFields(lines)).toEqual([]);
  });

  it("needs a fifth of the structured rows, not a tenth", () => {
    const rows = (withK: number) =>
      Array.from({ length: 20 }, (_, i) => json({ pad: "p", ...(i < withK ? { k: i % 2 ? "b" : "a" } : {}) }));
    expect(discoverFields(rows(2))).toEqual([]);
    expect(discoverFields(rows(4))).toEqual(["k"]);
  });

  it("measures coverage against the rows that have fields, not every line", () => {
    const lines = [
      json({ pad: "p", k: "a" }),
      json({ pad: "p", k: "b" }),
      ...Array.from({ length: 8 }, () => json({ pad: "p" })),
      ...Array.from({ length: 10 }, (_, i) => ql(`plain line ${i}`)),
    ];
    expect(discoverFields(lines)).toEqual(["k"]);
  });

  it("offers a field with exactly fifty values, and not one with fifty-one", () => {
    const withValues = (n: number) => Array.from({ length: n }, (_, i) => json({ v: `x${i}` }));
    expect(discoverFields(withValues(50))).toEqual(["v"]);
    expect(discoverFields(withValues(51))).toEqual([]);
  });

  it("counts case-only differences in a value as one value", () => {
    const lines = Array.from({ length: 10 }, (_, i) => json({ k: i % 2 ? "a" : "A" }));
    expect(discoverFields(lines)).toEqual([]);
  });
});

describe("facetCounts", () => {
  const lines = [
    '{"level":"info","system":"camlog"}',
    '{"level":"INFO","system":"camlog"}',
    '{"level":"warn","system":"schwarz"}',
    '{"level":"error","system":"camlog"}',
    "no fields here",
  ].map((t) => ql(t));

  it("counts a field's values among the lines the rest of the query keeps, ignoring its own clauses", () => {
    expect(facetCounts(lines, clauses("level:warn system:camlog"), "level")).toEqual([
      { value: "info", count: 2 },
      { value: "error", count: 1 },
    ]);
  });

  it("counts another field with every clause applied", () => {
    expect(facetCounts(lines, clauses("level:warn"), "system")).toEqual([{ value: "schwarz", count: 1 }]);
  });

  it("orders equal counts alphabetically, whichever spelling arrived first", () => {
    const tied = ['{"k":"Beta"}', '{"k":"alpha"}'].map((t) => ql(t));
    expect(facetCounts(tied, clauses(""), "k")).toEqual([
      { value: "alpha", count: 1 },
      { value: "Beta", count: 1 },
    ]);
  });
});

describe("volumeBuckets", () => {
  const timed = (text: string, iso: string) => ql(text, Date.parse(iso));

  it("picks the smallest round step that fits the span in sixty bars", () => {
    // Ten minutes: 10s bars would be 61, 15s bars are 41.
    const v = volumeBuckets([timed("info a", "2026-10-12T10:00:00Z"), timed("info b", "2026-10-12T10:10:00Z")]);
    expect(v.step).toBe(15_000);
    expect(v.buckets).toHaveLength(41);
  });

  it("stacks each bucket by core's severity and counts the lines with no time", () => {
    const v = volumeBuckets([
      timed("error a", "2026-10-12T10:00:01Z"),
      timed("warn b", "2026-10-12T10:00:02Z"),
      timed("info c", "2026-10-12T10:00:03Z"),
      timed("info d", "2026-10-12T10:00:03.500Z"),
      ql("untimed line"),
    ]);
    expect(v.step).toBe(1000);
    expect(v.untimed).toBe(1);
    expect(v.buckets.map((b) => [b.danger, b.warning, b.other])).toEqual([
      [1, 0, 0],
      [0, 1, 0],
      [0, 0, 2],
    ]);
    expect(v.buckets[0].end - v.buckets[0].start).toBe(1000);
  });

  it("keeps the empty buckets in between, so a gap is drawn as a gap", () => {
    const v = volumeBuckets([timed("info a", "2026-10-12T10:00:00Z"), timed("info b", "2026-10-12T10:00:05Z")]);
    expect(v.buckets.map((b) => b.other)).toEqual([1, 0, 0, 0, 0, 1]);
  });

  it("starts an hour bar on the hour where the reader is, not in UTC", () => {
    const v = volumeBuckets([timed("info a", "2026-10-12T00:10:00Z"), timed("info b", "2026-10-13T12:00:00Z")]);
    expect(v.step).toBe(3_600_000);
    for (const b of v.buckets) expect(new Date(b.start).getMinutes()).toBe(0);
  });

  it("draws one bucket for lines all at one instant, and none when nothing is timed", () => {
    expect(volumeBuckets([timed("info a", "2026-10-12T10:00:00Z")]).buckets).toHaveLength(1);
    expect(volumeBuckets([ql("untimed")])).toEqual({ step: 0, buckets: [], untimed: 1 });
  });
});

describe("inRange", () => {
  it("keeps a timed line inside [from, to) and never an untimed one", () => {
    const r = { from: 1000, to: 2000 };
    expect([ql("a", 999), ql("b", 1000), ql("c", 1999), ql("d", 2000), ql("e")].map((l) => inRange(l, r))).toEqual([
      false,
      true,
      true,
      false,
      false,
    ]);
  });
});

const DAY_MS = 86_400_000;
const HOUR_MS = 3_600_000;

describe("volumeBuckets, untrusted stamps", () => {
  it("counts a stamp that is not a finite instant as untimed, and never throws on it", () => {
    const junk = [Number.NaN, Infinity, -Infinity, 1e300].map((at) => ql("info junk", at));
    const v = volumeBuckets([ql("info a", Date.parse("2026-10-12T10:00:00Z")), ...junk]);
    expect(v.untimed).toBe(4);
    expect(v.buckets.map((b) => b.other)).toEqual([1]);
  });

  it("keeps the bucket count within sixty when the span is wider than the widest round step", () => {
    const v = volumeBuckets([ql("info a", 0), ql("info b", Date.parse("2026-10-12T10:00:00Z"))]);
    expect(v.buckets.length).toBeLessThanOrEqual(60);
    expect(v.step % DAY_MS).toBe(0);
  });

  it("bounds the bucket count for a stamp at the edge of the date range", () => {
    const v = volumeBuckets([ql("info a", 8.64e15), ql("info b", Date.parse("2026-10-12T10:00:00Z"))]);
    expect(v.buckets.length).toBeLessThanOrEqual(60);
    expect(v.buckets.every((b) => Number.isFinite(b.start) && Number.isFinite(b.end))).toBe(true);
  });

  it("counts a stamp one past the date range as untimed", () => {
    const v = volumeBuckets([ql("info a", 8.64e15 + 1), ql("info b", Date.parse("2026-10-12T10:00:00Z"))]);
    expect(v.untimed).toBe(1);
    expect(v.buckets.length).toBe(1);
  });

  it("respects a custom maxBars", () => {
    const v = volumeBuckets(
      [ql("info a", Date.parse("2026-10-01T00:00:00Z")), ql("info b", Date.parse("2026-10-31T00:00:00Z"))],
      10,
    );
    expect(v.buckets.length).toBeLessThanOrEqual(10);
  });

  it("treats a maxBars that is not finite or below one as one bar", () => {
    const lines = [ql("info a", Date.parse("2026-10-12T10:00:00Z")), ql("info b", Date.parse("2026-10-14T10:00:00Z"))];
    for (const bad of [Number.NaN, 0, -3, Infinity]) {
      expect(volumeBuckets(lines, bad).buckets.length, `maxBars ${bad}`).toBeLessThanOrEqual(1);
    }
  });
});

/**
 * Runs `body` in `zone`, then hands back the zone the process was in. The zone is
 * restored by name: on Windows, deleting TZ leaves Node in the pinned zone.
 */
function inZone(zone: string, body: () => void): void {
  const found = Intl.DateTimeFormat().resolvedOptions().timeZone;
  process.env.TZ = zone;
  try {
    body();
  } finally {
    process.env.TZ = found;
  }
}

const at = (iso: string): number => Date.parse(iso);
const MINUTE_MS = 60_000;

/**
 * Bars are contiguous, none inverts (a skipped hour's bar is empty, start equal to end), and each bar
 * holds exactly the lines inside it. Sub-hour tests check the one-step width separately.
 */
function expectBarsHoldTheirLines(v: ReturnType<typeof volumeBuckets>, lines: readonly QueryLine[]): void {
  for (const b of v.buckets) expect(b.start).toBeLessThanOrEqual(b.end);
  for (let i = 0; i + 1 < v.buckets.length; i += 1) expect(v.buckets[i].end).toBe(v.buckets[i + 1].start);
  for (const b of v.buckets) {
    const inside = lines.filter((l) => inRange(l, { from: b.start, to: b.end })).length;
    expect(inside).toBe(b.danger + b.warning + b.other);
  }
  for (const line of lines) {
    expect(v.buckets.filter((b) => inRange(line, { from: b.start, to: b.end }))).toHaveLength(1);
  }
}

/** `count` lines, one every 25 minutes from `fromIso`, with a mix of severities. */
function lineEvery25Min(fromIso: string, count: number): QueryLine[] {
  const start = at(fromIso);
  return Array.from({ length: count }, (_, i) => {
    const word = i % 5 === 0 ? "error" : i % 3 === 0 ? "warn" : "info";
    return ql(`${word} line ${i}`, start + i * 25 * MINUTE_MS);
  });
}

describe("volumeBuckets, local time across DST", () => {
  it("starts every day bar at local midnight, and the 25-hour day of 2026-10-25 is 25 hours", () => {
    inZone("Europe/Berlin", () => {
      // Guard: the zone really is Berlin (CEST on 1 Oct, CET on 26 Oct).
      expect(new Date(at("2026-10-01T12:00:00Z")).getTimezoneOffset()).toBe(-120);
      expect(new Date(at("2026-10-26T12:00:00Z")).getTimezoneOffset()).toBe(-60);
      const v = volumeBuckets([ql("info a", at("2026-10-01T12:00:00Z")), ql("info b", at("2026-11-20T12:00:00Z"))]);
      expect(v.step).toBe(DAY_MS);
      for (const b of v.buckets) {
        const local = new Date(b.start);
        expect([local.getHours(), local.getMinutes()]).toEqual([0, 0]);
      }
      const noon = at("2026-10-25T12:00:00Z");
      const day = v.buckets.find((b) => b.start <= noon && noon < b.end);
      expect(day).toBeDefined();
      expect(day!.end - day!.start).toBe(25 * HOUR_MS);
    });
  });

  it("keeps hour bars on the hour in a half-hour zone", () => {
    inZone("Asia/Kolkata", () => {
      expect(new Date(at("2026-10-12T00:00:00Z")).getTimezoneOffset()).toBe(-330);
      const v = volumeBuckets([ql("info a", at("2026-10-12T00:10:00Z")), ql("info b", at("2026-10-13T12:00:00Z"))]);
      expect(v.step).toBe(HOUR_MS);
      for (const b of v.buckets) expect(new Date(b.start).getMinutes()).toBe(0);
    });
  });
});

describe("inRange, untrusted stamps", () => {
  it("never places a stamp that is not a finite time inside a window", () => {
    const everything = { from: Number.NEGATIVE_INFINITY, to: Number.POSITIVE_INFINITY };
    for (const stamp of [Number.NaN, Number.NEGATIVE_INFINITY, Number.POSITIVE_INFINITY, 1e300]) {
      expect(inRange(ql("info a", stamp), everything), `stamp ${stamp}`).toBe(false);
    }
  });
});

describe("volumeBuckets, two-hour bars across DST", () => {
  it("keeps every two-hour bar on an even local hour across the clocks going back", () => {
    inZone("Europe/Berlin", () => {
      expect(new Date(at("2026-10-23T12:00:00Z")).getTimezoneOffset()).toBe(-120);
      expect(new Date(at("2026-10-27T12:00:00Z")).getTimezoneOffset()).toBe(-60);
      const v = volumeBuckets([ql("info a", at("2026-10-23T12:00:00Z")), ql("info b", at("2026-10-27T12:00:00Z"))]);
      expect(v.step).toBe(2 * HOUR_MS);
      for (const b of v.buckets) {
        const local = new Date(b.start);
        expect([local.getHours() % 2, local.getMinutes()]).toEqual([0, 0]);
      }
    });
  });
});

describe("volumeBuckets, the hour the clocks go back", () => {
  it("starts the 01:00 bar at the instant that reads 01:00 before the change, 23:00 UTC", () => {
    inZone("Europe/Berlin", () => {
      const v = volumeBuckets([ql("info a", at("2026-10-24T12:00:00Z")), ql("info b", at("2026-10-26T12:00:00Z"))]);
      expect(v.step).toBe(HOUR_MS);
      expect(v.buckets.some((b) => b.start === at("2026-10-24T23:00:00Z"))).toBe(true);
    });
  });
});

describe("volumeBuckets, a repeated hour and a skipped hour", () => {
  it("keeps a repeated hour's bar over both copies, so a line in either copy is inside it", () => {
    inZone("Europe/Berlin", () => {
      // Guard: 00:30Z is the first 02:30 (CEST), 01:30Z the second (CET).
      expect(new Date(at("2026-10-25T00:30:00Z")).getTimezoneOffset()).toBe(-120);
      expect(new Date(at("2026-10-25T01:30:00Z")).getTimezoneOffset()).toBe(-60);
      const firstCopy = ql("info first copy", at("2026-10-25T00:30:00Z"));
      const secondCopy = ql("info second copy", at("2026-10-25T01:30:00Z"));
      const v = volumeBuckets([
        ql("info a", at("2026-10-24T12:00:00Z")),
        firstCopy,
        secondCopy,
        ql("info b", at("2026-10-26T12:00:00Z")),
      ]);
      expect(v.step).toBe(HOUR_MS);
      const bar = v.buckets.find((b) => b.other === 2);
      expect(bar?.other).toBe(2);
      expect(inRange(firstCopy, { from: bar!.start, to: bar!.end })).toBe(true);
      expect(inRange(secondCopy, { from: bar!.start, to: bar!.end })).toBe(true);
    });
  });

  it("leaves a skipped hour as an empty bar, so bars stay contiguous and a line sits in the bar that counts it", () => {
    inZone("Europe/Berlin", () => {
      // Guard: the clocks jump from 02:00 CET to 03:00 CEST at 01:00Z on 29 March.
      expect(new Date(at("2026-03-29T00:30:00Z")).getTimezoneOffset()).toBe(-60);
      expect(new Date(at("2026-03-29T01:30:00Z")).getTimezoneOffset()).toBe(-120);
      // Distinct severities, so the bar that counts each line can be found by its count.
      const lineB = ql("warn b", at("2026-03-29T00:30:00Z")); // 01:30 CET, just before the gap
      const lineC = ql("error c", at("2026-03-29T01:30:00Z")); // 03:30 CEST, just after the gap
      const v = volumeBuckets([
        ql("info a", at("2026-03-28T12:00:00Z")),
        lineB,
        lineC,
        ql("info d", at("2026-03-30T12:00:00Z")),
      ]);
      expect(v.step).toBe(HOUR_MS);
      for (let i = 0; i + 1 < v.buckets.length; i += 1) {
        expect(v.buckets[i].start).toBeLessThanOrEqual(v.buckets[i].end);
        expect(v.buckets[i].end).toBe(v.buckets[i + 1].start);
      }
      const warned = v.buckets.find((b) => b.warning === 1);
      const errored = v.buckets.find((b) => b.danger === 1);
      expect(warned).toBeDefined();
      expect(errored).toBeDefined();
      expect(inRange(lineB, { from: warned!.start, to: warned!.end })).toBe(true);
      expect(inRange(lineC, { from: errored!.start, to: errored!.end })).toBe(true);
    });
  });
});

describe("volumeBuckets, zones above UTC+12", () => {
  it("keeps a repeated hour's bar over both copies at Pacific/Auckland (NZDT +13 to NZST +12)", () => {
    inZone("Pacific/Auckland", () => {
      // Guard: 13:30Z on 4 April is NZDT (+13); 14:30Z is the second copy of 02:30, NZST (+12).
      expect(new Date(at("2026-04-04T13:30:00Z")).getTimezoneOffset()).toBe(-780);
      expect(new Date(at("2026-04-04T14:30:00Z")).getTimezoneOffset()).toBe(-720);
      const lines = [
        ql("info a", at("2026-04-03T12:00:00Z")),
        ql("info first copy", at("2026-04-04T13:30:00Z")),
        ql("info second copy", at("2026-04-04T14:30:00Z")),
        ql("info d", at("2026-04-05T12:00:00Z")),
      ];
      const v = volumeBuckets(lines);
      expect(v.step).toBe(HOUR_MS);
      expectBarsHoldTheirLines(v, lines);
    });
  });

  it("leaves a skipped hour at Pacific/Auckland as an empty bar, with the line before it in its bar", () => {
    inZone("Pacific/Auckland", () => {
      // Guard: 02:00 to 03:00 on 27 September does not exist; 13:30Z on 26 September is 01:30 NZST.
      expect(new Date(at("2026-09-26T13:30:00Z")).getTimezoneOffset()).toBe(-720);
      expect(new Date(at("2026-09-26T14:30:00Z")).getTimezoneOffset()).toBe(-780);
      const lines = [
        ql("info a", at("2026-09-25T12:00:00Z")),
        ql("info b", at("2026-09-26T13:30:00Z")),
        ql("info c", at("2026-09-27T12:00:00Z")),
      ];
      const v = volumeBuckets(lines);
      expect(v.step).toBe(HOUR_MS);
      expectBarsHoldTheirLines(v, lines);
    });
  });
});

describe("volumeBuckets, sub-hour bars on absolute time", () => {
  it("keeps a 20-hour span across a fall-back in 30-minute bars that each hold their own lines", () => {
    inZone("Europe/Berlin", () => {
      // Guard: 00:30Z and 01:30Z are the two copies of 02:30 local on 25 October.
      expect(new Date(at("2026-10-25T00:30:00Z")).getTimezoneOffset()).toBe(-120);
      expect(new Date(at("2026-10-25T01:30:00Z")).getTimezoneOffset()).toBe(-60);
      const lines = lineEvery25Min("2026-10-24T16:00:00Z", 49); // 16:00Z to 12:00Z, 20 hours
      const v = volumeBuckets(lines);
      expect(v.step).toBe(30 * MINUTE_MS);
      expectBarsHoldTheirLines(v, lines);
      for (const b of v.buckets) expect(b.end - b.start).toBe(30 * MINUTE_MS);
    });
  });

  it("keeps a 20-hour span across a spring-forward in 30-minute bars that each hold their own lines", () => {
    inZone("Europe/Berlin", () => {
      // Guard: the clocks jump from 02:00 CET to 03:00 CEST at 01:00Z on 29 March.
      expect(new Date(at("2026-03-29T00:30:00Z")).getTimezoneOffset()).toBe(-60);
      expect(new Date(at("2026-03-29T01:30:00Z")).getTimezoneOffset()).toBe(-120);
      const lines = lineEvery25Min("2026-03-28T16:00:00Z", 49);
      const v = volumeBuckets(lines);
      expect(v.step).toBe(30 * MINUTE_MS);
      expectBarsHoldTheirLines(v, lines);
      for (const b of v.buckets) expect(b.end - b.start).toBe(30 * MINUTE_MS);
    });
  });

  it("starts every 30-minute bar on :00 or :30 local time in a half-hour zone", () => {
    inZone("Asia/Kolkata", () => {
      expect(new Date(at("2026-10-12T00:10:00Z")).getTimezoneOffset()).toBe(-330);
      const lines = lineEvery25Min("2026-10-12T00:10:00Z", 49);
      const v = volumeBuckets(lines);
      expect(v.step).toBe(30 * MINUTE_MS);
      for (const b of v.buckets) expect([0, 30]).toContain(new Date(b.start).getMinutes());
    });
  });
});

describe("volumeBuckets, a zone whose offset is not a multiple of the step", () => {
  it("still starts every 30-minute bar on :00 or :30 local time at +05:45", () => {
    inZone("Asia/Kathmandu", () => {
      // +05:45 is not a multiple of 30 minutes, so a UTC grid would start bars on :15 and :45 local.
      expect(new Date(at("2026-10-12T00:10:00Z")).getTimezoneOffset()).toBe(-345);
      const lines = lineEvery25Min("2026-10-12T00:10:00Z", 49);
      const v = volumeBuckets(lines);
      expect(v.step).toBe(30 * MINUTE_MS);
      for (const b of v.buckets) expect([0, 30]).toContain(new Date(b.start).getMinutes());
    });
  });
});

describe("volumeBuckets, zone pinning", () => {
  it("hands the zone back as it found it", () => {
    const found = Intl.DateTimeFormat().resolvedOptions().timeZone;
    const offsetBefore = new Date(at("2026-10-12T10:00:00Z")).getTimezoneOffset();
    inZone("Asia/Kolkata", () => {
      expect(new Date(at("2026-10-12T10:00:00Z")).getTimezoneOffset()).toBe(-330);
    });
    expect(new Date(at("2026-10-12T10:00:00Z")).getTimezoneOffset()).toBe(offsetBefore);
    expect(Intl.DateTimeFormat().resolvedOptions().timeZone).toBe(found);
  });
});

describe("highlightRanges", () => {
  it("marks free text, phrases and regex matches, case-insensitively", () => {
    expect(highlightRanges("Pool timeout: pool saturated", clauses('pool "TIMEOUT" /sat\\w+/'))).toEqual([
      [0, 4],
      [5, 12],
      [14, 18],
      [19, 28],
    ]);
  });

  it("marks a field clause where a row shows it inline", () => {
    expect(
      highlightRanges('Wrote batch  system=camlog note="a b"', clauses('system:camlog note:"a b"')),
    ).toEqual([
      [13, 26],
      [27, 37],
    ]);
  });

  it("merges overlaps, and marks nothing for a negation or the level", () => {
    expect(highlightRanges("abcdef level=info", clauses("abc bcd -ef level:info"))).toEqual([[0, 4]]);
  });

  it("survives a regex that matches the empty string", () => {
    expect(highlightRanges("abc", clauses("/x*/"))).toEqual([]);
  });

  it("marks a field only as a whole key, never inside a longer key", () => {
    expect(highlightRanges("id=5 request_id=5", clauses("id:5"))).toEqual([[0, 4]]);
  });

  it("marks a field only as a whole value, never a prefix of a longer one", () => {
    expect(highlightRanges("system=camlogger system=camlog", clauses("system:camlog"))).toEqual([[17, 30]]);
  });

  it("marks a value shown JSON-quoted, as inlineFields writes it", () => {
    const row = inlineFields({ path: "C:\\a b" });
    expect(row).toBe('path="C:\\\\a b"');
    expect(highlightRanges(row, clauses('path:"C:\\a b"'))).toEqual([[0, row.length]]);
  });

  it("marks an empty value only where it is shown as a bare key=", () => {
    expect(highlightRanges("key=abc", clauses('key:""'))).toEqual([]);
    expect(highlightRanges("key= n=1", clauses('key:""'))).toEqual([[0, 4]]);
  });

  it("marks a phrase as the term tally reads it, when its words are not literal", () => {
    expect(highlightRanges("Success: Wrote batch now", clauses('"success wrote batch"'))).toEqual([[0, 20]]);
  });

  it("does not mark a phrase inside a longer word", () => {
    expect(highlightRanges("wrote batches", clauses('"wrote batch"'))).toEqual([]);
  });

  it("marks a phrase whose words the writer quoted, as tokenize reads them", () => {
    expect(highlightRanges('say "wrote" batch', clauses('"wrote batch"'))).toEqual([[4, 17]]);
  });
});
