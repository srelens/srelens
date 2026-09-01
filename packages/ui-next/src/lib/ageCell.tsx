import { useSyncExternalStore } from "react";
import { ageFromTimestamp } from "@srelens/core";

/**
 * A live AGE cell for the resource lists (#405).
 *
 * The backend renders each summary's `age` ONCE, against the clock at the
 * moment the summary is built — and a summary is rebuilt only when a watch
 * event arrives for that object. An object nothing is changing therefore keeps
 * the age it was born with: a Secret created while its list is open reads `0s`
 * for as long as the list stays open, while the same Secret's detail panel
 * reads `3m ago`, because the detail panel derives from the timestamp.
 *
 * No backend fix is possible: there is no event on which to re-render the
 * string. The age has to be derived where a clock is ticking, which is here.
 *
 * ONE interval for the whole app, not one per row. A node with 110 pods would
 * otherwise hold 110 timers to show the same second; every cell subscribes to
 * a single module-level clock instead, the way `columnPrefs` shares its store.
 * The interval only runs while at least one cell is mounted.
 */
const TICK_MS = 1_000;

let now = Date.now();
let timer: ReturnType<typeof setInterval> | null = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  // The clock stops when the last cell unmounts, so `now` is as stale as the
  // gap since — leave a resource list for ten minutes and the first paint on
  // return would otherwise show a ten-minute-old age until the next tick.
  // Safe here and not in `getSnapshot`, which React calls during render and
  // which must return a stable value.
  if (listeners.size === 0) now = Date.now();
  listeners.add(listener);
  if (timer === null) {
    timer = setInterval(() => {
      now = Date.now();
      for (const l of listeners) l();
    }, TICK_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  };
}

// A cached value, never `Date.now()` inline: `useSyncExternalStore` compares
// snapshots by identity and would re-render forever on a fresh number.
const getSnapshot = () => now;

/** The shared ticking clock, for anything else that must re-derive on time. */
export function useNow(): number {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

/**
 * `created` is the object's `creationTimestamp`; `age` is the backend's
 * pre-rendered string, used only for a kind whose summary does not carry a
 * timestamp yet.
 *
 * A row with neither renders an em dash rather than a number — a kind that
 * cannot be timed must show nothing, never something wrong.
 */
export function AgeCell({ created, age }: { created?: string | null; age?: string }) {
  const tick = useNow();
  if (created) return <>{ageFromTimestamp(created, tick)}</>;
  return <>{age || "—"}</>;
}
