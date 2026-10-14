import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import {
  logLineHealth,
  logLineLevel,
  logLineWords,
  matchLine,
  parseLogFields,
  parseLogQuery,
  type ParsedQuery,
  type QueryLine,
} from "@srelens/core";
import { knownSlow, oracleFor, oracles, resetFakeOracles } from "../../lib/regexWorker.fake";
import type { MessageRow } from "./LogMessage";
import { keepMatching, useRegexGate } from "./useRegexGate";

// jsdom has no Worker: the oracles are fakes the test answers (see the fake's module).
vi.mock("../../lib/regexWorker", () => import("../../lib/regexWorker.fake"));

type Row = QueryLine & MessageRow;

/** A line the way the Logs screen builds one from a stamp-stripped line. */
function row(text: string): Row {
  const { message, fields } = parseLogFields(text);
  const health = logLineHealth(text);
  return {
    text,
    haystack: `${text} ${health}`.toLowerCase(),
    words: logLineWords(message),
    fields,
    level: logLineLevel(text) ?? "",
    health,
    message,
    structured: text.startsWith("{"),
    truncated: false,
  };
}

const LINES = [row("info starting up"), row("error pool timeout"), row("aaaaaaaa")];

const parse = (query: string) => parseLogQuery(query);

/** The hook over a parsed query and a buffer, as the screen hands them over. */
function gate(query: string | ParsedQuery, rows: readonly Row[] = LINES) {
  const applied = typeof query === "string" ? parse(query) : query;
  return renderHook((props: { applied: ParsedQuery; rows: readonly Row[] }) => useRegexGate(props.applied, props.rows), {
    initialProps: { applied, rows },
  });
}

/** The worker for `source` answers what it has been asked, and the hook sees it. */
async function reply(source: string) {
  await act(async () => oracleFor(source).reply());
}

const texts = (rows: readonly Row[]) => rows.map((r) => r.text);

beforeEach(() => resetFakeOracles());

