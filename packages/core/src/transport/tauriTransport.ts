import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { relaunch } from "@tauri-apps/plugin-process";
import { parseClusterLoginRequired, requestClusterLogin } from "../lib/clusterLogin";

/** Request/response to a backend capability. */
export async function invokeCapability<T>(id: string, input: unknown = null): Promise<T> {
  try {
    if ((id === "extensions.packageManifest" || id === "extensions.configure")
      && input !== null && typeof input === "object" && "package" in input
      && input.package instanceof Uint8Array) {
      const { package: packageBytes, ...metadata } = input;
      return await invoke<T>("invoke_package_capability", packageBytes, {
        headers: { "x-srelens-package-input": JSON.stringify({ id, input: metadata }) },
      });
    }
    return await invoke<T>("invoke_capability", { id, input });
  } catch (e) {
    const login = parseClusterLoginRequired(e);
    if (login) {
      requestClusterLogin(login);
      throw new Error("cluster_login_required");
    }
    throw e;
  }
}

/**
 * The commands that open a stream on a channel of the page's own (#733): each
 * is passed `onEvent`, and the host sends the stream's frames on it.
 */
const STREAMS_ON_A_CHANNEL = new Set(["start_resource_watch", "start_pod_exec", "extension_stream_open"]);

/**
 * The commands that open something the calling window owns (#700, #735). The
 * host ends a window's streams when it closes; a reload keeps the window and
 * loses the page, so the new page asks the host to end what the old one held —
 * and none of these may run before that, or the reset would end the new page's
 * stream as well. Log streams, port-forwards, terminals and helm operations
 * broadcast their output on events instead, so they wait without a channel.
 */
const OPENS_A_STREAM = new Set([
  ...STREAMS_ON_A_CHANNEL,
  "start_log_stream",
  "start_port_forward",
  "start_terminal",
  "start_helm_op",
]);

let windowReset: Promise<void> | null = null;

/**
 * End every stream this window held before this page loaded: once a page, and
 * before its first stream opens. Called as the transport loads, and awaited by
 * every command that opens a stream. The host reads the window from the call
 * itself, so a page can only ever end its own window's streams.
 *
 * Opens that arrive during one attempt share it. A failed attempt rejects them
 * all and is forgotten, so the next open tries again; a successful one is
 * never repeated, since a second reset would end this page's own streams.
 */
export function resetWindowStreams(): Promise<void> {
  if (!windowReset) {
    const attempt: Promise<void> = invoke("window_streams_reset").then(
      () => {},
      (e: unknown) => {
        if (windowReset === attempt) windowReset = null;
        console.warn("srelens: could not end this window's streams from before the reload", e);
        throw e;
      },
    );
    windowReset = attempt;
  }
  return windowReset;
}

/** One frame of a stream, as the host sends it on the opener's channel. */
interface StreamFrame {
  event: string;
  payload: unknown;
}

/**
 * This page's subscriptions, by event name. A stream's frames arrive on the
 * channel passed with its open, not as events (#733), and are handed out here.
 */
const subscriptions = new Map<string, Set<(payload: unknown) => void>>();

function hold(channel: string, handler: (payload: unknown) => void): () => void {
  let handlers = subscriptions.get(channel);
  if (!handlers) subscriptions.set(channel, (handlers = new Set()));
  handlers.add(handler);
  return () => {
    handlers.delete(handler);
    if (handlers.size === 0 && subscriptions.get(channel) === handlers) subscriptions.delete(channel);
  };
}

function deliver({ event, payload }: StreamFrame): void {
  for (const handler of [...(subscriptions.get(event) ?? [])]) handler(payload);
}

/**
 * Invoke a raw Tauri command (for streaming primitives like watches). A
 * command that opens a stream fails closed: until the old page's streams are
 * ended, it is not sent, so it can never be ended by a later reset either.
 *
 * A command that opens a stream is also passed `onEvent`, a channel of its
 * own. The host sends the stream's frames on it, and Tauri answers a channel
 * only in the page that made the call, so no other window receives them
 * (#733). Each open needs its own: Tauri numbers a channel's messages from
 * the host end and closes it when that stream ends.
 */
export async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (OPENS_A_STREAM.has(command)) {
    try {
      await resetWindowStreams();
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      throw new Error(
        `Not opened: srelens could not end this window's streams from before the reload (${reason}). Try again.`,
      );
    }
    if (STREAMS_ON_A_CHANNEL.has(command)) {
      return invoke<T>(command, { ...args, onEvent: new Channel<StreamFrame>(deliver) });
    }
  }
  return invoke<T>(command, args);
}

