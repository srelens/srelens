import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useMemo } from "react";
import {
  logLineHealth,
  logLineLevel,
  logLineWords,
  parseLogFields,
  parseLogQuery,
  type QueryLine,
} from "@srelens/core";
import type { Verdict } from "../../lib/regexTrial";

/**
 * The trial is the worker's to run, so here it is a promise the test settles.
 * Settling writes the verdict to the memory first, as the real one does: the
 * hook reads the memory when it renders.
 */
const h = vi.hoisted(() => ({
  memory: new Map<string, "ok" | "slow">(),
  trials: [] as {
    source: string;
    flags: string;
    texts: readonly string[];
    settle: (verdict: "ok" | "slow") => void;
  }[],
}));
vi.mock("../../lib/regexTrial", () => ({
  cachedVerdict: (source: string, flags: string) => h.memory.get(`${flags}/${source}`),
  trialRegex: (source: string, flags: string, texts: readonly string[]) =>
    new Promise<Verdict>((resolve) => {
      h.trials.push({
        source,
        flags,
        texts,
        settle: (verdict) => {
          h.memory.set(`${flags}/${source}`, verdict);
          resolve(verdict);
        },
      });
    }),
}));

import { useRegexGate } from "./useRegexGate";

/** A line the way the Logs screen builds one from a stamp-stripped line. */
function ql(text: string): QueryLine {
  const { message, fields } = parseLogFields(text);
  const health = logLineHealth(text);
  return {
    text,
    haystack: `${text} ${health}`.toLowerCase(),
    words: logLineWords(message),
    fields,
    level: logLineLevel(text) ?? "",
    health,
  };
}

const LINES = [ql("info starting up"), ql("error pool timeout"), ql("aaaaaaaa")];

/** The hook over a query typed as text, parsed the way the screen does: once per text. */
function gate(initial: string, lines: readonly QueryLine[] = LINES) {
  return renderHook(
    ({ query, rows }) => useRegexGate(useMemo(() => parseLogQuery(query), [query]), rows),
    { initialProps: { query: initial, rows: lines } },
  );
}

/** Settle the trial at `at`, and let the hook see it. */
async function settle(at: number, verdict: "ok" | "slow") {
  await act(async () => h.trials[at].settle(verdict));
}

const sources = (clauses: readonly { value: string }[]) => clauses.map((c) => c.value);

beforeEach(() => {
  h.memory.clear();
  h.trials.length = 0;
});

