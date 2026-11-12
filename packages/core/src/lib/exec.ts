import { invokeCommand, subscribe } from "../transport/transport";
import { isApplePlatform } from "./shortcuts";

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
 * How long to wait for a subscription to be acknowledged before giving up on
 * the shell.
 *
 * Exported so a test can wait exactly this long rather than hard-coding a
 * number that would go quietly stale beside it.
 */
export const SUBSCRIBE_TIMEOUT_MS = 10_000;

/**
 * {@link subscribe}, with an end to the waiting.
 *
 * On the DESKTOP `subscribe` is Tauri's `listen`, which settles on its own. On
 * the WEB it resolves only when the `subbed` ack comes back over the socket,
 * and `wsClient` has no timeout: if the socket is down it retries with a
 * backoff up to 10s, forever, and this promise stays pending forever with it.
 * So: web session open, server dies, reader clicks "Open shell" — no error, no
 * rejection, a spinner that never resolves. The project's rule is that a
 * timeout surfaces as an error.
 *
 * Before the subscribe-before-start reorder, exec used fire-and-forget `on()`
 * and the first failure came from `invokeCommand` over HTTP, which rejects
 * promptly. The reorder is right and stays — the exit event can be emitted in
 * the same tick the session spawns, and it is the only one there will ever be —
 * so the wait it introduced is what gets a bound.
 *
 * The message says "timed out" on purpose: `describeError` classifies on it, so
 * the reader gets "Request timed out" and a remedy for their platform instead of
 * a raw string, with `subject` naming what actually timed out in the original it
 * keeps.
 */
async function subscribeWithin(
  channel: string,
  subject: string,
  handler: (payload: unknown) => void,
): Promise<() => void> {
  const pending = subscribe(channel, handler);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const bound = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(
      () =>
        reject(
          new Error(
            `subscribing to the ${subject} timed out after ${SUBSCRIBE_TIMEOUT_MS / 1000}s (the srelens server never acknowledged it)`,
          ),
        ),
      SUBSCRIBE_TIMEOUT_MS,
    );
  });
  try {
    return await Promise.race([pending, bound]);
  } catch (e) {
    // The subscription may still land after the wait was given up — the web
    // transport registers the handler SYNCHRONOUSLY and awaits only the ack, so
    // a late `subbed` would leave a live handler on a channel nobody reads, and
    // `wsClient` resubscribes it on every reconnect. Dispose it whenever it
    // arrives.
    void pending.then(
      (dispose) => dispose(),
      () => {},
    );
    throw e;
  } finally {
    clearTimeout(timer);
  }
}

/**
 * The command a pod or node shell runs: `/bin/sh` handing over to the first
 * shell the image has of zsh then bash on a Mac, or bash elsewhere, and
 * staying in `/bin/sh` without one. Plain `/bin/sh` is dash on Debian and
 * Ubuntu images, which has no line editing or bracketed paste, so arrow keys
 * print `^[[A` and a pasted block runs line by line (#1040). zsh is macOS's
 * login shell. Busybox images have neither.
 *
 * Picked here, by the reader's own OS, rather than by the backend's: in web
 * mode the server's OS is not the reader's. The TUI runs the same script from
 * `srelens_kube::exec::shell_command`.
 */
export function interactiveShell(platform?: string): string[] {
  const shells = isApplePlatform(platform) ? "zsh bash" : "bash";
  return ["/bin/sh", "-c", `for s in ${shells}; do command -v $s >/dev/null 2>&1 && exec $s; done; exec /bin/sh`];
}

/**
 * Open an interactive shell into a pod. `onData` receives stdout chunks;
 * `onExit` fires when the session ends (with an optional error). `size` sizes
 * the remote PTY at attach so it matches the panel from the first prompt.
 * `command` defaults to {@link interactiveShell}.
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
  // Bounded, so a socket that never acknowledges is an error and not a spinner
  // — see {@link subscribeWithin}. Ordered so the second failing still drops
  // the first: an exec session is never started below unless BOTH listeners are
  // live, because a session with no listener on its exit channel is a row
  // attached forever and, for a node session, a privileged debug pod left
  // running on the node.
  const disposeOut = await subscribeWithin(`exec:out:${channel}`, "shell's output", (p) =>
    onData(p as string),
  );
  let disposeExit: () => void;
  try {
    disposeExit = await subscribeWithin(`exec:exit:${channel}`, "shell's exit", (p) =>
      onExit((p as string | null) ?? null),
    );
  } catch (e) {
    disposeOut();
    throw e;
  }
  let session: number;
  try {
    session = await invokeCommand<number>("start_pod_exec", {
      context,
      namespace,
      pod,
      container: container ?? null,
      shell: null,
      command: command ?? interactiveShell(),
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
