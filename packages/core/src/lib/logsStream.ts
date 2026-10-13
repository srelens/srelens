import { isTauri } from "../transport/platform";
import { invokeCommand, subscribe } from "../transport/transport";
import { csrfHeader } from "../transport/webTransport";
import type { LogLine } from "./logBuffer";

/** One pod/container to follow; `label` tags lines when several share a stream. */
export interface LogTarget {
  pod: string;
  container?: string;
  label?: string;
}

export interface LogStream {
  /** Stop following and unsubscribe. */
  stop: () => void;
  /**
   * Lines `[before - limit, before)` of the stream's history on disk (#914).
   * There only when the stream was opened with `spoolMaxBytes`.
   */
  readEarlier?: (before: number, limit: number) => Promise<SpoolPage>;
  /**
   * Save the stream's history, all of it or the last `sinceSeconds`:
   * through the native dialog on the desktop, as a download in a browser.
   * Resolves to where it went, or null when the reader cancelled.
   */
  exportSpool?: (filename: string, sinceSeconds?: number) => Promise<string | null>;
}

/** Where a frame's lines sit: `lines[0]` is `seq`, and `first` is the oldest
 *  line still on disk when the stream keeps history. */
export interface LogFrame {
  seq: number;
  first?: number;
}

/** Lines read back from a stream's history; the page ends where the read asked. */
export interface SpoolPage {
  first: number;
  seq: number;
  lines: LogLine[];
}

/** Connection health of a live-tail stream. */
export type LogStatus = "live" | "reconnecting" | "completed";

/** Per-stream options: how much history to tail and whether to timestamp. */
export interface LogStreamOptions {
  /** Prefix each line with an RFC 3339 timestamp. */
  timestamps?: boolean;
  /** On the first connect, only lines newer than this many seconds ago. */
  sinceSeconds?: number;
  /** On the first connect, how many trailing lines to tail. */
  tailLines?: number;
  /** Keep the stream's history on disk, up to this many bytes (#914). */
  spoolMaxBytes?: number;
  /** Take each frame whole, instead of line by line through `onLine`. */
  onLines?: (lines: LogLine[], frame: LogFrame) => void;
  /** The backend stopped keeping history, and why. The live lines go on. */
  onSpoolFailed?: (reason: string) => void;
}

/** A line as the backend sends it. */
interface WireLine {
  source: string;
  line: string;
  truncated?: boolean;
}

const toLine = (w: WireLine): LogLine =>
  w.truncated ? { source: w.source, text: w.line, truncated: true } : { source: w.source, text: w.line };

// Monotonic id so each stream gets a unique channel.
let streamSeq = 0;

/**
 * Follow logs for one or more pod/container targets. The backend sends lines
 * as frames every 50 ms (#914): `options.onLines` takes each frame whole,
 * otherwise `onLine` fires per line, with the line's source tag (empty for a
 * single target).
 *
 * The listener is registered BEFORE the backend stream starts, so the initial
 * tail lines can't race ahead of the subscription and get dropped.
 *
 * `onStatus` fires per TARGET, not per stream: a stream fans out over many
 * pods, each of which connects and reconnects on its own schedule, and the
 * backend tags every status with the same `source` that target's lines carry
 * (`crates/streams/src/logs.rs`). A caller that tracks the tag can say how
 * many of its targets are streaming; one that ignores it sees only the last
 * event any target happened to send. The tag is the second argument so a
 * caller that only wants the state — classic's `LogsView` — keeps working
 * unchanged.
 *
 * `onLine`'s third argument says the host cut the line at its limit (#747):
 * the backend reads at most so much of one line, drops the rest up to its
 * newline, and marks what it kept (`crates/kube/src/logs.rs`).
 */
export async function startLogStream(
  context: string,
  namespace: string,
  targets: LogTarget[],
  onLine: (source: string, line: string, truncated: boolean) => void,
  onStatus?: (status: LogStatus, source: string) => void,
  options: LogStreamOptions = {},
): Promise<LogStream> {
  if (targets.length === 0) throw new Error("cannot start live logs without a pod target");
  const channel = `logs:line:${++streamSeq}-${Math.random().toString(36).slice(2, 10)}`;
  const dispose = await subscribe(channel, (p) => {
    // Three payloads share the channel (#914): a target's status
    // (`{source, status}`), a frame of lines (`{seq, first?, lines}`), and the
    // one notice that history stopped (`{spool: {failed}}`). A missing tag on
    // a status means the unlabelled source a single-target stream sends.
    if (!p || typeof p !== "object") return;
    if ("status" in p) {
      const { status, source } = p as { status: LogStatus; source?: string };
      onStatus?.(status, source ?? "");
    } else if ("lines" in p) {
      const { seq, first, lines } = p as { seq: number; first?: number; lines: WireLine[] };
      if (options.onLines) options.onLines(lines.map(toLine), first === undefined ? { seq } : { seq, first });
      else for (const l of lines) onLine(l.source, l.line, l.truncated === true);
    } else if ("spool" in p) {
      options.onSpoolFailed?.((p as { spool: { failed: string } }).spool.failed);
    }
  });
  try {
    // Tauri maps these camelCase args to the command's snake_case params.
    await invokeCommand("start_log_stream", {
      context,
      namespace,
      channel,
      timestamps: options.timestamps ?? false,
      sinceSeconds: options.sinceSeconds ?? null,
      tailLines: options.tailLines ?? null,
      spoolMaxBytes: options.spoolMaxBytes ?? null,
      targets: targets.map((t) => ({
        pod: t.pod,
        container: t.container ?? null,
        label: t.label ?? "",
      })),
    });
  } catch (e) {
    dispose();
    throw e;
  }
  const stream: LogStream = {
    stop: () => {
      dispose();
      void invokeCommand("stop_log_stream", { channel });
    },
  };
  if (options.spoolMaxBytes !== undefined) {
    stream.readEarlier = async (before, limit) => {
      const page = await invokeCommand<{ first: number; seq: number; lines: WireLine[] }>(
        "read_log_spool",
        { channel, before, limit },
      );
      return { first: page.first, seq: page.seq, lines: page.lines.map(toLine) };
    };
    stream.exportSpool = (filename, sinceSeconds) =>
      isTauri()
        ? invokeCommand<string | null>("export_log_spool", {
            channel,
            filename,
            sinceSeconds: sinceSeconds ?? null,
          }).then((path) => path ?? null)
        : downloadSpool(channel, filename, sinceSeconds);
  }
  return stream;
}

/**
 * In a browser the server streams the history (`GET /api/logs/export`) and the
 * browser saves it as a download.
 *
 * ponytail: the whole file sits in a Blob first; a short-lived download token
 * would let a plain link stream it straight to disk.
 */
async function downloadSpool(channel: string, filename: string, sinceSeconds?: number): Promise<string> {
  const query = new URLSearchParams({ channel });
  if (sinceSeconds !== undefined) query.set("sinceSeconds", String(sinceSeconds));
  const res = await fetch(`/api/logs/export?${query}`, { credentials: "include", headers: csrfHeader() });
  if (!res.ok) {
    // Over HTTP/2 there is no reason phrase, so `statusText` can be empty.
    let message = res.statusText || `HTTP ${res.status}`;
    try {
      const data = JSON.parse(await res.text()) as { error?: unknown };
      if (data.error !== undefined) message = String(data.error);
    } catch {
      // Not JSON: the status text is all there is.
    }
    throw new Error(message);
  }
  const url = URL.createObjectURL(await res.blob());
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
  return filename;
}
