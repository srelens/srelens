import { useSyncExternalStore } from "react";

/**
 * That the shell's MCP auto-start has SETTLED — nothing about how.
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
 * **A signal, not a status.** What crosses this store is one fact — the shell's
 * start has finished, either way — and the pane answers it by taking its own
 * `mcpHttpStatus()` read again. It deliberately does NOT carry the URL the start
 * returned: `running` on that pane is a live read of the process and not a
 * stored flag (see the file comment there), and a value published from here
 * would be a second source of truth for it — one that could lie in the other
 * direction, claiming a listener over a server that came up and then fell over.
 * The extra round trip buys the pane's own reasoning intact.
 *
 * A counter rather than a boolean, because what an effect needs is something
 * that CHANGES. The value itself means nothing; only that it is not what it was
 * the last time the effect ran.
 *
 * Announced on a refused start as well as a successful one. The refusal is
 * swallowed where it happens, as classic swallows it, and the pane's Start
 * button is where a reader finds out — but the status is worth re-reading
 * either way, and a signal that only fired on success would be one the pane
 * could not distinguish from a start that never happened.
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
let settlements = 0;
const listeners = new Set<() => void>();

/**
 * The shell's auto-start has finished. Called by `Window` for both outcomes;
 * see above for why the outcome is not carried.
 */
export function mcpAutoStartSettled(): void {
  settlements += 1;
  // A copy, because a listener may unsubscribe while this is running.
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** How many have settled, for a caller that is not inside a render. */
export function mcpAutoStartSettlements(): number {
  return settlements;
}

/**
 * The current count, subscribed. A number, not an object: `useSyncExternalStore`
 * re-reads its snapshot after every render and compares by identity, so a getter
 * that allocates never settles — the same note `lib/clusters.ts` carries, which
 * shipped that bug once.
 */
export function useMcpAutoStart(): number {
  return useSyncExternalStore(subscribe, mcpAutoStartSettlements, mcpAutoStartSettlements);
}

/**
 * Reset the module between tests, the way `resetLock` and `resetContexts` do for
 * their stores. Not called by anything shipped: vitest isolates files, not the
 * tests inside one, so a settlement announced in one test would otherwise still
 * be counted in the next.
 */
export function resetMcpAutoStart(): void {
  settlements = 0;
  for (const listener of [...listeners]) listener();
}
