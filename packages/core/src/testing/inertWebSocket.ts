/**
 * A `WebSocket` for component tests: it never connects, never fails and never
 * closes (#730). Test support only — nothing in the app imports it.
 *
 * Component suites run the real web transport, and no server answers them.
 * jsdom's own `WebSocket` really dials `ws://localhost:3000`, is refused, and
 * closes. A view's subscription waits for the server's ack before handing back
 * its unsubscribe, so under test it can never be disposed — and a socket that
 * closes with a channel still open makes `WsClient` schedule a reconnect, with
 * backoff, for as long as the worker lives. Once Vitest has torn jsdom down,
 * that timer reads a `location` that is gone and fails the run.
 *
 * This one stays CONNECTING. The client never reconnects and nothing reaches
 * the network, and every subscription behaves as it would against a server
 * that has not answered yet. A test that drives a socket stubs its own.
 */
export class InertWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readonly readyState = InertWebSocket.CONNECTING;
  constructor(readonly url: string | URL) {}
  /** Nothing is ever dispatched, so no listener is kept. */
  addEventListener(): void {}
}

/** Replace the environment's `WebSocket` with `InertWebSocket`. */
export function installInertWebSocket(): void {
  globalThis.WebSocket = InertWebSocket as unknown as typeof WebSocket;
}
