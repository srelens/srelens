import { afterEach, describe, expect, it, vi } from "vitest";
import { subscribe } from "@srelens/core/transport";
import { InertWebSocket } from "@srelens/core/testing/inertWebSocket";

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

// #730. A view subscribes on mount and waits for the server's ack before it is
// handed its unsubscribe. No server answers a test, so the channel stays open
// for the rest of the file. If the test's socket could reach the network, it
// would be refused and closed, and the client would schedule a reconnect —
// which, fired after Vitest has torn jsdom down, reads a `location` that is
// gone and fails the whole run with every test green.
describe("a channel a test never closes", () => {
  afterEach(() => vi.useRealTimers());
  it("leaves no reconnect behind, to fire after jsdom is torn down", async () => {
    // Stated outright: with jsdom's own socket, whether the timers below throw
    // depends on the refusal arriving within the wait.
    expect(globalThis.WebSocket).toBe(InertWebSocket);
    const wait = setTimeout;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    void subscribe("test:never-acked", () => {});
    // Ample real time for a socket that did dial out to be refused and close.
    await new Promise((resolve) => wait(resolve, 200));

    // What Vitest's jsdom teardown does to every window key it copied.
    const location = Object.getOwnPropertyDescriptor(globalThis, "location")!;
    delete (globalThis as { location?: unknown }).location;
    try {
      expect(() => vi.runAllTimers()).not.toThrow();
    } finally {
      Object.defineProperty(globalThis, "location", location);
    }
  });
});
