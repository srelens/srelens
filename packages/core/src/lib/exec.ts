import { invokeCommand, subscribe } from "../transport/transport";

export interface ExecSession {
  /** Send a keystroke / input string to the pod's stdin. */
  send: (data: string) => void;
  /** Resize the remote PTY to match the xterm viewport. */
  resize: (cols: number, rows: number) => void;
  /** Close the session and unsubscribe. */
  close: () => void;
}

// Monotonic within this window, with a random half: a reload restarts the
// counter while a session it opened is still alive on the backend (web mode
// keeps running exec sessions across a browser refresh), and a bare counter
// would hand the next shell a channel the old one is still emitting on.
let execSeq = 0;
function nextChannel(): string {
  return `exec-${execSeq++}-${Math.random().toString(36).slice(2, 10)}`;
}

/**
 * Open an interactive shell into a pod. `onData` receives stdout chunks;
 * `onExit` fires when the session ends (with an optional error). `size` sizes
 * the remote PTY at attach so it matches the panel from the first prompt.
 * Returns a session handle for sending input, resizing, and closing.
 *
 * **Both listeners are registered before the backend is told to start.** The
 * exec task can emit its exit event in the same tick it is spawned — an
 * invalid context, an RBAC refusal — and that is the only exit event there
 * will ever be. Registered after the invoke (even with `on`, which returns
 * while `listen` is still pending), the exit is lost: `onExit` never fires, so
 * the session's row stays attached forever and a node session's privileged
 * debug pod is left running on the node. Hence the caller-supplied `channel`,
 * the same shape {@link startLocalTerminal} uses: the id the backend assigns
 * does not exist yet, so there is nothing else to subscribe on first.
 */
export async function startPodExec(
  context: string,
  namespace: string,
  pod: string,
  onData: (chunk: string) => void,
  onExit: (error: string | null) => void,
  container?: string,
  command?: string[],
  size?: { cols: number; rows: number },
): Promise<ExecSession> {
  const channel = nextChannel();
  const disposeOut = await subscribe(`exec:out:${channel}`, (p) => onData(p as string));
  const disposeExit = await subscribe(`exec:exit:${channel}`, (p) =>
    onExit((p as string | null) ?? null),
  );
  let session: number;
  try {
    session = await invokeCommand<number>("start_pod_exec", {
      context,
      namespace,
      pod,
      container: container ?? null,
      shell: null,
      command: command ?? null,
      channel,
      cols: size?.cols ?? null,
      rows: size?.rows ?? null,
    });
  } catch (e) {
    disposeOut();
    disposeExit();
    throw e;
  }
  return {
    send: (data) => void invokeCommand("exec_input", { session, data }),
    resize: (cols, rows) => void invokeCommand("exec_resize", { session, cols, rows }),
    close: () => {
      disposeOut();
      disposeExit();
      void invokeCommand("exec_close", { session });
    },
  };
}
