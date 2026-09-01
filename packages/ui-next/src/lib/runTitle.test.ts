import { describe, it, expect } from "vitest";
import { titleFromQuestion } from "./runTitle";

describe("a conversation's title", () => {
  it("raises the first letter and keeps the reader's own words", () => {
    expect(titleFromQuestion("why is checkout-api throwing 5xx?")).toBe(
      "Why is checkout-api throwing 5xx?",
    );
  });

  it("cuts at a word boundary, not mid-token", () => {
    // The reported case: the rail showed `What is m01-prod-04-mongodb-0 usin…`
    const question =
      "What is m01-prod-04-mongodb-0 using right now and is that a problem for the replica set";
    const title = titleFromQuestion(question);
    expect(title.endsWith("…")).toBe(true);

    // The PROPERTY, not the example. An earlier version of this asserted the
    // title did not contain "usin…", which a hard 56-character cut also
    // satisfies — it lands elsewhere in the sentence and still mid-word. So:
    // whatever was kept must be a prefix of the question that stops exactly
    // where a word does.
    const body = title.replace("…", "");
    const original = question.charAt(0).toUpperCase() + question.slice(1);
    expect(original.startsWith(body)).toBe(true);
    expect(original.charAt(body.length)).toBe(" ");
  });

  it("drops an opener that says nothing about the question", () => {
    expect(titleFromQuestion("can you please check the cluster health")).toBe("Check the cluster health");
    expect(titleFromQuestion("show me pods restarting today")).toBe("Pods restarting today");
  });

  it("drops an opener only at the start, and only once", () => {
    // "show me" appearing again later is part of the sentence.
    expect(titleFromQuestion("show me the pods that show me trouble")).toBe(
      "The pods that show me trouble",
    );
  });

  it("keeps a question mark but not a trailing full stop", () => {
    expect(titleFromQuestion("is anything unhealthy?")).toBe("Is anything unhealthy?");
    expect(titleFromQuestion("roll it back.")).toBe("Roll it back");
  });

  it("does not collapse to nothing on one very long token", () => {
    const long = "a".repeat(200);
    const title = titleFromQuestion(long);
    // No space to break at, so it cuts hard rather than returning "…".
    expect(title.length).toBeGreaterThan(40);
  });

  it("says nothing for nothing", () => {
    expect(titleFromQuestion("   ")).toBe("");
    expect(titleFromQuestion("please")).toBe("");
  });
});
