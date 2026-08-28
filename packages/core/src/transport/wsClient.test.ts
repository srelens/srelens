import { beforeEach, describe, expect, it, vi } from "vitest";
import { requestClusterLogin } from "../lib/clusterLogin";

vi.mock("../lib/clusterLogin", async (orig) => ({
  ...(await orig<typeof import("../lib/clusterLogin")>()),
  requestClusterLogin: vi.fn(),
}));

// A controllable fake WebSocket.
class FakeWS {
  static instances: FakeWS[] = [];
  static OPEN = 1;
  static CONNECTING = 0;
  readyState = 0;
  sent: string[] = [];
  private listeners: Record<string, Array<(e: unknown) => void>> = {};
  constructor(public url: string) {
    FakeWS.instances.push(this);
  }
  addEventListener(type: string, cb: (e: unknown) => void) {
    (this.listeners[type] ??= []).push(cb);
  }
  send(data: string) {
    this.sent.push(data);
  }
  close() {
    this.readyState = 3;
    this.emit("close", {});
  }
  open() {
    this.readyState = 1;
    this.emit("open", {});
  }
  message(data: string) {
    this.emit("message", { data });
  }
  private emit(type: string, e: unknown) {
    for (const cb of this.listeners[type] ?? []) cb(e);
  }
}

describe("wsClient", () => {
  beforeEach(() => {
    FakeWS.instances = [];
    vi.stubGlobal("WebSocket", FakeWS as unknown as typeof WebSocket);
    vi.stubGlobal("location", { protocol: "http:", host: "127.0.0.1:8080" } as unknown as Location);
    vi.resetModules();
    vi.mocked(requestClusterLogin).mockClear();
  });

  it("subscribe resolves only after the subbed ack and delivers payloads", async () => {
    const { default: client } = await import("./wsClient");
    const handler = vi.fn();
    const promise = client.subscribeChannel("watch:1", handler, { awaitAck: true });
    const ws = FakeWS.instances[0];
    ws.open();
    expect(ws.sent).toContain(JSON.stringify({ op: "sub", channel: "watch:1" }));

    let resolved = false;
    void promise.then(() => (resolved = true));
    await Promise.resolve();
    expect(resolved).toBe(false); // not until the ack

    ws.message(JSON.stringify({ op: "subbed", channel: "watch:1" }));
    await promise;

    ws.message(JSON.stringify({ channel: "watch:1", payload: { rows: [] } }));
    expect(handler).toHaveBeenCalledWith({ rows: [] });
  });

  it("resubscribes live channels after a reconnect", async () => {
    vi.useFakeTimers();
    const { default: client } = await import("./wsClient");
    await new Promise<void>((resolve) => {
      void client.subscribeChannel("watch:1", vi.fn(), { awaitAck: false });
      resolve();
    });
    const first = FakeWS.instances[0];
    first.open();
    first.close(); // socket dropped

    vi.advanceTimersByTime(600); // backoff elapses → reconnect
    const second = FakeWS.instances[1];
    expect(second).toBeTruthy();
    second.open();
    expect(second.sent).toContain(JSON.stringify({ op: "sub", channel: "watch:1" }));
    vi.useRealTimers();
  });

  it("does not resurrect a socket just to send unsub after the socket is down", async () => {
    vi.useFakeTimers();
    const { default: client } = await import("./wsClient");
    const dispose = await client.subscribeChannel("watch:1", vi.fn(), { awaitAck: false });
    const first = FakeWS.instances[0];
    first.open();
    first.close(); // socket dropped

    dispose(); // last (only) handler for watch:1 goes away

    expect(FakeWS.instances.length).toBe(1); // no second socket created just to unsub
    vi.useRealTimers();
  });

  it("does not reconnect once every channel has been unsubscribed during the backoff window", async () => {
    vi.useFakeTimers();
    const { default: client } = await import("./wsClient");
    const dispose = await client.subscribeChannel("watch:1", vi.fn(), { awaitAck: false });
    const first = FakeWS.instances[0];
    first.open();
    first.close();               // schedules the reconnect timer (handlers still non-empty here)
    dispose();                   // empties handlers WHILE the backoff timer is pending

    vi.advanceTimersByTime(20_000); // past the backoff
    expect(FakeWS.instances.length).toBe(1); // fire-time re-check prevents a new socket
    vi.useRealTimers();
  });

  it("sends unsub over an open socket when the last handler is removed", async () => {
    const { default: client } = await import("./wsClient");
    const dispose = await client.subscribeChannel("watch:1", vi.fn(), { awaitAck: false });
    const first = FakeWS.instances[0];
    first.open();

    dispose();

    expect(first.sent).toContain(JSON.stringify({ op: "unsub", channel: "watch:1" }));
  });

  it("prompts cluster sign-in when a stream frame carries the needs-login marker, and still fans out the payload", async () => {
    const { default: client } = await import("./wsClient");
    const handler = vi.fn();
    await client.subscribeChannel("watch:x", handler, { awaitAck: false });
    const ws = FakeWS.instances[0];
    ws.open();

    const payload = { error: "NEEDS_CLUSTER_LOGIN:k:ctx" };
    ws.message(JSON.stringify({ channel: "watch:x", payload }));

    expect(requestClusterLogin).toHaveBeenCalledWith(
      expect.objectContaining({ key: "k", context: "ctx" }),
    );
    expect(handler).toHaveBeenCalledWith(payload); // fan-out not broken
  });

  it("does not prompt cluster sign-in for a normal stream frame", async () => {
    const { default: client } = await import("./wsClient");
    const handler = vi.fn();
    await client.subscribeChannel("watch:x", handler, { awaitAck: false });
    const ws = FakeWS.instances[0];
    ws.open();

    const payload = { status: "live" };
    ws.message(JSON.stringify({ channel: "watch:x", payload }));

    expect(requestClusterLogin).not.toHaveBeenCalled();
    expect(handler).toHaveBeenCalledWith(payload);
  });

  it("does not prompt on raw output channels even if the bytes contain the marker", async () => {
    const { default: client } = await import("./wsClient");
    const handler = vi.fn();
    // A terminal/exec stdout stream: the user could cat a file that literally
    // contains the marker string — it must NOT trigger a sign-in prompt.
    await client.subscribeChannel("term:out:1", handler, { awaitAck: false });
    const ws = FakeWS.instances[0];
    ws.open();

    const payload = "$ grep NEEDS_CLUSTER_LOGIN:k:ctx ./src\n";
    ws.message(JSON.stringify({ channel: "term:out:1", payload }));

    expect(requestClusterLogin).not.toHaveBeenCalled();
    expect(handler).toHaveBeenCalledWith(payload); // output still delivered
  });
});
