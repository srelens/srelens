import { describe, it, expect } from "vitest";
import { parseCurrentContext } from "./addCluster";
import { parseDeepLink } from "./deepLink";
import { describeForbidden } from "./errors";
import { isTableSeparator } from "./assistantMarkdown";
import { certificateBlocks } from "./k8sSecret";
import { tokenize } from "./logTerms";

/**
 * Regressions for nine `js/polynomial-redos` findings (#43-#51).
 *
 * Each of these parsers reads input the user does not control: a pasted
 * kubeconfig, a deep link handed over by the OS, an error message from the API
 * server, and the assistant's own markdown output. An ambiguous quantifier in
 * any of them is a way to freeze the UI thread from outside the app.
 *
 * Measured before fixing, so these numbers are what was actually happening
 * rather than what the rule name implies:
 *
 *   current-context   cubic      51s at a 4KB line
 *   table separator   quadratic  10.7s at 40KB
 *   deep link         quadratic  2.5s at 40KB
 *   describeForbidden quadratic  78ms at 66KB
 *
 * The budget below is deliberately loose. It is not a benchmark — it is the
 * difference between linear and catastrophic, and a loose bound keeps the test
 * from flaking on a busy CI runner while still failing outright on a
 * regression, which would take seconds or minutes rather than milliseconds.
 */
const BUDGET_MS = 250;

function withinBudget(label: string, run: () => void): void {
  const started = performance.now();
  run();
  const elapsed = performance.now() - started;
  expect(elapsed, `${label} took ${elapsed.toFixed(0)}ms, budget ${BUDGET_MS}ms`).toBeLessThan(
    BUDGET_MS,
  );
}

describe("parsers stay linear on hostile input", () => {
  it("reads current-context from a kubeconfig padded with spaces (#43)", () => {
    // 51 seconds before the fix, at a quarter of this size.
    const yaml = `current-context:${" ".repeat(16000)}#`;
    withinBudget("current-context", () => parseCurrentContext(yaml));
  });

  it("parses a deep link buried in slashes (#48)", () => {
    const url = `srelens:${"/".repeat(40000)}\n`;
    withinBudget("deep link", () => parseDeepLink(url));
  });

  it("recognises a table separator padded with spaces (#44)", () => {
    const row = `-${"  ".repeat(20000)}x`;
    withinBudget("table separator", () => isTableSeparator(row));
  });

  it("describes a forbidden error that never completes its pattern (#49)", () => {
    const raw = `cannot a resource "!"${'cannot a resource "!"a'.repeat(3000)}`;
    withinBudget("describeForbidden", () => describeForbidden(raw));
  });

  /**
   * Three more of the same class, found on PR #380. Measured on the real
   * patterns before changing them, on this machine:
   *
   *   certificate blocks  quadratic  1386ms at 328KB (12k BEGIN, no END)
   *   token quote run     quadratic  709ms at 20k quotes, 2.7s at 40k, 11.1s at 80k
   *
   * The token figure is PER PATTERN and `tokenize` applied three of them to
   * every token, so one 40KB run of quotes in one log line cost ~8 seconds of
   * the UI thread.
   */
  it("scans a PEM with many BEGIN markers and no END (#380)", () => {
    // A Secret's `tls.crt` as the cluster hands it over. Each BEGIN was a
    // fresh start position for a lazy `[\s\S]*?` that then rescanned to the
    // end of the string looking for an END that is not there.
    const pem = "-----BEGIN CERTIFICATE-----\n".repeat(12_000);
    withinBudget("certificateBlocks", () => certificateBlocks(pem));
  });

  it("tokenizes a log line whose token is nothing but quotes (#380)", () => {
    // Log lines are STREAMED from the cluster: this is the least controlled
    // input in the app, and the only one of these that arrives continuously.
    const line = `${'"'.repeat(40_000)}x`;
    withinBudget("tokenize", () => tokenize(line));
  });
});