describe("useRegexGate", () => {
  it("gives a query with no regex its clauses at once, and never starts a trial", () => {
    const { result } = gate("level:error pool");
    expect(sources(result.current.clauses)).toEqual(["error", "pool"]);
    expect(result.current.slow).toBeUndefined();
    expect(h.trials).toHaveLength(0);
  });

  it("holds a regex query back, on the empty list at first, until its trial says ok", async () => {
    const { result } = gate("/a+b/");
    expect(result.current.clauses).toEqual([]);
    expect(h.trials).toHaveLength(1);
    await settle(0, "ok");
    expect(sources(result.current.clauses)).toEqual(["a+b"]);
    expect(result.current.slow).toBeUndefined();
  });

  it("holds it back on the clauses the screen was filtering by, not on none", async () => {
    const { result, rerender } = gate("level:error");
    const before = result.current.clauses;
    rerender({ query: "level:error /a+b/", rows: LINES });
    expect(result.current.clauses).toBe(before);
    await settle(0, "ok");
    expect(sources(result.current.clauses)).toEqual(["error", "a+b"]);
  });

  it("keeps the previous clauses and names the pattern when its trial says slow", async () => {
    const { result, rerender } = gate("level:error");
    const before = result.current.clauses;
    rerender({ query: "level:error /(a+)+$/", rows: LINES });
    await settle(0, "slow");
    expect(result.current.clauses).toBe(before);
    expect(result.current.slow).toBe("/(a+)+$/");
  });

  it("clears the alert once the query changes to one that is fine", async () => {
    const { result, rerender } = gate("/(a+)+$/");
    await settle(0, "slow");
    expect(result.current.slow).toBe("/(a+)+$/");
    rerender({ query: "pool", rows: LINES });
    expect(result.current.slow).toBeUndefined();
    expect(sources(result.current.clauses)).toEqual(["pool"]);
  });

  it("names the slow pattern even while another regex is still on trial", async () => {
    const { result } = gate("/a+b/ /(a+)+$/");
    expect(h.trials.map((t) => t.source)).toEqual(["a+b", "(a+)+$"]);
    await settle(1, "slow");
    expect(result.current.slow).toBe("/(a+)+$/");
    expect(result.current.clauses).toEqual([]);
  });

  it("applies the query only when every one of its regexes has passed", async () => {
    const { result } = gate("/a+b/ /c+d/");
    await settle(0, "ok");
    expect(result.current.clauses).toEqual([]);
    await settle(1, "ok");
    expect(sources(result.current.clauses)).toEqual(["a+b", "c+d"]);
  });

  it("sends a free-text regex each line's text", () => {
    gate("/a+b/");
    expect(h.trials[0].texts).toEqual(["info starting up", "error pool timeout", "aaaaaaaa"]);
    expect(h.trials[0].flags).toBe("i");
  });

  it("sends a field's regex that field's values, from the lines that have it", () => {
    gate("msg:/a+b/", [ql('{"msg":"one"}'), ql("no fields here"), ql('{"msg":"two","level":"info"}')]);
    expect(h.trials[0].texts).toEqual(["one", "two"]);
  });

  it("sends the level word to a level regex", () => {
    gate("level:/err/");
    // The third line names no level, so it has no value to test.
    expect(h.trials[0].texts).toEqual(["info", "error"]);
  });

  it("applies a regex tried before at once, with no trial", () => {
    h.memory.set("i/a+b", "ok");
    const { result } = gate("/a+b/");
    expect(sources(result.current.clauses)).toEqual(["a+b"]);
    expect(h.trials).toHaveLength(0);
  });

  it("refuses a regex found slow before at once, with no trial", () => {
    h.memory.set("i/(a+)+$", "slow");
    const { result } = gate("/(a+)+$/");
    expect(result.current.clauses).toEqual([]);
    expect(result.current.slow).toBe("/(a+)+$/");
    expect(h.trials).toHaveLength(0);
  });

  it("does not start the same trial again when lines arrive or the rest of the query changes", () => {
    const { rerender } = gate("/a+b/");
    rerender({ query: "/a+b/", rows: [...LINES, ql("one more")] });
    rerender({ query: "/a+b/ pool", rows: [...LINES, ql("one more")] });
    expect(h.trials).toHaveLength(1);
  });

  it("does not start a pattern's trial again when a second regex joins it while it runs", () => {
    const { rerender } = gate("/a+b/");
    rerender({ query: "/a+b/ /c+d/", rows: LINES });
    expect(h.trials.map((t) => t.source)).toEqual(["a+b", "c+d"]);
  });

  it("is not moved by the trial of a pattern the query has since left", async () => {
    const { result, rerender } = gate("/a+b/");
    rerender({ query: "/c+d/", rows: LINES });
    await settle(0, "ok");
    expect(result.current.clauses).toEqual([]);
    await settle(1, "ok");
    expect(sources(result.current.clauses)).toEqual(["c+d"]);
  });

  it("returns the empty list, with no alert, for a query that does not parse, as the screen does without the gate", async () => {
    const { result, rerender } = gate("level:error");
    expect(sources(result.current.clauses)).toEqual(["error"]);
    rerender({ query: "/(/", rows: LINES });
    expect(result.current.clauses).toEqual([]);
    expect(result.current.slow).toBeUndefined();
    expect(h.trials).toHaveLength(0);
  });

  // A restored query meets an empty buffer: the stream has not sent a line yet. A trial over nothing finishes at once,
  // and its "ok" would be remembered for the session and let the slow pattern through when the lines do arrive.
  it("waits for a line before it trials a regex, then trials it over the lines that came", async () => {
    const { result, rerender } = gate("/a+b/", []);
    expect(result.current.clauses).toEqual([]);
    expect(h.trials).toHaveLength(0);
    rerender({ query: "/a+b/", rows: LINES });
    expect(h.trials).toHaveLength(1);
    expect(h.trials[0].texts).toHaveLength(3);
    await settle(0, "ok");
    expect(sources(result.current.clauses)).toEqual(["a+b"]);
  });
});
