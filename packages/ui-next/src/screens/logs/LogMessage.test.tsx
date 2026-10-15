import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { parseLogQuery, type QueryClause } from "@srelens/core";
import { renderMessage, shownText } from "./LogMessage";

const row = { message: "Wrote batch", structured: true, fields: { system: "camlog" }, truncated: false };

function clauses(query: string): readonly QueryClause[] {
  const parsed = parseLogQuery(query);
  if (!parsed.ok) throw new Error(parsed.error);
  return parsed.clauses;
}

const marks = (query: string, extra?: readonly (readonly [number, number])[]) =>
  [...render(<>{renderMessage(row, clauses(query), extra)}</>).container.querySelectorAll("mark")].map(
    (m) => m.textContent,
  );

describe("shownText", () => {
  it("is the message, the gap, and the inline fields: the string a regex's marks are worked out over", () => {
    expect(shownText(row)).toBe("Wrote batch  system=camlog");
  });
});

describe("renderMessage", () => {
  it("marks a regex where the ranges it is given say, and nowhere by itself", () => {
    expect(marks("/cam\\w+/", [[20, 26]])).toEqual(["camlog"]);
    expect(marks("/cam\\w+/")).toEqual([]);
  });

  it("marks the ranges it is given alongside the query's own marks", () => {
    expect(marks("wrote /x/", [[6, 11]])).toEqual(["Wrote", "batch"]);
  });
});
