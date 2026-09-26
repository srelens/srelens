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
import type { LogStatus, LogStream, LogStreamOptions } from "./logsStream";

/** What an exec session's host confirmation named: the session runs only when it names exactly this. */
export interface ExtensionExecConfirmation {
  pod: string;
  container: string;
  command: string[];
}

/**
 * What a stream carries. `read` re-runs a declared reader every `intervalSeconds`
 * (5–300, default 15). `watch` follows the kind a declared reader lists and says
 * when it changed, as an {@link ExtensionWatchEvent} (#566).
 *
 * The pod sources (#567) go through an app's pod binding (`capability`), to a pod
 * its scope admits: `name` is the object whose pods, for a binding scoped by
 * `resource`. `logs` follows one container ({@link ExtensionLogEvent}); `exec` runs
 * the binding's command once, and only with `confirmed` naming exactly the pod,
 * container and command the host confirmation showed ({@link ExtensionExecEvent});
 * `portForward` listens on a local port the host picks, to `pod` or through
 * `service` ({@link ExtensionForwardEvent}).
 */
export type ExtensionStreamSource =
  | { kind: "read"; capability: string; intervalSeconds?: number }
  | { kind: "watch"; capability: string }
  | {
      kind: "logs";
      capability: string;
      name?: string;
      pod: string;
      container?: string;
      tailLines?: number;
      sinceSeconds?: number;
      timestamps?: boolean;
    }
  | {
      kind: "exec";
      capability: string;
      name?: string;
      pod: string;
      container?: string;
      confirmed?: ExtensionExecConfirmation;
    }
  | { kind: "portForward"; capability: string; name?: string; pod?: string; service?: string };

/**
 * A log source's `data` (#567): lines, each tagged `pod/container`, and each
 * source's connection state. The same shape a log provider will send (#569), so
 * the pod log view follows any source through one path.
 */
export type ExtensionLogEvent =
  | { event: "lines"; lines: Array<{ source: string; line: string; truncated?: boolean }>; dropped?: number }
  | { event: "status"; source: string; status: LogStatus; message?: string };

/** An exec session's `data`: output as it arrives, then the command's exit code. */
export type ExtensionExecEvent =
  | { event: "output"; chunks: Array<{ stream: "stdout" | "stderr"; text: string }> }
  | { event: "exit"; code: number };

/**
 * A port-forward's `data`: it is listening on `localPort` of this computer, and
 * reaches `port` of `pod` (through `service`'s `servicePort`, for a forward through
 * a Service). Sent again when a Service's forward follows another pod.
 */
export type ExtensionForwardEvent = {
  event: "ready";
  localPort: number;
  pod: string;
  port: number;
  service?: string;
  servicePort?: number;
};

const record = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;

export function isExtensionLogEvent(data: unknown): data is ExtensionLogEvent {
  const d = record(data);
  if (d?.event === "lines") {
    return (
      Array.isArray(d.lines) &&
      d.lines.every((l) => typeof record(l)?.source === "string" && typeof record(l)?.line === "string") &&
      (d.dropped === undefined || typeof d.dropped === "number")
    );
  }
  return (
    d?.event === "status" &&
    typeof d.source === "string" &&
    (d.status === "live" || d.status === "reconnecting" || d.status === "completed")
  );
}

export function isExtensionExecEvent(data: unknown): data is ExtensionExecEvent {
  const d = record(data);
  if (d?.event === "exit") return typeof d.code === "number";
  return (
    d?.event === "output" &&
    Array.isArray(d.chunks) &&
    d.chunks.every((c) => (record(c)?.stream === "stdout" || record(c)?.stream === "stderr") && typeof record(c)?.text === "string")
  );
}

export function isExtensionForwardEvent(data: unknown): data is ExtensionForwardEvent {
  const d = record(data);
  return d?.event === "ready" && typeof d.localPort === "number" && typeof d.pod === "string" && typeof d.port === "number";
}

/**
 * A `watch` stream's `data`. Never an object: `synced` (a full list completed —
 * read again) and `changed` (read again) ask the view to re-read through its own
 * path; `reconnecting` says what the view shows is not current until the next
 * `synced`.
 */
export type ExtensionWatchEvent =
  | { event: "synced" }
  | { event: "changed" }
  | { event: "reconnecting"; message: string };

export function isExtensionWatchEvent(data: unknown): data is ExtensionWatchEvent {
  if (!data || typeof data !== "object") return false;
  const d = data as Record<string, unknown>;
  if (d.event === "synced" || d.event === "changed") return true;
  return d.event === "reconnecting" && typeof d.message === "string";
}

/** The desktop host announces every app inventory write on this channel. */
export const EXTENSION_INVENTORY_CHANNEL = "extensions:inventory";

/**
 * Hear every app inventory write the host announces (#566). Resolves to the
 * unsubscribe once listening; rejects when the channel is unavailable, so a
 * caller can fall back and say it did.
 */
export async function onExtensionInventoryChanged(listener: () => void): Promise<() => void> {
  return subscribe(EXTENSION_INVENTORY_CHANNEL, () => listener());
}

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
  | "appRemoved"
  // The window that opened it closed or reloaded (#700). The page that owned
  // the stream is gone, so these are never followed by a reopen.
  | "windowClosed"
  | "windowReloaded";

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
  const source = sourcePayload(request.source);
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