describe("the parsers still do their jobs", () => {
  it("reads a current-context, quoted or not, and ignores comments", () => {
    expect(parseCurrentContext("current-context: prod")).toBe("prod");
    expect(parseCurrentContext('current-context: "prod"')).toBe("prod");
    expect(parseCurrentContext("current-context: 'prod'")).toBe("prod");
    expect(parseCurrentContext("current-context:   prod   ")).toBe("prod");
    expect(parseCurrentContext("current-context: prod # the live one")).toBe("prod");
    expect(parseCurrentContext("apiVersion: v1\ncurrent-context: prod\nkind: Config")).toBe("prod");
    // OpenShift-style names survive intact.
    expect(parseCurrentContext("current-context: default/api-example-com:6443/user")).toBe(
      "default/api-example-com:6443/user",
    );
  });

  it("keeps a hash that belongs to the name (#313 review)", () => {
    // YAML starts a comment at `#` only when whitespace precedes it, and never
    // inside quotes. Cutting at every `#` turned a valid name into a shorter
    // one, or left a stray quote, and srelens then probed a context that does
    // not exist.
    expect(parseCurrentContext("current-context: prod#live")).toBe("prod#live");
    expect(parseCurrentContext('current-context: "prod#live"')).toBe("prod#live");
    expect(parseCurrentContext("current-context: 'prod#live'")).toBe("prod#live");
    expect(parseCurrentContext("current-context: #only-a-comment")).toBeNull();
    // Whitespace before the hash still ends the scalar.
    expect(parseCurrentContext("current-context: prod # the live one")).toBe("prod");
    expect(parseCurrentContext("current-context: prod\t# tabbed comment")).toBe("prod");
    // A comment after a quoted scalar is still a comment.
    expect(parseCurrentContext('current-context: "prod" # the live one')).toBe("prod");
    // Unterminated quote is malformed, not a name with a quote in it.
    expect(parseCurrentContext('current-context: "prod')).toBeNull();
    // Escaped and doubled quotes belong to the name, not to the delimiter.
    expect(parseCurrentContext('current-context: "prod\\"live"')).toBe('prod"live');
    expect(parseCurrentContext("current-context: 'prod''live'")).toBe("prod'live");
  });

  it("returns null when there is no current-context to read", () => {
    expect(parseCurrentContext("kind: Config")).toBeNull();
    expect(parseCurrentContext("current-context:")).toBeNull();
    expect(parseCurrentContext("current-context:    ")).toBeNull();
    expect(parseCurrentContext("current-context: # only a comment")).toBeNull();
  });

  it("still parses a deep link the same way", () => {
    // The scheme and slashes are stripped in two steps now instead of one
    // pattern; every accepted and rejected form must behave as before.
    const target = { route: "cluster", context: "prod" };
    expect(parseDeepLink("srelens://cluster/prod")).toEqual(target);
    expect(parseDeepLink("srelens:cluster/prod")).toEqual(target);
    expect(parseDeepLink("srelens:///cluster/prod")).toEqual(target);
    expect(parseDeepLink("SRELENS://cluster/prod")).toEqual(target);
    expect(parseDeepLink("  srelens://cluster/prod  ")).toEqual(target);
    expect(parseDeepLink("srelens://cluster/prod?tab=logs")).toEqual(target);
    expect(parseDeepLink("srelens://cluster/prod#top")).toEqual(target);
    expect(parseDeepLink("https://example.com/cluster/prod")).toBeNull();
    expect(parseDeepLink("srelens://")).toBeNull();
  });

  it("refuses to invent a scope the error never stated", () => {
    // The pattern this replaced required one of the two scope markers, so a
    // message with neither fell through to describeError's generic RBAC
    // guidance. A truncated or aggregated-API error must not be reported as
    // cluster-scoped just because it lacks a namespace. (#313 review)
    expect(describeForbidden('cannot patch resource "deployments"')).toBeNull();
    expect(describeForbidden('cannot patch resource "deployments" somewhere odd')).toBeNull();

    expect(describeForbidden('cannot patch resource "nodes" at the cluster scope')).toBe(
      "You don't have permission to patch nodes at the cluster scope.",
    );
    expect(
      describeForbidden('cannot patch resource "deployments" in the namespace "prod"'),
    ).toBe("You don't have permission to patch deployments in prod.");
    // Both present: namespaced wins, as the old alternation ordered it.
    expect(
      describeForbidden(
        'cannot patch resource "deployments" in the namespace "prod" at the cluster scope',
      ),
    ).toBe("You don't have permission to patch deployments in prod.");
  });

  it("finds every certificate block, and nothing when a block never closes", () => {
    const one = "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----";
    expect(certificateBlocks("")).toEqual([]);
    expect(certificateBlocks("not a certificate at all")).toEqual([]);
    expect(certificateBlocks(one)).toEqual([one]);
    expect(certificateBlocks(`lead\n${one}\ntail`)).toEqual([one]);
    expect(certificateBlocks(`${one}\n${one}`)).toEqual([one, one]);
    // A BEGIN with no END is not a block, and neither is what follows it: the
    // regex behaved the same way, and this is the input that made it slow.
    expect(certificateBlocks("-----BEGIN CERTIFICATE-----\nAAA\n")).toEqual([]);
    expect(certificateBlocks(`${one}\n-----BEGIN CERTIFICATE-----\n`)).toEqual([one]);
    // An END before any BEGIN is not a block either.
    expect(certificateBlocks(`-----END CERTIFICATE-----\n${one}`)).toEqual([one]);
    // A nested BEGIN is swallowed by the block that opened first — the same
    // answer the single lazy pattern gave, since it matched from the earliest
    // start position it could.
    const nested = "-----BEGIN CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----";
    expect(certificateBlocks(nested)).toEqual([nested]);
  });

  it("recognises table separators and rejects other rows", () => {
    expect(isTableSeparator("|---|---|")).toBe(true);
    expect(isTableSeparator(" --- ")).toBe(true);
    expect(isTableSeparator("|:--|--:|:-:|")).toBe(true);
    expect(isTableSeparator("| a | b |")).toBe(false);
    // No dash at all is a row of pipes, not a separator.
    expect(isTableSeparator("|   |   |")).toBe(false);
    expect(isTableSeparator("")).toBe(false);
  });
});
