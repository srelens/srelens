import { describe, expect, it } from "vitest";

/** The zone in effect, or `timeZone` as ICU names it ("Asia/Kathmandu" comes back "Asia/Katmandu"). */
const zoneNow = (timeZone?: string) => Intl.DateTimeFormat(undefined, { timeZone }).resolvedOptions().timeZone;

// The two cases run in order: the first pins a zone, the second checks it did not outlive the test.
describe("a time zone a test pins", () => {
  const machine = zoneNow();
  const other = zoneNow(machine === zoneNow("Asia/Kathmandu") ? "America/Los_Angeles" : "Asia/Kathmandu");
  it("applies within the test", () => {
    process.env.TZ = other;
    expect(zoneNow()).toBe(other);
  });
  it("is gone by the next test", () => {
    expect(zoneNow()).toBe(machine);
  });
});
