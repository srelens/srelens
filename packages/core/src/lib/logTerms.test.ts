import { describe, it, expect } from "vitest";
import { tallyLogTerms, logLineHealth, logLineLevel, tokenize } from "./logTerms";
import type { LogLine } from "./logBuffer";

const line = (text: string, source = ""): LogLine => ({ source, text });

describe("tallyLogTerms", () => {
  it("recovers 'pool timeout' from the design's sample line and its siblings", () => {
    // Verbatim message from docs/superpowers/specs/mock-full-design.md §15,
    // repeated with a different wait, pool size, in-use count and route each
    // time — the parts a real pool-timeout error always varies — and an
    // "error" level word out front, as a real raw line would carry.
    const lines = [
      line(
        "error pool timeout waited=30.0s pool_size=5 in_use=5 route=POST /v2/checkout/authorize",
      ),
      line("error pool timeout waited=12.4s pool_size=5 in_use=5 route=POST /v2/cart/add"),
      line("error pool timeout waited=8.9s pool_size=8 in_use=8 route=GET /v2/catalog/search"),
    ];
    expect(tallyLogTerms(lines)).toEqual([{ term: "pool timeout", count: 3, tone: "danger" }]);
  });

  it("recovers 'status=503' — a key=value pair — because it recurs identically, not because of its shape", () => {
    // Same shape as the untrusted pairs beside it (`trace_id=…`,
    // `duration=…ms`) — a word, '=', digits. What tells them apart is
    // cardinality: status=503 is the literal same token every time, the
    // other two are practically never repeated. Only cardinality can see
    // that; no per-token regex can.
    const lines = [
      line("error request failed status=503 trace_id=a1b2c3 duration=30011ms"),
      line("error request failed status=503 trace_id=f9e8d7 duration=15230ms"),
      line("error request failed status=503 trace_id=001122 duration=42009ms"),
    ];
    expect(tallyLogTerms(lines)).toEqual([{ term: "status=503", count: 3, tone: "danger" }]);
  });

  it("recovers 'pool saturated', warn-toned, from its own siblings", () => {
    const lines = [
      line("warn pool saturated, queueing request depth=18"),
      line("warn pool saturated, queueing request depth=31"),
      line("warn pool saturated, queueing request depth=9"),
    ];
    expect(tallyLogTerms(lines)).toEqual([{ term: "pool saturated", count: 3, tone: "warning" }]);
  });

  it("recovers 'liveness deadline exceeded', warn-toned, from its own siblings", () => {
    // The whole clause, not two words of it: the run ends at the comma, and
    // those three words are the same three words on both lines, so the
    // headline is as long as the thing that actually recurred.
    const lines = [
      line("warn liveness deadline exceeded, terminating"),
      line("warn liveness deadline exceeded, terminating"),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "liveness deadline exceeded", count: 2, tone: "warning" },
    ]);
  });

  it("a key=value pair shaped exactly like a trusted one, but that never repeats, still falls through to the headline", () => {
    // Same key, same shape as the status=503 case above — the only
    // difference is that this value is different every time, which is
    // exactly what a real build id would do.
    const lines = [
      line("starting build=cafeb0b1 now"),
      line("starting build=deadbeef now"),
      line("starting build=0ff1ce00 now"),
    ];
    expect(tallyLogTerms(lines)).toEqual([{ term: "starting", count: 3, tone: "neutral" }]);
  });

  it("worst tone wins when the same term appears at more than one severity", () => {
    const lines = [
      line("warn pool saturated, queueing request depth=18"),
      line("error pool saturated, queueing request depth=41"),
    ];
    expect(tallyLogTerms(lines)).toEqual([{ term: "pool saturated", count: 2, tone: "danger" }]);
  });

  it("counts most frequent first", () => {
    const lines = [
      line("warn pool saturated, queueing request depth=18"),
      line("warn pool saturated, queueing request depth=41"),
      line("error pool timeout waited=30.0s pool_size=5 in_use=5 route=POST /a"),
      line("error pool timeout waited=12.4s pool_size=5 in_use=5 route=POST /b"),
      line("error pool timeout waited=8.9s pool_size=8 in_use=8 route=GET /c"),
      line("error pool timeout waited=2.1s pool_size=5 in_use=4 route=GET /d"),
      line("error pool timeout waited=44.0s pool_size=6 in_use=6 route=POST /e"),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "pool timeout", count: 5, tone: "danger" },
      { term: "pool saturated", count: 2, tone: "warning" },
    ]);
  });

  it("a buffer of unique, unrelated lines yields nothing rather than a list of ones", () => {
    const lines = [
      line("info starting checkout-api build=4f2a1c pool_size=5 pool_timeout=30s"),
      line("info shutting down http server, draining 18 in-flight requests"),
      line("GET /healthz 200 1ms"),
      line("warn readiness probe failing, 3 consecutive 503s"),
    ];
    expect(tallyLogTerms(lines)).toEqual([]);
  });

  it("an empty buffer yields nothing", () => {
    expect(tallyLogTerms([])).toEqual([]);
  });

  it("a single occurrence does not earn a row, but a second one does", () => {
    const lines = [line("error pool timeout waited=30.0s pool_size=5 in_use=5 route=POST /x")];
    expect(tallyLogTerms(lines)).toEqual([]);
    lines.push(line("error pool timeout waited=1.0s pool_size=5 in_use=5 route=GET /y"));
    expect(tallyLogTerms(lines)).toEqual([{ term: "pool timeout", count: 2, tone: "danger" }]);
  });

  it("caps the number of terms reported", () => {
    const lines: LogLine[] = [];
    for (let i = 0; i < 12; i += 1) {
      // 12 distinct two-word leading phrases, each recurring 3 times, so
      // every one clears the recurrence threshold and only the cap decides.
      lines.push(line(`term${i} alpha count=${i}`));
      lines.push(line(`term${i} alpha count=${i + 100}`));
      lines.push(line(`term${i} alpha count=${i + 200}`));
    }
    const result = tallyLogTerms(lines);
    expect(result.length).toBeLessThanOrEqual(8);
  });

  it("honours a caller-supplied cap", () => {
    const lines: LogLine[] = [];
    for (let i = 0; i < 5; i += 1) {
      lines.push(line(`term${i} alpha count=${i}`));
      lines.push(line(`term${i} alpha count=${i + 100}`));
    }
    expect(tallyLogTerms(lines, { cap: 2 })).toHaveLength(2);
  });

  it("ignores the source tag entirely — tallying is over the message, not the tag", () => {
    const lines = [
      line("error pool timeout waited=30.0s pool_size=5 in_use=5 route=POST /x", "pod-a/api"),
      line("error pool timeout waited=1.0s pool_size=5 in_use=5 route=GET /y", "pod-b/api"),
    ];
    expect(tallyLogTerms(lines)).toEqual([{ term: "pool timeout", count: 2, tone: "danger" }]);
  });

  it("a line that opens with a variable token contributes nothing", () => {
    const lines = [line("503 errors spiking"), line("503 errors spiking again")];
    expect(tallyLogTerms(lines)).toEqual([]);
  });

  it("a leading level word is structural and does not itself count toward the two-word cap", () => {
    const lines = [line("error pool timeout waited=30.0s"), line("warn pool timeout waited=9.0s")];
    // Different level words, same headline: still "pool timeout", not
    // "error pool" / "warn pool" — and the tone is the worst of the two.
    expect(tallyLogTerms(lines)).toEqual([{ term: "pool timeout", count: 2, tone: "danger" }]);
  });
});

