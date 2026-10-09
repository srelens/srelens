import { useEffect, useSyncExternalStore } from "react";

/**
 * Which tabs have a log stream open and not finished — what Home's "Live now"
 * lists, rather than every `/logs/…` route.
 *
 * A logs tab is not a stream: its subject may not resolve, may have nothing to
 * follow, or its stream may have failed or completed, and in all of those the
 * route is the same. So the stream says so itself: `LogsStream` marks its tab
 * while its stream is connecting, live or reconnecting, and unmarks it when the
 * stream ends or the screen goes.
 */
let live: ReadonlySet<string> = new Set();
const listeners = new Set<() => void>();

/** The tabs streaming now. A new set on every change, so its identity is the snapshot. */
export function liveLogStreams(): ReadonlySet<string> {
  return live;
}

export function markLogStream(tabId: string, running: boolean): void {
  if (live.has(tabId) === running) return;
  const next = new Set(live);
  if (running) next.add(tabId);
  else next.delete(tabId);
  live = next;
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useLiveLogStreams(): ReadonlySet<string> {
  return useSyncExternalStore(subscribe, liveLogStreams, liveLogStreams);
}

/** Mark `tabId` as streaming while `running`; unmarked on unmount. No tab, no mark. */
export function useMarkLogStream(tabId: string | null, running: boolean): void {
  useEffect(() => {
    if (tabId === null) return;
    markLogStream(tabId, running);
    return () => markLogStream(tabId, false);
  }, [tabId, running]);
}
