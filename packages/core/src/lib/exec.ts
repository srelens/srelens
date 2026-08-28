import { invokeCommand, on } from "../transport/transport";

export interface ExecSession {
  /** Send a keystroke / input string to the pod's stdin. */
  send: (data: string) => void;
  /** Resize the remote PTY to match the xterm viewport. */
  resize: (cols: number, rows: number) => void;
  /** Close the session and unsubscribe. */
  close: () => void;
}

/**
 * Open an interactive shell into a pod. `onData` receives stdout chunks;
 * `onExit` fires when the session ends (with an optional error). `size` sizes
 * the remote PTY at attach so it matches the panel from the first prompt.
 * Returns a session handle for sending input, resizing, and closing.
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
  const session = await invokeCommand<number>("start_pod_exec", {
    context,
    namespace,
    pod,
    container: container ?? null,
    shell: null,
    command: command ?? null,
    cols: size?.cols ?? null,
    rows: size?.rows ?? null,
  });
  const disposeOut = on(`exec:out:${session}`, (p) => onData(p as string));
  const disposeExit = on(`exec:exit:${session}`, (p) => onExit((p as string | null) ?? null));
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
