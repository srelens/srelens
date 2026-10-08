import { invokeCommand, subscribe } from "../transport/transport";

export interface TerminalSession {
  /** Send keystrokes / pasted input to the shell's stdin. */
  send: (data: string) => void;
  /** Resize the PTY to match the xterm viewport. */
  resize: (cols: number, rows: number) => void;
  /** Close the session and unsubscribe. */
  close: () => void;
}

// Monotonic id so each terminal gets a unique channel.
let terminalSeq = 0;

/**
 * Open a local shell scoped to `context` (kubectl targets it by default).
 * Runs on the user's machine — distinct from in-pod exec. `onData` receives
 * stdout chunks; `onExit` fires when the shell ends.
 *
 * `command`, when given, is run first and the shell follows it — the host
 * starts it as the terminal's first process, so it is not keystrokes that a
 * shell still reading its rc files could swallow (#820). Written in POSIX
 * quoting, as `toKubectl` writes it; the caller has already asked the reader.
 */
export async function startLocalTerminal(
  context: string,
  extraKubeconfigs: string[],
  onData: (chunk: string) => void,
  onExit: () => void,
  size?: { cols: number; rows: number },
  command?: string,
): Promise<TerminalSession> {
  // Unique channel so we can subscribe BEFORE the backend spawns and emits —
  // otherwise the first prompt can race ahead of the listener.
  const channel = `term-${terminalSeq++}-${Math.random().toString(36).slice(2, 10)}`;
  const disposeOut = await subscribe(`term:out:${channel}`, (p) => onData(p as string));
  const disposeExit = await subscribe(`term:exit:${channel}`, () => onExit());
  let session: number;
  try {
    session = await invokeCommand<number>("start_terminal", {
      context,
      extraKubeconfigs,
      channel,
      cols: size?.cols ?? null,
      rows: size?.rows ?? null,
      command: command ?? null,
    });
  } catch (e) {
    disposeOut();
    disposeExit();
    throw e;
  }
  return {
    send: (data) => void invokeCommand("terminal_input", { session, data }),
    resize: (cols, rows) => void invokeCommand("terminal_resize", { session, cols, rows }),
    close: () => {
      disposeOut();
      disposeExit();
      void invokeCommand("terminal_close", { session });
    },
  };
}