/**
 * The tokenizer's own contract, pinned as a table because its three quote and
 * punctuation strips were rewritten off regexes (js/polynomial-redos, #380) and
 * "identical output" is the whole of what the rewrite promised. Every row here
 * was produced by the regex version first.
 */
describe("tokenize", () => {
  const words = (text: string) => tokenize(text).map((t) => t.text);
  const clauseEnds = (text: string) => tokenize(text).map((t) => t.endsClause);

  it("splits on whitespace and strips the writer's quoting", () => {
    expect(words("pool saturated")).toEqual(["pool", "saturated"]);
    expect(words('  pool   saturated  ')).toEqual(["pool", "saturated"]);
    expect(words('"pool" saturated')).toEqual(["pool", "saturated"]);
    expect(words("'pool' `saturated`")).toEqual(["pool", "saturated"]);
    expect(words('"""pool"""')).toEqual(["pool"]);
    // A quote inside a word is part of the word — only the ends are stripped.
    expect(words(`don't stop`)).toEqual(["don't", "stop"]);
  });

  it("strips trailing sentence punctuation, and colons with it", () => {
    expect(words("pool saturated.")).toEqual(["pool", "saturated"]);
    expect(words("plugin/kubernetes: failed")).toEqual(["plugin/kubernetes", "failed"]);
    expect(words("what?! now")).toEqual(["what", "now"]);
    expect(words('saturated,"')).toEqual(["saturated"]);
    // Leading punctuation that is not a quote stays: it is part of the word.
    expect(words(".hidden -flag")).toEqual([".hidden", "-flag"]);
  });

  it("drops a token that was nothing but quotes and punctuation", () => {
    expect(words('""" ,,, `')).toEqual([]);
    expect(words("")).toEqual([]);
    expect(words("   ")).toEqual([]);
    expect(words('pool """ saturated')).toEqual(["pool", "saturated"]);
  });

  it("reads the clause end off the writer's punctuation, quotes ignored", () => {
    expect(clauseEnds("pool saturated, queueing")).toEqual([false, true, false]);
    expect(clauseEnds('pool saturated,"')).toEqual([false, true]);
    expect(clauseEnds("pool saturated;")).toEqual([false, true]);
    expect(clauseEnds("pool saturated!")).toEqual([false, true]);
    // A colon is NOT a clause end — one thought, not two.
    expect(clauseEnds("plugin/kubernetes: failed")).toEqual([false, false]);
    // A token of quotes alone ends no clause, and is not reported at all.
    expect(clauseEnds('""')).toEqual([]);
  });
});

