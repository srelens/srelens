import { afterEach, describe, expect, it, vi } from "vitest";

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
  afterEach(() => { vi.useRealTimers(); });

  // Not jsdom's: a real one to the transport's URL is refused, and the refusal
  // is what arms the transport's reconnect timer. Checked by name rather than
  // by importing the setup file, which would install it and so pass even when
  // `setupFiles` no longer lists it.
  it("is the offline one the setup installs", () => {
    expect(WebSocket.name).toBe("OfflineWebSocket");
  });

  it("never connects and never fails, even when closed, so the transport has nothing to retry", async () => {
    vi.useFakeTimers();
    const socket = new WebSocket("ws://localhost:3000/api/ws");
    const events: string[] = [];
    for (const type of ["open", "message", "error", "close"]) socket.addEventListener(type, () => events.push(type));
    await vi.runAllTimersAsync();
    expect(socket.readyState).toBe(WebSocket.CONNECTING);
    socket.close();
    await vi.runAllTimersAsync();
    expect(socket.readyState).toBe(WebSocket.CLOSED);
    expect(events).toEqual([]);
  });
});
