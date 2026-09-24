// App streams (#565): the client half of the generic stream contract.
//
// A view of an app — a page, a detail tab, a panel — opens its streams
// through an `ExtensionView` and owns them: `view.close()` ends every stream
// it opened, on the host, in one call. The host ends them too when the app is
// disabled, updated or removed, and says so in the `close` frame.
//
// Frames travel on a channel the client listens on BEFORE it asks the host to
// open, so the first frame cannot race the listener: `open`, then any number
// of `data`, then exactly one of `close` (with a reason) or `error` (with a
// code and a message). The two terminal frames stay distinct all the way to
// `onEnd`: a stream that failed or was stopped for a limit must never read as
// one that simply ended. See docs/extensions/streams.md.
import { invokeCapability, invokeCommand, subscribe } from "../transport/transport";

/** What a stream carries. `read` re-runs a declared reader every `intervalSeconds` (5–300, default 15). */
export type ExtensionStreamSource = { kind: "read"; capability: string; intervalSeconds?: number };

export interface ExtensionStreamRequest {
  /** The app's ID. */
  id: string;
  /** The installed revision the view was rendered from. */
  revision: number;
  context: string;
  namespace?: string;
  source: ExtensionStreamSource;
}

export type ExtensionStreamCloseReason =
  | "completed"
  | "cancelled"
  | "viewClosed"
  | "appDisabled"
  | "appUpdated"
  | "appRemoved";

export type ExtensionStreamErrorCode = "source" | "rateLimited";

/** How a stream ended. A `close` is an ending; an `error` is a failure. */
export type ExtensionStreamEnd =
  | { type: "close"; reason: ExtensionStreamCloseReason }
  | { type: "error"; code: ExtensionStreamErrorCode; message: string };

export interface ExtensionStreamHandlers<T = unknown> {
  onData: (data: T, seq: number) => void;
  onEnd?: (end: ExtensionStreamEnd) => void;
}

export interface ExtensionStream {
  readonly stream: string;
  /** Ask the host to end this stream. Harmless once it has ended. */
  cancel: () => Promise<void>;
}

export interface ExtensionView {
  /** The id the host groups this view's streams by. Unique per mounted view. */
  readonly view: string;
  open: <T = unknown>(request: ExtensionStreamRequest, handlers: ExtensionStreamHandlers<T>) => Promise<ExtensionStream>;
  /** End every stream this view opened. Call it when the view unmounts. */
  close: () => Promise<void>;
}

/**
 * The host's `OpenStreamIn`, field for field and in its spelling. Exported so
 * a test can hold it to `extension-stream-open.json`, which the Rust side
 * deserializes too.
 */
export function extensionStreamPayload(view: string, channel: string, request: ExtensionStreamRequest) {
  const source: ExtensionStreamSource = { kind: request.source.kind, capability: request.source.capability };
  if (request.source.intervalSeconds !== undefined) source.intervalSeconds = request.source.intervalSeconds;
  return {
    id: request.id,
    revision: request.revision,
    view,
    channel,
    context: request.context,
    namespace: request.namespace ?? "",
    source,
  };
}

let viewSeq = 0;
let channelSeq = 0;

/** A view's handle on its streams. `label` names the view for people reading the metrics. */
export function openExtensionView(appId: string, label: string): ExtensionView {
  // The counter restarts in every window and after every reload, and the host
  // groups by this id across the process, so it carries a random part too.
  const view = `${appId}/${label}#${++viewSeq}-${Math.random().toString(36).slice(2, 10)}`;
  let closed = false;

  const open = async <T,>(request: ExtensionStreamRequest, handlers: ExtensionStreamHandlers<T>): Promise<ExtensionStream> => {
    if (closed) throw new Error("This view has closed; its streams cannot be reopened");
    // A Tauri event name allows only [a-zA-Z0-9/:_-], so nothing of the app's goes in it.
    const channel = `extstream:${++channelSeq}-${Math.random().toString(36).slice(2, 10)}`;
    let ended = false;
    let dispose: () => void = () => {};
    dispose = await subscribe(channel, (frame) => {
      if (ended) return;
      const end = terminal(frame);
      if (end) {
        ended = true;
        dispose();
        handlers.onEnd?.(end);
        return;
      }
      if (isData(frame)) handlers.onData(frame.data as T, frame.seq);
    });
    let opened: { stream: string };
    try {
      opened = await invokeCommand<{ stream: string; channel: string }>("extension_stream_open", {
        input: extensionStreamPayload(view, channel, request),
      });
    } catch (e) {
      ended = true;
      dispose();
      throw e;
    }
    const cancel = async () => {
      if (ended) return;
      await invokeCommand<boolean>("extension_stream_cancel", { stream: opened.stream });
    };
    // The view closed while the host was opening: its close could not name this one.
    if (closed) await cancel();
    return { stream: opened.stream, cancel };
  };

  return {
    view,
    open,
    close: async () => {
      if (closed) return;
      closed = true;
      await invokeCommand<number>("extension_stream_close_view", { view });
    },
  };
}

function terminal(frame: unknown): ExtensionStreamEnd | null {
  if (!frame || typeof frame !== "object") return null;
  const f = frame as Record<string, unknown>;
  if (f.type === "close" && typeof f.reason === "string") {
    return { type: "close", reason: f.reason as ExtensionStreamCloseReason };
  }
  if (f.type === "error") {
    return {
      type: "error",
      code: (typeof f.code === "string" ? f.code : "source") as ExtensionStreamErrorCode,
      message: typeof f.message === "string" ? f.message : "The stream failed",
    };
  }
  return null;
}

function isData(frame: unknown): frame is { type: "data"; seq: number; data: unknown } {
  if (!frame || typeof frame !== "object") return false;
  const f = frame as Record<string, unknown>;
  return f.type === "data" && typeof f.seq === "number" && "data" in f;
}

const CLOSED: Record<ExtensionStreamCloseReason, string> = {
  completed: "The stream finished.",
  cancelled: "The stream was cancelled.",
  viewClosed: "The view that opened the stream closed.",
  appDisabled: "The app was disabled.",
  appUpdated: "The app was updated; reopen the view to follow it again.",
  appRemoved: "The app was removed.",
};

/** One sentence for how a stream ended. A failure says what failed. */
export function describeStreamEnd(end: ExtensionStreamEnd): string {
  if (end.type === "close") return CLOSED[end.reason] ?? `The stream ended (${end.reason}).`;
  return end.code === "rateLimited" ? `The host stopped the stream: ${end.message}` : `The stream failed: ${end.message}`;
}

export interface ExtensionStreamMetrics {
  apps: Array<{
    app: string;
    openStreams: number;
    opened: number;
    messages: number;
    bytes: number;
    rateLimited: number;
    refused: number;
    streams: Array<{ stream: string; view: string; revision: number; source: string; messages: number; bytes: number }>;
  }>;
  maxOpenPerApp: number;
  messagesPerSecond: number;
}

/** The host's app stream counters, for the Inspector (#575). Desktop only. */
export const extensionStreamMetrics = () => invokeCapability<ExtensionStreamMetrics>("extensions.streams", {});