describe("useRegexGate", () => {
  it("hands a query with no regex straight through: the same rows and clauses, and no worker", () => {
    const applied = parse("level:error pool");
    const { result } = gate(applied);
    expect(result.current.rows).toBe(LINES);
    expect(result.current.clauses).toBe(applied.ok && applied.clauses);
    expect(result.current.slow).toBeUndefined();
    expect(result.current.unavailable).toBe(false);
    expect(oracles).toHaveLength(0);
  });

  it("returns no clauses and every row for a query that does not parse, as the screen does without the gate", () => {
    const { result } = gate("/(/");
    expect(result.current.clauses).toEqual([]);
    expect(result.current.rows).toBe(LINES);
    expect(oracles).toHaveLength(0);
  });

  it("holds every row back until the worker has answered it, then gives them in order", async () => {
    const { result } = gate("/a+/");
    expect(result.current.rows).toEqual([]);
    await reply("a+");
    expect(texts(result.current.rows)).toEqual(texts(LINES));
    // The clauses read the answers: `starting` and `aaaaaaaa` match, `error pool timeout` does not.
    expect(texts(result.current.rows.filter((r) => matchLine(r, result.current.clauses)))).toEqual([
      "info starting up",
      "aaaaaaaa",
    ]);
  });

  it("asks only for the rows a new commit brings, and holds just those back", async () => {
    const applied = parse("/a+/");
    const { result, rerender } = gate(applied);
    await reply("a+");
    rerender({ applied, rows: [...LINES, row("zzz")] });
    expect(texts(result.current.rows)).toEqual(texts(LINES));
    expect(oracleFor("a+").asks.map((items) => items.map((i) => i.test))).toEqual([texts(LINES), ["zzz"]]);
    await reply("a+");
    expect(texts(result.current.rows)).toEqual([...texts(LINES), "zzz"]);
  });

  it("does not ask again when only the rest of the query changes", async () => {
    const { rerender } = gate("/a+/");
    rerender({ applied: parse("/a+/ pool"), rows: LINES });
    expect(oracles).toHaveLength(1);
    expect(oracleFor("a+").asks).toHaveLength(1);
  });

  it("asks a free-text regex with each line's text and the row as it is shown, with the clause's flags", () => {
    gate("/a+/", [row('{"msg":"Wrote batch","system":"camlog"}')]);
    expect(oracleFor("a+").flags).toBe("i");
    expect(oracleFor("a+").asks).toEqual([
      [{ test: '{"msg":"Wrote batch","system":"camlog"}', shown: "Wrote batch  system=camlog" }],
    ]);
  });

  it("asks a field's regex with that field's values, from the rows that have it, and nothing to mark", async () => {
    const rows = [row('{"msg":"one"}'), row("no fields here"), row('{"msg":"two"}')];
    const { result } = gate("msg:/o/", rows);
    expect(oracleFor("o").asks).toEqual([[{ test: "one" }, { test: "two" }]]);
    await reply("o");
    // A row with no such field has nothing to wait on.
    expect(texts(result.current.rows)).toEqual(texts(rows));
    expect(result.current.rows.filter((r) => matchLine(r, result.current.clauses))).toEqual([rows[0], rows[2]]);
  });

  it("asks a level regex with the level words", () => {
    gate("level:/err/");
    expect(oracleFor("err").asks).toEqual([[{ test: "info" }, { test: "error" }]]);
  });

  it("never runs the pattern on this thread: its clauses read the worker's answers", async () => {
    const { result } = gate("/a+/ -msg:/b/", [...LINES, row('{"msg":"b"}')]);
    await reply("a+");
    await reply("b");
    const real = { test: RegExp.prototype.test, exec: RegExp.prototype.exec };
    const ran: string[] = [];
    const test = vi.spyOn(RegExp.prototype, "test").mockImplementation(function (this: RegExp, s: string) {
      ran.push(this.source);
      return real.test.call(this, s);
    });
    const exec = vi.spyOn(RegExp.prototype, "exec").mockImplementation(function (this: RegExp, s: string) {
      ran.push(this.source);
      return real.exec.call(this, s);
    });
    try {
      const kept = result.current.rows.filter((r) => matchLine(r, result.current.clauses));
      expect(texts(kept)).toEqual(["info starting up", "aaaaaaaa"]);
    } finally {
      test.mockRestore();
      exec.mockRestore();
    }
    expect(ran.filter((source) => source === "a+" || source === "b")).toEqual([]);
  });

  it("gives a row the worker's marks for a free-text regex, and none for a negated or a field regex", async () => {
    const { result } = gate("/a+/ -/^b/ msg:/b/", [row('{"msg":"b aa"}')]);
    await reply("a+");
    await reply("^b");
    await reply("b");
    // Over the shown string `b aa`, as the worker found it.
    expect(result.current.rangesFor(result.current.rows[0])).toEqual([[2, 4]]);
  });

  it("goes back to the clauses it had, with every row, when a later batch runs too slow", async () => {
    const { result, rerender } = gate("level:error");
    const before = result.current.clauses;
    rerender({ applied: parse("level:error /(a+)+$/"), rows: LINES });
    // The first batch, the buffer, comes back in time and the query is applied.
    await reply("(a+)+$");
    expect(result.current.slow).toBeUndefined();
    // The next line is the bad kind.
    const rows = [...LINES, row("aaaaaaaaaaaaaaaaaaaaaaaaaaaa!")];
    rerender({ applied: parse("level:error /(a+)+$/"), rows });
    await act(async () => oracleFor("(a+)+$").fail("slow"));
    expect(result.current.clauses).toBe(before);
    expect(result.current.rows).toBe(rows);
    expect(result.current.slow).toBe("/(a+)+$/");
    expect(result.current.unavailable).toBe(false);
  });

  it("goes back past the half-typed queries that did not parse on the way to the regex", async () => {
    const { result, rerender } = gate("level:error");
    const before = result.current.clauses;
    rerender({ applied: parse("level:error /(a+"), rows: LINES });
    rerender({ applied: parse("level:error /(a+)+$/"), rows: LINES });
    await act(async () => oracleFor("(a+)+$").fail("slow"));
    expect(result.current.clauses).toBe(before);
  });

  it("goes back the same way, and says so, when no worker can run the regex", async () => {
    const { result, rerender } = gate("pool");
    const before = result.current.clauses;
    rerender({ applied: parse("pool /a+/"), rows: LINES });
    await act(async () => oracleFor("a+").fail("unavailable"));
    expect(result.current.clauses).toBe(before);
    expect(result.current.rows).toBe(LINES);
    expect(result.current.unavailable).toBe(true);
    expect(result.current.slow).toBeUndefined();
  });

  it("tries an unavailable regex again on the next change to the query", async () => {
    const { result, rerender } = gate("/a+/");
    await act(async () => oracleFor("a+").fail("unavailable"));
    rerender({ applied: parse("/a+/ pool"), rows: LINES });
    expect(oracles).toHaveLength(2);
    expect(result.current.unavailable).toBe(false);
  });

  it("is slow at once, asking nothing, for a pattern already found slow", () => {
    knownSlow.add("i/(a+)+$");
    const { result } = gate("/(a+)+$/");
    expect(result.current.slow).toBe("/(a+)+$/");
    expect(result.current.clauses).toEqual([]);
    expect(result.current.rows).toBe(LINES);
    expect(oracles.every((o) => o.asks.length === 0)).toBe(true);
  });

  it("clears the alert once the query changes to one that is fine", async () => {
    const { result, rerender } = gate("/(a+)+$/");
    await act(async () => oracleFor("(a+)+$").fail("slow"));
    rerender({ applied: parse("pool"), rows: LINES });
    expect(result.current.slow).toBeUndefined();
    expect(result.current.clauses.map((c) => c.value)).toEqual(["pool"]);
  });

  it("ends the old workers when the regexes change, and every worker when the screen goes", () => {
    const { rerender, unmount } = gate("/a+/");
    const first = oracleFor("a+");
    rerender({ applied: parse("/b+/"), rows: LINES });
    expect(first.disposed).toBe(true);
    const second = oracleFor("b+");
    expect(second.disposed).toBe(false);
    unmount();
    expect(second.disposed).toBe(true);
  });

  it("keeps the answers of the rows still in the buffer once its memory passes twice the buffer", async () => {
    const { rerender } = gate("/a+/", [row("one")]);
    await reply("a+");
    rerender({ applied: parse("/a+/"), rows: [row("two")] });
    await reply("a+");
    rerender({ applied: parse("/a+/"), rows: [row("three")] });
    await reply("a+");
    expect([...oracleFor("a+").answers.keys()]).toEqual(["one", "two", "three"]);
    // Three answers for a buffer of one: past twice its size, so only the buffer's strings are kept.
    rerender({ applied: parse("/a+/"), rows: [row("three")] });
    expect([...oracleFor("a+").answers.keys()]).toEqual(["three"]);
  });
});

