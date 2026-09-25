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

describe("a WebSocket a test opens", () => {
  // A real one to the transport's URL is refused within milliseconds, and the
  // refusal is what arms the transport's reconnect timer.
  it("never connects and never fails, so the transport has nothing to retry", async () => {
    const socket = new WebSocket("ws://localhost:3000/api/ws");
    const events: string[] = [];
    for (const type of ["open", "message", "error", "close"]) socket.addEventListener(type, () => events.push(type));
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(events).toEqual([]);
    expect(socket.readyState).toBe(WebSocket.CONNECTING);
  });
});
