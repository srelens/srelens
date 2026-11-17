import { useState } from "react";

/** The reason the field's text cannot be used as a filter, or null when it can. */
export function tableFilterError(query: string, regex = false): string | null {
  const pattern = query.trim();
  if (!regex || pattern === "") return null;
  try {
    new RegExp(pattern, "i");
    return null;
  } catch {
    return "Invalid regular expression";
  }
}

/**
 * The one rule every filter box matches by, so the same words find the same
 * rows on every list: case-insensitive, a substring or a regular expression.
 *
 * A function of the value rather than a test of one: a list asks about every
 * cell of every row, and the pattern is compiled once here. No global flag, so
 * `test` stays stateless between them.
 */
export function filterMatcher(query: string, regex = false): (value: string) => boolean {
  const pattern = query.trim();
  if (!pattern) return () => true;
  if (regex) {
    try {
      const matcher = new RegExp(pattern, "i");
      return (value) => matcher.test(value);
    } catch {
      // An incomplete expression is normal while someone types. Keep the
      // current list available; FilterBar names the invalid state beside it.
      return () => true;
    }
  }
  const normalized = pattern.toLocaleLowerCase();
  return (value) => value.toLocaleLowerCase().includes(normalized);
}

/**
 * A filter a list keeps for itself, ready to spread onto `FilterBar`, for the
 * lists whose filter does not outlive them. `matches` is the rule above.
 */
export function useListFilter() {
  const [value, onValueChange] = useState("");
  const [regex, onRegexChange] = useState(false);
  return {
    value,
    onValueChange,
    regex,
    onRegexChange,
    invalid: tableFilterError(value, regex) !== null,
    matches: filterMatcher(value, regex),
  };
}
