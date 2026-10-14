/**
 * A setup file for the suites that render the app: no test may open a real
 * WebSocket. Listed in each such project's `setupFiles` rather than copied
 * into each `test-setup.ts`, for the reason `vitest.shared.ts` gives.
 *
 * Under jsdom `@srelens/core` picks its web transport, because its own
 * `isTauri()` is false there — mocking `isTauri` on the `@srelens/core` barrel
 * does not reach it. So a test that renders a screen whose stream it did not
 * mock subscribes over a real socket to `ws://localhost:3000/api/ws`. Nothing
 * listens there; the refusal closes the socket, and `wsClient` reconnects with
 * backoff for as long as any channel is held. One taken with `subscribe` is
 * held for good: its unsubscribe only arrives with the server's ack.
 *
 * The loop outlives the test file. A reconnect that fires after jsdom has torn
 * down reads `location` in `wsUrl()` and throws `ReferenceError: location is
 * not defined`, which vitest reports as an unhandled error and fails the run
 * with every test green. It fires only while a worker is still busy after the
 * teardown — collecting coverage on a loaded CI runner — so it failed now and
 * then, against whichever file that opened sockets it happened to be.
 *
 * This socket never connects and never fails: it stays CONNECTING and fires
 * nothing, so the transport never reaches its close handler and never
 * schedules a retry. To the tests nothing else changes — a socket that is
 * refused delivers no frames either. A test of the transport itself stubs
 * its own fake over this one (`vi.stubGlobal`, as `wsClient.test.ts` does).
 */
class OfflineWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readonly url: string;
  readyState: number = OfflineWebSocket.CONNECTING;

  constructor(url: string | URL) {
    super();
    this.url = String(url);
  }

  send(): void {}

  /** Closed, silently: a `close` event is exactly what would start a retry. */
  close(): void {
    this.readyState = OfflineWebSocket.CLOSED;
  }
}

globalThis.WebSocket = OfflineWebSocket as unknown as typeof WebSocket;
