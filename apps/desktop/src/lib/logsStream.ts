import { invokeCommand, subscribe } from "../transport/transport";

/** One pod/container to follow; `label` tags lines when several share a stream. */
export interface LogTarget {
  pod: string;
  container?: string;
  label?: string;
}

export interface LogStream {
  /** Stop following and unsubscribe. */
  stop: () => void;
}

/** Connection health of a live-tail stream. */
export type LogStatus = "live" | "reconnecting";

/** Per-stream options: how much history to tail and whether to timestamp. */
export interface LogStreamOptions {
  /** Prefix each line with an RFC 3339 timestamp. */
  timestamps?: boolean;
  /** On the first connect, only lines newer than this many seconds ago. */
  sinceSeconds?: number;
  /** On the first connect, how many trailing lines to tail. */
  tailLines?: number;
}

// Monotonic id so each stream gets a unique channel.
let streamSeq = 0;

/**
 * Follow logs for one or more pod/container targets. `onLine` fires for each
 * line as it arrives, with the line's source tag (empty for a single target).
 *
 * The listener is registered BEFORE the backend stream starts, so the initial
 * tail lines can't race ahead of the subscription and get dropped.
 */
export async function startLogStream(
  context: string,
  namespace: string,
  targets: LogTarget[],
  onLine: (source: string, line: string) => void,
  onStatus?: (status: LogStatus) => void,
  options: LogStreamOptions = {},
): Promise<LogStream> {
  if (targets.length === 0) throw new Error("cannot start live logs without a pod target");
  const channel = `logs:line:${++streamSeq}`;
  const dispose = await subscribe(channel, (p) => {
    // The backend emits either a log line (`{source, line}`) or a `{status}`.
    if (p && typeof p === "object" && "status" in p) {
      onStatus?.((p as { status: LogStatus }).status);
    } else {
      const { source, line } = p as { source: string; line: string };
      onLine(source, line);
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
  return {
    stop: () => {
      dispose();
      void invokeCommand("stop_log_stream", { channel });
    },
  };
}
