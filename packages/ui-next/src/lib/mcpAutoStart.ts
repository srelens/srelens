import { useSyncExternalStore } from "react";

/**
 * Where the shell's MCP auto-start is: not begun, in flight, or finished.
 *
 * #374 item 2 left one half of itself behind. `Window` brings the HTTP server
 * back up when the reader left it enabled, and it does that at the moment
 * `LockGate` reports the vault usable; `McpServer` reads `mcpHttpStatus()` in
 * its own effect at mount. When a saved Settings tab is restored with the server
 * enabled, both happen at once — and `mcp_http_start` binds the listener before
 * `McpHttpManager` records anything as running, so the pane's read can
 * legitimately answer `null` while the bind is still in flight. Nothing told it
 * afterwards. The pane then sat permanently on `not running` offering a Start
 * button, and pressing that restarts a live server and drops every agent
 * request in flight.
 *
 * **A settlement alone fixed half of that.** The first version of this store
 * was a counter that changed when the shell's start had finished, and the pane
 * re-read its status on the change — which closes the "permanently" and leaves
 * the window open. Between `Window` calling `startMcpHttp` and that call
 * settling — up to two seconds, since `stop_running` (`mcp.rs`) waits that long
 * on a listener a webview reload left behind — the pane's read answers `null`,
 * it offers Start, and a click queues a SECOND `startMcpHttp` that tears down
 * the server the first has just brought up, dropping requests. The pane could
 * not refuse the click because a count of settlements does not say whether one
 * is in flight now. So this carries the state: {@link McpAutoStartPhase}.
 *
 * **Three states, not a boolean.** "Not begun" and "finished" are both
 * not-in-flight, and a boolean would collapse them; this codebase has done
 * that four times and every one was a defect. The pane needs both edges —
 * `starting` to take Start away and say why, the transition INTO `settled` to
 * take its own read again — and only a value that names all three has both.
 *
 * **A state about the shell's CALL, not about the listener.** What crosses here
 * is where `Window`'s `startMcpHttp` is; the pane answers `settled` by taking
 * its own `mcpHttpStatus()` read again. It deliberately does NOT carry the URL
 * the start returned: `running` on that pane is a live read of the process and
 * not a stored flag (see the file comment there), and a value published from
 * here would be a second source of truth for it — one that could lie in the
 * other direction, claiming a listener over a server that came up and then fell
 * over. The extra round trip buys the pane's own reasoning intact.
 *
 * `settled` on a refused start as well as a successful one. The refusal is
 * swallowed where it happens, as classic swallows it, and the pane's Start
 * button is where a reader finds out — but the status is worth re-reading
 * either way, and Start has to come back either way: `starting` is a claim
 * about the call, and it ends when the call does.
 *
 * **The token is not part of this.** `McpServer` re-reads the bearer after its
 * OWN start because the first start mints one, and this store might look like it
 * owes the same. It does not: `revoke()` and a refused `start()` both persist
 * `enabled: false`, so a persisted `enabled: true` is only ever left behind by a
 * start that succeeded — which had already minted the token. The pane's token
 * read at mount is accurate whatever the listener is doing, because
 * `getMcpToken()` does not depend on one.
 *
 * Module-level, and in this package rather than in core, for the reason the
 * other shell stores are: the shell writes it and a screen reads it, and a
 * screen receives only `{ route }`, so a prop could never reach one.
 */
export type McpAutoStartPhase = "idle" | "starting" | "settled";

let phase: McpAutoStartPhase = "idle";
const listeners = new Set<() => void>();

function publish(next: McpAutoStartPhase): void {
  if (phase === next) return;
  phase = next;
  // A copy, because a listener may unsubscribe while this is running.
  for (const listener of [...listeners]) listener();
}

/** The shell is about to call `startMcpHttp`. Marked BEFORE the call, so no
 *  render can see the call in flight and the store not saying so. */
export function mcpAutoStartStarting(): void {
  publish("starting");
}

/**
 * The shell's auto-start has finished. Called by `Window` for both outcomes;
 * see above for why the outcome is not carried.
 */
export function mcpAutoStartSettled(): void {
  publish("settled");
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The current phase, for a caller that is not inside a render. */
export function mcpAutoStartPhase(): McpAutoStartPhase {
  return phase;
}

/**
 * The current phase, subscribed. A string, not an object: `useSyncExternalStore`
 * re-reads its snapshot after every render and compares by identity, so a getter
 * that allocates never settles — the same note `lib/clusters.ts` carries, which
 * shipped that bug once.
 */
export function useMcpAutoStart(): McpAutoStartPhase {
  return useSyncExternalStore(subscribe, mcpAutoStartPhase, mcpAutoStartPhase);
}

/**
 * Reset the module between tests, the way `resetLock` and `resetContexts` do for
 * their stores. Not called by anything shipped: vitest isolates files, not the
 * tests inside one, so a phase reached in one test would otherwise still be
 * current in the next.
 */
export function resetMcpAutoStart(): void {
  publish("idle");
}
