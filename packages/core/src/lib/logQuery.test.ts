import { describe, it, expect } from "vitest";
import { clauseState, discoverFields, facetCounts, inlineFields, isQueryableKey, logLineWords, matchLine, parseLogFields, parseLogQuery, removeClause, toggleClause, type QueryClause, type QueryLine } from "./logQuery";
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
