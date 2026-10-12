import { describe, it, expect } from "vitest";
import { inlineFields, logLineWords, parseLogFields } from "./logQuery";
import { healthRank } from "./logTerms";

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
});