describe("logLineHealth", () => {
  // The public surface: this is now the one place that decides a raw log
  // line's severity, for both the term tally above and any other consumer
  // (the Logs screen's LogLine level prop and its level filter). Tested in
  // its own right, not just indirectly through tallyLogTerms.

  it("reads 'error', 'fatal' and 'panic' as danger", () => {
    expect(logLineHealth("connection error: pool exhausted")).toBe("danger");
    expect(logLineHealth("fatal: liveness deadline exceeded, terminating")).toBe("danger");
    expect(logLineHealth("panic: runtime error: index out of range")).toBe("danger");
  });

  it("reads 'warn' and 'warning' as warning", () => {
    expect(logLineHealth("warn pool saturated, queueing request")).toBe("warning");
    expect(logLineHealth("WARNING: certificate expires in 6 days")).toBe("warning");
  });

  it("reads 'info' as info", () => {
    expect(logLineHealth("info starting checkout-api build=4f2a1c")).toBe("info");
  });

  it("reads anything with no recognised level word as neutral", () => {
    expect(logLineHealth("GET /healthz 200 1ms")).toBe("neutral");
    expect(logLineHealth("")).toBe("neutral");
  });

  it("is case-insensitive and matches anywhere in the line, not only a leading word", () => {
    expect(logLineHealth("14:07:41.902 ERROR pool timeout waited=30.0s")).toBe("danger");
    expect(logLineHealth("request failed status=503, see Warn budget below")).toBe("warning");
  });

  it("prefers danger over warning or info when a line somehow carries more than one", () => {
    // Not expected in practice, but the precedence should be principled
    // (worst word wins) rather than "whichever regex runs first" by luck.
    expect(logLineHealth("warn: escalated to error after 3 retries")).toBe("danger");
  });

  it("does not match a level word as a substring of an unrelated word", () => {
    // 'informant' contains 'info', 'forewarned' contains 'warn' — neither
    // should trip the level scan; the classic-derived word-boundary regexes
    // guard exactly this.
    expect(logLineHealth("the informant forewarned the team")).toBe("neutral");
  });
});