describe("keepMatching", () => {
  function clausesOf(query: string) {
    const parsed = parse(query);
    if (!parsed.ok) throw new Error(parsed.error);
    return parsed.clauses;
  }

  it("keeps the lines that pass, each regex answered by a worker of its own that is ended after", async () => {
    const lines = [row("pool a"), row("pool b"), row("other a")];
    const kept = keepMatching(clausesOf("pool /a/"), lines);
    expect(oracleFor("a").asks).toEqual([[{ test: "pool a" }, { test: "pool b" }, { test: "other a" }]]);
    oracleFor("a").reply();
    await expect(kept).resolves.toEqual([lines[0]]);
    expect(oracles[0].disposed).toBe(true);
  });

  it("asks a buffer's worth of lines at a time, so the budget means what it does on screen", () => {
    void keepMatching(clausesOf("/a/"), Array.from({ length: 5001 }, (_, i) => row(`line ${i}`)));
    expect(oracleFor("a").asks.map((items) => items.length)).toEqual([5000, 1]);
  });

  it("refuses, in the screen's words, when a regex is too slow or cannot run", async () => {
    const slow = keepMatching(clausesOf("/(a+)+$/"), [row("aaaa!")]);
    oracleFor("(a+)+$").fail("slow");
    await expect(slow).rejects.toThrow("Too slow to run on these lines: /(a+)+$/");
    const cannot = keepMatching(clausesOf("/a/"), [row("a")]);
    oracleFor("a").fail("unavailable");
    await expect(cannot).rejects.toThrow("Regex search is unavailable here");
    expect(oracles.every((o) => o.disposed)).toBe(true);
  });
});
