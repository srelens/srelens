import { invokeCommand, on } from "../transport/transport";

export interface TerminalSession {
  /** Send keystrokes / pasted input to the shell's stdin. */
  send: (data: string) => void;
  /** Resize the PTY to match the xterm viewport. */
  resize: (cols: number, rows: number) => void;
  /** Close the session and unsubscribe. */
  close: () => void;
}

/**
 * Open a local shell scoped to `context` (kubectl targets it by default).
 * Runs on the user's machine — distinct from in-pod exec. `onData` receives
 * stdout chunks; `onExit` fires when the shell ends.
 */
export async function startLocalTerminal(
  context: string,
  extraKubeconfigs: string[],
  onData: (chunk: string) => void,
  onExit: () => void,
  size?: { cols: number; rows: number },
): Promise<TerminalSession> {
  const session = await invokeCommand<number>("start_terminal", {
    context,
    extraKubeconfigs,
    cols: size?.cols ?? null,
    rows: size?.rows ?? null,
  });
  const disposeOut = on(`term:out:${session}`, (p) => onData(p as string));
  const disposeExit = on(`term:exit:${session}`, () => onExit());
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