describe("logLineLevel", () => {
  // The ONE scan that decides what level word a raw log line carries — spelt
  // as the line itself spells it, for the level column. `logLineHealth`
  // above is now a consumer of this, not a second regex over the same text.

  it("returns the level word exactly as the line spelled it, not a tone name", () => {
    expect(logLineLevel("connection error: pool exhausted")).toBe("error");
    expect(logLineLevel("14:07:41.902 ERROR pool timeout waited=30.0s")).toBe("ERROR");
    expect(logLineLevel("WARNING: certificate expires in 6 days")).toBe("WARNING");
    expect(logLineLevel("warn pool saturated, queueing request")).toBe("warn");
    expect(logLineLevel("info starting checkout-api build=4f2a1c")).toBe("info");
  });

  it("recognises debug and trace, which carry no tone of their own", () => {
    expect(logLineLevel("debug cache miss for key=42")).toBe("debug");
    expect(logLineLevel("trace entering handler")).toBe("trace");
  });

  it("returns undefined when the line carries no recognised level word", () => {
    expect(logLineLevel("GET /healthz 200 1ms")).toBeUndefined();
    expect(logLineLevel("")).toBeUndefined();
  });

  it("does not match a level word as a substring of an unrelated word", () => {
    expect(logLineLevel("the informant forewarned the team")).toBeUndefined();
  });

  it("prefers the worst word when a line carries more than one, same as logLineHealth", () => {
    // 'error' (danger family) beats 'warn' (warning family) beats 'info',
    // exactly the precedence logLineHealth checks — because logLineHealth is
    // now derived from this scan, not a second one.
    expect(logLineLevel("warn: escalated to error after 3 retries")).toBe("error");
  });

  it("logLineHealth is derived from this scan, not a second regex over the same text", () => {
    // Every level word this function can return either maps to the same
    // HealthKind logLineHealth already returned for it, or — for a word this
    // function recognises but logLineHealth never toned (debug, trace) —
    // logLineHealth still reads neutral, unchanged from before the refactor.
    const samples = [
      "connection error: pool exhausted",
      "fatal: liveness deadline exceeded, terminating",
      "panic: runtime error: index out of range",
      "warn pool saturated, queueing request",
      "WARNING: certificate expires in 6 days",
      "info starting checkout-api build=4f2a1c",
      "debug cache miss for key=42",
      "trace entering handler",
      "GET /healthz 200 1ms",
    ];
    for (const text of samples) {
      const level = logLineLevel(text);
      const health = logLineHealth(text);
      if (level === undefined) {
        expect(health).toBe("neutral");
      } else if (/^(?:error|fatal|panic)$/i.test(level)) {
        expect(health).toBe("danger");
      } else if (/^warn(?:ing)?$/i.test(level)) {
        expect(health).toBe("warning");
      } else if (/^info$/i.test(level)) {
        expect(health).toBe("info");
      } else {
        // debug / trace: recognised as a level, but not a tone.
        expect(health).toBe("neutral");
      }
    }
  });
});

/**
 * THE REAL INPUT. The tally above was tuned against five invented lines from
 * a design mock; every fixture in this block is the shape a running cluster
 * actually emits — nginx, klog (every control-plane component), CoreDNS, a
 * crash-looping pod, a JSON logger, a container entrypoint. The rail showed
 * one row on a real workload because these lines open with their framing and
 * the rule read the framing.
 */