/**
 * Subscribe to an event: a broadcast (mirrors ipcRendererOn /
 * broadcastMessage), or the frames of a stream this page opens on `channel`.
 */
export function on(channel: string, handler: (payload: unknown) => void): () => void {
  const release = hold(channel, handler);
  const unlistenPromise = listen(channel, (event) => handler(event.payload));
  let disposed = false;
  unlistenPromise.then((un) => {
    if (disposed) un();
  });
  return () => {
    disposed = true;
    release();
    unlistenPromise.then((un) => un());
  };
}

/**
 * Subscribe and await registration before resolving. Use this (not `on`) when
 * the backend starts emitting as soon as it's invoked: subscribe first, then
 * start the producer, so the initial emission can't race ahead of the listener.
 */
export async function subscribe(
  channel: string,
  handler: (payload: unknown) => void,
): Promise<() => void> {
  const release = hold(channel, handler);
  try {
    const unlisten = await listen(channel, (event) => handler(event.payload));
    return () => {
      release();
      unlisten();
    };
  } catch (e) {
    release();
    throw e;
  }
}

/** Restart the app (used after an update is installed). */
export async function relaunchApp(): Promise<void> {
  return relaunch();
}

/** The running app's bundle version. */
export async function appVersion(): Promise<string> {
  return getVersion();
}

/** Set the webview's native zoom level (1 = 100%) — the #237 interface scale. */
export async function setWebviewZoom(factor: number): Promise<void> {
  await getCurrentWebview().setZoom(factor);
}

/**
 * Intercept window close request, run an async cleanup handler (e.g. flushing
 * state writes to disk), then destroy the window.
 *
 * The timeout is a *stall* guard, not a success: if the handler has not settled
 * by then, the FIRST close is refused so the user can try again, because
 * destroying over an in-flight flush drops the settings write with the webview.
 *
 * But a refusal cannot be the standing answer. The close was already
 * prevented, so an inert red light is all the user sees, and a cleanup that
 * keeps exceeding the timeout — a wedged backend, a disk that has stopped
 * answering — made the window impossible to close at all. So the refusals are
 * bounded: the second timed-out attempt destroys the window anyway. That is
 * the decision the classic path already recorded for the same reason (#425) —
 * "a best-effort flush must never cost the user the ability to quit" — reached
 * here after one retry rather than immediately, since here there is a retry to
 * be had.
 *
 * A handler that THROWS is a different fact: it settled, and it failed. That
 * still refuses the close (the caller asked to be told before the window goes)
 * and it counts toward the same bound, so a handler that throws every time
 * cannot wedge the window either.
 */
export function onWindowCloseRequested(
  handler: () => Promise<void> | void,
  timeoutMs = 500,
  /** How many refused attempts before the window is destroyed regardless. */
  maxRefusals = 1,
): () => void {
  const win = getCurrentWindow();
  if (typeof win?.onCloseRequested !== "function") return () => {};
  let unlisten: (() => void) | undefined;
  let disposed = false;
  let closing = false;
  // How many times this window has refused to close because cleanup did not
  // finish. Never reset: the point is to bound the refusals over the window's
  // life, not per burst of clicks.
  let refusals = 0;
  // Only the intentional `win.close()` fallback may proceed without our
  // preventDefault — a second user click while a flush is in flight must not.
  let allowClose = false;

  /** Close for good: destroy, and fall back to close() if destroy is refused. */
  const finish = async () => {
    try {
      await win.destroy();
    } catch {
      // destroy() can be refused when `core:window:allow-destroy` was not
      // granted. close() re-emits this event, so it is the last resort rather
      // than the path, and `allowClose` stops it looping.
      allowClose = true;
      try {
        await win.close();
      } finally {
        allowClose = false;
        closing = false;
      }
    }
  };

  void win
    .onCloseRequested(async (event) => {
      if (allowClose) return;
      event.preventDefault();
      if (closing) return;
      closing = true;
      let settled = false;
      let threw = false;
      try {
        await Promise.race([
          Promise.resolve(handler()).then(() => {
            settled = true;
          }),
          new Promise<void>((resolve) => setTimeout(resolve, timeoutMs)),
        ]);
      } catch {
        threw = true;
      }
      if (!settled || threw) {
        refusals += 1;
        if (refusals <= maxRefusals) {
          closing = false;
          return;
        }
        // Out of retries. The flush is best effort; being able to close the
        // window is not.
      }
      await finish();
    })
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** The window label assigned by Tauri ("main", "ctx-...", etc.). */
export function currentWindowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}