/** Only the defined ones of `fields`, so an absent option is absent rather than `undefined`. */
function defined<T extends Record<string, unknown>>(fields: T): Partial<T> {
  return Object.fromEntries(Object.entries(fields).filter(([, value]) => value !== undefined)) as Partial<T>;
}

/**
 * The source, field by field and per kind, so nothing the host refuses — an
 * interval on a watch, a command or a local port on a pod source — is sent.
 */
function sourcePayload(asked: ExtensionStreamSource): ExtensionStreamSource {
  switch (asked.kind) {
    case "watch":
      return { kind: "watch", capability: asked.capability };
    case "logs":
      return {
        kind: "logs",
        capability: asked.capability,
        ...defined({ name: asked.name }),
        pod: asked.pod,
        ...defined({
          container: asked.container,
          tailLines: asked.tailLines,
          sinceSeconds: asked.sinceSeconds,
          timestamps: asked.timestamps,
        }),
      };
    case "exec":
      return {
        kind: "exec",
        capability: asked.capability,
        ...defined({ name: asked.name }),
        pod: asked.pod,
        ...defined({ container: asked.container }),
        ...(asked.confirmed
          ? {
              confirmed: {
                pod: asked.confirmed.pod,
                container: asked.confirmed.container,
                command: [...asked.confirmed.command],
              },
            }
          : {}),
      };
    case "portForward":
      return {
        kind: "portForward",
        capability: asked.capability,
        ...defined({ name: asked.name, pod: asked.pod, service: asked.service }),
      };
    default:
      return {
        kind: "read",
        capability: asked.capability,
        ...(asked.intervalSeconds !== undefined ? { intervalSeconds: asked.intervalSeconds } : {}),
      };
  }
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
  windowClosed: "The window that opened the stream closed.",
  windowReloaded: "The window that opened the stream reloaded.",
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
    /** Streams ended because the window that opened them closed or reloaded (#700). */
    windowEnded: number;
    streams: Array<{ stream: string; view: string; revision: number; source: string; messages: number; bytes: number }>;
  }>;
  maxOpenPerApp: number;
  messagesPerSecond: number;
}

/** The host's app stream counters, for the Inspector (#575). Desktop only. */
export const extensionStreamMetrics = () => invokeCapability<ExtensionStreamMetrics>("extensions.streams", {});

/** What `extensions.pods` is asked: one pod binding, and the object whose pods (#567). */
export interface ExtensionPodsRequest {
  id: string;
  revision: number;
  /** The pod binding's name. */
  capability: string;
  context: string;
  namespace: string;
  /** The object whose pods, for a binding scoped by `resource`. */
  name?: string;
}

export interface ExtensionPods {
  pods: Array<{ name: string; namespace: string; containers: string[]; phase?: string | null; ready: boolean }>;
  /** For a port-forward through a Service: the Services that reach a pod in scope. */
  services?: Array<{ name: string; port: number }>;
  /** More pods than the host lists; the list is not all of them. */
  truncated?: boolean;
  /** The scope, in the host's words: `pods selected by Deployment web`. */
  scope: string;
}

/** The pods (and Services) one of an app's pod bindings may reach now, as the host holds them to its scope. */
export const extensionPods = (request: ExtensionPodsRequest) =>
  invokeCapability<ExtensionPods>("extensions.pods", {
    id: request.id,
    revision: request.revision,
    capability: request.capability,
    context: request.context,
    namespace: request.namespace,
    ...(request.name !== undefined ? { name: request.name } : {}),
  });

export interface ExtensionLogStreamHandlers {
  /** Lines the host dropped because they arrived faster than it sends them. */
  onDropped?: (count: number) => void;
  /** How the stream ended: a close is an ending, an error a failure. */
  onEnd?: (end: ExtensionStreamEnd) => void;
}

/**
 * Follow an app's `logs` source (#567) through the callbacks `startLogStream`
 * takes, so the pod log view's buffer, pause and status counting follow it
 * unchanged — and a log provider's stream later (#569), which sends the same
 * {@link ExtensionLogEvent}s. `options` fill in what the source leaves unset.
 */
export async function startExtensionLogStream(
  view: ExtensionView,
  request: ExtensionStreamRequest & { source: Extract<ExtensionStreamSource, { kind: "logs" }> },
  onLine: (source: string, line: string) => void,
  onStatus?: (status: LogStatus, source: string) => void,
  handlers: ExtensionLogStreamHandlers = {},
  options: LogStreamOptions = {},
): Promise<LogStream> {
  let ended = false;
  const stream = await view.open(
    {
      ...request,
      source: {
        ...request.source,
        tailLines: request.source.tailLines ?? options.tailLines,
        sinceSeconds: request.source.sinceSeconds ?? options.sinceSeconds,
        timestamps: request.source.timestamps ?? options.timestamps,
      },
    },
    {
      onData: (data) => {
        if (!isExtensionLogEvent(data)) return;
        if (data.event === "status") {
          onStatus?.(data.status, data.source);
          return;
        }
        for (const line of data.lines) onLine(line.source, line.line);
        if (data.dropped) handlers.onDropped?.(data.dropped);
      },
      onEnd: (end) => {
        ended = true;
        handlers.onEnd?.(end);
      },
    },
  );
  return {
    stop: () => {
      if (!ended) void stream.cancel();
    },
  };
}
