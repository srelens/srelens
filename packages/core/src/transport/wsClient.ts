// apps/desktop/src/transport/wsClient.ts
// A single multiplexed WebSocket to /api/ws. Many logical channels share one
// socket; the server fans out {channel,payload} frames. Reconnects with
// backoff and resubscribes every live channel so a dropped socket self-heals.

import { parseClusterLoginRequired, requestClusterLogin } from "../lib/clusterLogin";

type Handler = (payload: unknown) => void;

export function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/api/ws`;
}

class WsClient {
  private socket: WebSocket | null = null;
  private handlers = new Map<string, Set<Handler>>();
  private ackWaiters = new Map<string, Array<() => void>>();
  private backoff = 500;
  private connecting = false;

  private ensureSocket(): void {
    if (this.socket && (this.socket.readyState === WebSocket.OPEN || this.socket.readyState === WebSocket.CONNECTING)) {
      return;
    }
    if (this.connecting) return;
    this.connecting = true;
    const ws = new WebSocket(wsUrl());
    this.socket = ws;
    ws.addEventListener("open", () => {
      this.connecting = false;
      this.backoff = 500;
      // Resubscribe every live channel.
      for (const channel of this.handlers.keys()) {
        ws.send(JSON.stringify({ op: "sub", channel }));
      }
    });
    ws.addEventListener("message", (e) => this.onMessage(String(e.data)));
    ws.addEventListener("close", () => {
      this.connecting = false;
      this.socket = null;
      if (this.handlers.size > 0) {
        const delay = this.backoff;
        this.backoff = Math.min(this.backoff * 2, 10_000);
        setTimeout(() => {
          // Channels may have been disposed during the backoff window; only
          // reconnect if there's still something to subscribe to.
          if (this.handlers.size > 0) this.ensureSocket();
        }, delay);
      }
    });
    ws.addEventListener("error", () => ws.close());
  }

  private onMessage(text: string): void {
    let frame: { op?: string; channel?: string; payload?: unknown };
    try {
      frame = JSON.parse(text);
    } catch {
      return;
    }
    if (frame.op === "subbed" && frame.channel) {
      const waiters = this.ackWaiters.get(frame.channel);
      if (waiters) {
        this.ackWaiters.delete(frame.channel);
        for (const resolve of waiters) resolve();
      }
      return;
    }
    if (frame.channel && "payload" in frame) {
      // A stream may report that its context's OIDC cluster needs sign-in (the
      // client is resolved async inside the stream, so this can't be an HTTP
      // 401). Detect the marker centrally and prompt, then still fan the
      // payload out so the stream tears down as usual. Skip raw byte-output
      // channels (`*:out:*`, log lines) — the marker only ever arrives on
      // error/exit/closed channels, and raw output could contain the literal
      // string (e.g. cat-ing a source file) and spuriously prompt.
      const rawOutput =
        frame.channel.includes(":out:") || frame.channel.startsWith("logs:line");
      if (!rawOutput) {
        const login = parseClusterLoginRequired(frame.payload);
        if (login) requestClusterLogin(login);
      }
      const set = this.handlers.get(frame.channel);
      if (set) for (const h of set) h(frame.payload);
    }
  }

  private sendWhenOpen(msg: object): void {
    this.ensureSocket();
    const ws = this.socket;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    } else if (ws) {
      ws.addEventListener("open", () => ws.send(JSON.stringify(msg)), { once: true });
    }
  }

  subscribeChannel(channel: string, handler: Handler, opts?: { awaitAck?: boolean }): Promise<() => void> {
    let set = this.handlers.get(channel);
    if (!set) {
      set = new Set();
      this.handlers.set(channel, set);
    }
    set.add(handler);

    const ackPromise = opts?.awaitAck
      ? new Promise<void>((resolve) => {
          const arr = this.ackWaiters.get(channel) ?? [];
          arr.push(resolve);
          this.ackWaiters.set(channel, arr);
        })
      : Promise.resolve();

    this.sendWhenOpen({ op: "sub", channel });

    const dispose = () => {
      const s = this.handlers.get(channel);
      if (s) {
        s.delete(handler);
        if (s.size === 0) {
          this.handlers.delete(channel);
          // Only send unsub over an already-open socket — never resurrect one
          // just to say goodbye. If the socket is down, the server already
          // dropped our subscriptions, and since the channel is gone it won't
          // be resubscribed on reconnect, so there's nothing left to unsub.
          if (this.socket && this.socket.readyState === WebSocket.OPEN) {
            this.socket.send(JSON.stringify({ op: "unsub", channel }));
          }
        }
      }
    };
    return ackPromise.then(() => dispose);
  }
}

const client = new WsClient();
export default client;