describe("tallyLogTerms on real Kubernetes lines", () => {
  it("tallies nginx's message rather than stopping on the date it opens with", () => {
    // The single most repeated line in an nginx pod's stream, and it used to
    // contribute NOTHING: the leading `2026/08/25` is a digit, and a digit
    // ended the run.
    const lines = [
      line("2026/08/25 07:53:02 [notice] 1#1: start worker process 38"),
      line("2026/08/25 07:53:02 [notice] 1#1: start worker process 39"),
      line("2026/08/25 07:53:03 [notice] 1#1: start worker process 40"),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "start worker process", count: 3, tone: "neutral" },
    ]);
  });

  it("tallies a klog line's message, not its severity letter and month-day", () => {
    // `I0825` is a severity letter and a date. It is identical on every line
    // of every control-plane component, so tallying it says nothing at all.
    const lines = [
      line(
        "I0825 08:23:00.651626       1 cidrallocator.go:278] updated ClusterIP allocator for Service CIDR 10.96.0.0/16",
      ),
      line(
        "I0825 08:24:00.652003       1 cidrallocator.go:278] updated ClusterIP allocator for Service CIDR 10.96.0.0/16",
      ),
      line(
        "I0825 08:25:00.651980       1 cidrallocator.go:278] updated ClusterIP allocator for Service CIDR 10.96.0.0/16",
      ),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "updated ClusterIP", count: 3, tone: "neutral" },
    ]);
  });

  it("keeps CoreDNS legible, with its level in the tone instead of the text", () => {
    const lines = [
      line("[ERROR] plugin/kubernetes: Failed to watch *v1.Namespace: Unauthorized"),
      line("[ERROR] plugin/kubernetes: Failed to watch *v1.Service: Unauthorized"),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "plugin/kubernetes Failed", count: 2, tone: "danger" },
    ]);
  });

  it("tallies a crash-looping pod's own sentence, past its level word", () => {
    const lines = [
      line("FATAL cannot reach mainframe gateway 10.0.4.12:9443"),
      line("FATAL cannot reach mainframe gateway 10.0.4.19:9443"),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "cannot reach mainframe gateway", count: 2, tone: "danger" },
    ]);
  });

  it("tallies a JSON line's msg, which is the only human part of it", () => {
    // Whole-line tallying gives nothing here: the object is one whitespace-
    // free token and the timestamp inside makes it unique per line.
    const lines = [
      line(
        '{"level":"info","app":"deployment-srv","component":"services","time":"2026-08-25T08:13:42.051Z","msg":"component updated"}',
      ),
      line(
        '{"level":"info","app":"deployment-srv","component":"services","time":"2026-08-25T08:14:12.884Z","msg":"component updated"}',
      ),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "component updated", count: 2, tone: "info" },
    ]);
  });

  it("reads `message` as well as `msg`", () => {
    const lines = [
      line('{"level":"warn","message":"backpressure applied","queue":91}'),
      line('{"level":"warn","message":"backpressure applied","queue":204}'),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "backpressure applied", count: 2, tone: "warning" },
    ]);
  });

  it("a JSON object with no message field contributes nothing, rather than its own braces", () => {
    const lines = [
      line('{"level":"info","app":"deployment-srv","phase":"reconcile"}'),
      line('{"level":"info","app":"deployment-srv","phase":"prune"}'),
    ];
    expect(tallyLogTerms(lines)).toEqual([]);
  });

  it("does not throw on a line that merely opens with a brace", () => {
    const lines = [line("{not json at all, honestly"), line("{not json at all, really")];
    expect(() => tallyLogTerms(lines)).not.toThrow();
    expect(tallyLogTerms(lines)).toEqual([{ term: "{not json at all", count: 2, tone: "neutral" }]);
  });

  it("tallies the entrypoint's message across the scripts it launches", () => {
    const lines = [
      line("/docker-entrypoint.sh: Launching /docker-entrypoint.d/10-listen-on-ipv6-by-default.sh"),
      line("/docker-entrypoint.sh: Launching /docker-entrypoint.d/20-envsubst-on-templates.sh"),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "/docker-entrypoint.sh Launching", count: 2, tone: "neutral" },
    ]);
  });

  it("gives a mixed real buffer a row per component, and no framing anywhere", () => {
    // The screenshot: one workload's stream, four components talking. The old
    // rule produced one row (or four rows of `I0825`); this is the whole
    // defect, stated once.
    const lines = [
      line("2026/08/25 07:53:02 [notice] 1#1: start worker process 38"),
      line("2026/08/25 07:53:02 [notice] 1#1: start worker process 39"),
      line(
        "I0825 08:23:00.651626       1 cidrallocator.go:278] updated ClusterIP allocator for Service CIDR 10.96.0.0/16",
      ),
      line(
        "I0825 08:24:00.652003       1 cidrallocator.go:278] updated ClusterIP allocator for Service CIDR 10.96.0.0/16",
      ),
      line('{"level":"info","app":"deployment-srv","time":"2026-08-25T08:13:42.051Z","msg":"component updated"}'),
      line('{"level":"info","app":"deployment-srv","time":"2026-08-25T08:14:12.884Z","msg":"component updated"}'),
      line("[ERROR] plugin/kubernetes: Failed to watch *v1.Namespace: Unauthorized"),
      line("[ERROR] plugin/kubernetes: Failed to watch *v1.Service: Unauthorized"),
    ];
    const terms = tallyLogTerms(lines).map((t) => t.term);
    expect(terms).toEqual(
      expect.arrayContaining([
        "start worker process",
        "updated ClusterIP",
        "component updated",
        "plugin/kubernetes Failed",
      ]),
    );
    // Nothing that is the log's framing rather than its message.
    expect(terms.some((t) => /^I\d{4}\b/.test(t))).toBe(false);
    expect(terms.some((t) => t.startsWith("2026/"))).toBe(false);
    expect(terms.some((t) => t.startsWith("["))).toBe(false);
  });

  it("keeps a whole clause only while the WHOLE clause is what recurred", () => {
    // Both lines open on the same five words and part on the sixth, so the
    // clause is not what repeated — the headline is. Take the clause here and
    // the rail shows a phrase ending in "the", which is the ragged edge the
    // sliding-prefix version of this rule produced everywhere.
    const lines = [
      line("info liveness deadline extended for the drain"),
      line("info liveness deadline extended for the shutdown"),
    ];
    expect(tallyLogTerms(lines)).toEqual([{ term: "liveness deadline", count: 2, tone: "info" }]);
  });

  it("still ends a term on a numeric token in the MIDDLE of a line", () => {
    // Skipping structure is a rule about the START of a line only. Inside the
    // message a number is data — this is what keeps `duration=30011ms` and a
    // worker pid out of the tally.
    const lines = [line("cache flushed 4096 entries"), line("cache flushed 128 entries")];
    expect(tallyLogTerms(lines)).toEqual([{ term: "cache flushed", count: 2, tone: "neutral" }]);
  });

  it("keeps every term short enough for a 272px rail", () => {
    const long =
      "reconciliation of the exceedingly verbose custom resource definition completed successfully without incident";
    const lines = [line(long), line(long), line(long)];
    const [top] = tallyLogTerms(lines);
    expect(top.term.length).toBeLessThanOrEqual(40);
  });

  it("a long quoted klog warning is trimmed to its headline, not left 90 characters wide", () => {
    const warning =
      'I0825 08:13:41.721258       7 warnings.go:107] "Warning: Use tokens from the TokenRequest API or manually created secret-based tokens instead of auto-generated ones"';
    const [top] = tallyLogTerms([line(warning), line(warning)]);
    expect(top).toEqual({ term: "Use tokens", count: 2, tone: "warning" });
  });

  it("a buffer of unrelated real lines still yields nothing, not a list of ones", () => {
    // The threshold's own failure mode, restated over lines that all now get
    // past their framing — getting past it must not turn noise into rows.
    const lines = [
      line("2026/08/25 07:53:02 [notice] 1#1: start worker process 38"),
      line("I0825 08:23:00.651626       1 cidrallocator.go:278] updated ClusterIP allocator"),
      line('{"level":"info","app":"deployment-srv","msg":"component updated"}'),
      line("[ERROR] plugin/kubernetes: Failed to watch"),
      line("FATAL cannot reach mainframe gateway 10.0.4.12:9443"),
    ];
    expect(tallyLogTerms(lines)).toEqual([]);
  });
});

