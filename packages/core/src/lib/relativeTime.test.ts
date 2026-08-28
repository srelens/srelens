import { describe, it, expect } from "vitest";
import { relativeTime } from "./relativeTime";

describe("relativeTime", () => {
  const now = 1_700_000_000_000;

  it("renders 'just now' for zero elapsed time", () => {
    expect(relativeTime(now, now)).toBe("just now");
  });

  it("renders 'just now' up to 59 seconds", () => {
    expect(relativeTime(now - 59_000, now)).toBe("just now");
  });

  it("renders whole minutes once a minute has elapsed", () => {
    expect(relativeTime(now - 60_000, now)).toBe("1m ago");
    expect(relativeTime(now - 5 * 60_000, now)).toBe("5m ago");
    expect(relativeTime(now - 59 * 60_000, now)).toBe("59m ago");
  });

  it("renders whole hours once an hour has elapsed", () => {
    expect(relativeTime(now - 60 * 60_000, now)).toBe("1h ago");
    expect(relativeTime(now - 2 * 60 * 60_000, now)).toBe("2h ago");
    expect(relativeTime(now - 23 * 60 * 60_000, now)).toBe("23h ago");
  });

  it("renders whole days once a day has elapsed", () => {
    expect(relativeTime(now - 24 * 60 * 60_000, now)).toBe("1d ago");
    expect(relativeTime(now - 3 * 24 * 60 * 60_000, now)).toBe("3d ago");
    expect(relativeTime(now - 400 * 24 * 60 * 60_000, now)).toBe("400d ago");
  });

  it("clamps a future timestamp (clock skew) to 'just now' instead of a negative duration", () => {
    expect(relativeTime(now + 60_000, now)).toBe("just now");
  });
});
