import { startPodExec } from "./exec";
import { startLocalTerminal } from "./terminal";
import type { ExitReason } from "./terminalReconnect";

/** A live terminal session the pane drives. */
export interface TerminalConnection {
  send(data: string): void;
  resize(cols: number, rows: number): void;
  close(): void;
}

/**
 * A pluggable session source for `TerminalPane`. Abstracts the three terminal
 * kinds (in-pod exec, local PTY, node shell) behind one contract so the pane's
 * xterm wiring, resize, search, and reconnect logic live in one place.
 */
export interface TerminalDriver {
  kind: "pod" | "local" | "node";
  /** Whether a fresh session can be opened (drives reconnect/restart). */
  reconnectable: boolean;
  connect(handlers: {
    onData: (chunk: string) => void;
    onExit: (reason: ExitReason) => void;
    initialSize?: { cols: number; rows: number };
  }): Promise<TerminalConnection>;
}

/**
 * In-pod exec (or node shell, when `kind: "node"` with an `nsenter` command
 * override). A backend error maps to an unexpected drop; a clean end is an
 * intentional exit.
 */
export function podExecDriver(opts: {
  context: string;
  namespace: string;
  pod: string;
  container?: string;
  command?: string[];
  kind?: "pod" | "node";
}): TerminalDriver {
  return {
    kind: opts.kind ?? "pod",
    reconnectable: true,
    async connect({ onData, onExit, initialSize }) {
      const session = await startPodExec(
        opts.context,
        opts.namespace,
        opts.pod,
        onData,
        (error) => onExit(error ? { kind: "error", message: error } : { kind: "closed" }),
        opts.container,
        opts.command,
        initialSize,
      );
      return { send: session.send, resize: session.resize, close: session.close };
    },
  };
}

/**
 * The user's local shell scoped to a context. A local PTY only ends when the
 * shell exits, which is intentional — so its exit is always "closed" (the pane
 * offers a manual Restart rather than auto-reconnecting).
 */
export function localTerminalDriver(opts: {
  context: string;
  extraKubeconfigs: string[];
}): TerminalDriver {
  return {
    kind: "local",
    reconnectable: true,
    async connect({ onData, onExit, initialSize }) {
      const session = await startLocalTerminal(
        opts.context,
        opts.extraKubeconfigs,
        onData,
        () => onExit({ kind: "closed" }),
        initialSize,
      );
      return { send: session.send, resize: session.resize, close: session.close };
    },
  };
}