/**
 * THE SECOND DEFECT. `{"level":"warn",…,"error":{},…}` rendered ERROR in the
 * level column: `\berror\b` matched an empty JSON field NAME, because a quote
 * is a non-word character. The line said what it was and we overrode it.
 */
describe("a line that declares its own level", () => {
  it("believes a declared level over a scary word elsewhere in the line", () => {
    const declared =
      '{"level":"warn","app":"deployment-srv","source":"helm","name":"m01-test-01-core-services","error":{},"time":"2026-08-25T08:22:00.594Z","msg":"upgrade failed"}';
    expect(logLineLevel(declared)).toBe("warn");
    expect(logLineHealth(declared)).toBe("warning");
  });

  it("returns the declared level exactly as the application spelled it", () => {
    expect(logLineLevel('{"level":"ERROR","msg":"UpdateServices"}')).toBe("ERROR");
    expect(logLineHealth('{"level":"ERROR","msg":"UpdateServices"}')).toBe("danger");
    expect(logLineLevel('{"level":"info","app":"deployment-srv","msg":"component updated"}')).toBe(
      "info",
    );
  });

  it("reads `severity` as well as `level`", () => {
    // The declared word must CONTRADICT the text scan, or a rule that ignored
    // `severity` entirely would still read ERROR off the message and pass.
    const stackdriver = '{"severity":"warning","message":"connection error, retried"}';
    expect(logLineLevel(stackdriver)).toBe("warning");
    expect(logLineHealth(stackdriver)).toBe("warning");
  });

  it("tones the syslog spellings an application may declare", () => {
    // `err`, `crit` and `notice` are levels the text scan never returns, so
    // they can only arrive declared — and a real level must not land on
    // neutral by omission, which is the bug 277924e fixed for `panic`.
    expect(logLineHealth('{"level":"err","msg":"boom"}')).toBe("danger");
    expect(logLineHealth('{"level":"crit","msg":"boom"}')).toBe("danger");
    expect(logLineHealth('{"level":"critical","msg":"boom"}')).toBe("danger");
    expect(logLineHealth('{"level":"emerg","msg":"boom"}')).toBe("danger");
    expect(logLineHealth('{"level":"alert","msg":"boom"}')).toBe("danger");
    expect(logLineHealth('{"level":"notice","msg":"listening"}')).toBe("info");
    expect(logLineHealth('{"level":"warning","msg":"slow"}')).toBe("warning");
    expect(logLineHealth('{"level":"debug","msg":"cache miss"}')).toBe("neutral");
  });

  it("reports a level word it does not recognise, but tones it neutral rather than guessing", () => {
    // The word is the application's, so the column prints it; the tone is a
    // claim we cannot make, so we do not make one up from the rest of the
    // line — which is exactly the guess this whole change removes.
    expect(logLineLevel('{"level":"audit","msg":"policy evaluated"}')).toBe("audit");
    expect(logLineHealth('{"level":"audit","msg":"policy evaluated"}')).toBe("neutral");
  });

  it("falls back to the text scan when the line is not a JSON object", () => {
    // Malformed, and not an object: both must reach the old path untouched,
    // and neither may throw.
    expect(logLineLevel('{"level":"warn","msg":"unterminated')).toBe("warn");
    expect(logLineLevel("{oops not json, error inside}")).toBe("error");
    expect(logLineLevel("[1,2,3] error after an array")).toBe("error");
    expect(logLineLevel('"just a json string"')).toBeUndefined();
    expect(logLineHealth("null")).toBe("neutral");
  });

  it("ignores a non-string level and falls back to the text scan", () => {
    // pino writes `"level":30`. There is no word to print, so the scan
    // decides, exactly as it did before this existed.
    expect(logLineLevel('{"level":30,"msg":"listening"}')).toBeUndefined();
    expect(logLineLevel('{"level":50,"msg":"connection error"}')).toBe("error");
  });

  it("does not let an empty error field repaint a declared info line", () => {
    const line1 = '{"level":"info","error":{},"msg":"reconciled"}';
    expect(logLineHealth(line1)).toBe("info");
  });

  it("tones the term tally from the declared level too", () => {
    const lines = [
      line('{"level":"warn","error":{},"time":"2026-08-25T08:22:00.594Z","msg":"upgrade failed"}'),
      line('{"level":"warn","error":{},"time":"2026-08-25T08:23:11.104Z","msg":"upgrade failed"}'),
    ];
    expect(tallyLogTerms(lines)).toEqual([
      { term: "upgrade failed", count: 2, tone: "warning" },
    ]);
  });
});
