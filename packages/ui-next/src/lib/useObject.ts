import { useCallback, useEffect, useRef, useState } from "react";
import { getObject, type K8sObject } from "@srelens/core";

export type ObjectStatus = "loading" | "ready" | "error";

export interface ObjectResource {
  object?: K8sObject;
  status: ObjectStatus;
  error?: string;
  reload(): void;
  /**
   * Read the same subject again WITHOUT going back to `loading`: what is on
   * screen stays until the new read lands and replaces it.
   *
   * For a read the reader did not ask for — the re-read after a write made
   * from this very pane (#820). `reload` there blanks the pane to a spinner
   * and unmounts everything in it, the action bar that made the write
   * included. A failed refresh leaves what is shown as it was: it is no worse
   * than not having re-read, and an error state would take a readable object
   * away over a read nobody requested. `reload` is still the retry.
   */
  refresh(): void;
}

/** The four values that identify what is being loaded, as one comparable
 *  string. Same shape `ResourceDetailView` already builds for its own reset. */
function keyFor(context: string, kind: string, namespace: string | null, name: string): string {
  return `${context}|${kind}|${namespace ?? ""}|${name}`;
}

/**
 * Loads a single object by context/kind/namespace/name. Both the peek pane
 * and the full tab drive the same hook so they can never disagree about what
 * they're showing. Follows useResource's generation-counter shape: a result
 * arriving after the target changed or the component unmounted is dropped.
 *
 * What it returns is GATED on the target the held state was fetched for
 * matching the one passed in THIS render — the same render-time gate
 * `ResourceDetailView`'s own `useLoad` applies to its panes, and for the same
 * reason. The effect below resets to "loading" on a target change, but an
 * effect runs after commit and after paint: on the very render the caller
 * switches subjects, the previous subject's object is still in this hook's
 * state, and a real browser paints one committed frame pairing the NEW
 * subject's heading with the OLD subject's body. A settled-state test cannot
 * see it (RTL flushes effects synchronously), which is exactly how it
 * survived review.
 *
 * That was not hypothetical: the resource list's peek pane is the first
 * caller that changes these four props on a MOUNTED hook — every earlier
 * caller mounted fresh per subject — and it painted a Pod's name over the
 * previously peeked Pod's Properties panel on every row-to-row click. The
 * YAML and Events panes were already safe because `useLoad` has this gate;
 * the Details pane, the default one, read straight off here and was not.
 *
 * A plain comparison computed fresh every render, not a second effect: it
 * holds on the very first commit after the target changes, and it cannot be
 * undone by a future refactor reordering effects.
 */
export function useObject(context: string, kind: string, namespace: string | null, name: string): ObjectResource {
  const targetKey = keyFor(context, kind, namespace, name);
  const [state, setState] = useState<Omit<ObjectResource, "reload" | "refresh"> & { targetKey: string }>({
    status: "loading",
    targetKey,
  });
  const gen = useRef(0);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  /** Set by `refresh`, taken by the fetch it causes — that one fetch is quiet. */
  const quietNext = useRef(false);
  const refresh = useCallback(() => {
    quietNext.current = true;
    setTick((t) => t + 1);
  }, []);

  useEffect(() => {
    const mine = ++gen.current;
    const quiet = quietNext.current;
    quietNext.current = false;
    // Quiet only over an object already shown for THIS target. Anything else —
    // a first read, a changed target, a pane sitting on an error — is a read
    // the reader is waiting on, and says so.
    const keeps = (prev: { status: string; targetKey: string }) =>
      quiet && prev.status === "ready" && prev.targetKey === targetKey;
    setState((prev) => (keeps(prev) ? prev : { status: "loading", targetKey }));
    getObject(context, kind, namespace, name).then(
      (result) => {
        if (gen.current !== mine) return;
        if (result.error) {
          const error = result.error;
          setState((prev) => (keeps(prev) ? prev : { status: "error", error, targetKey }));
          return;
        }
        setState({ status: "ready", object: result.object, targetKey });
      },
      (e: unknown) => {
        if (gen.current !== mine) return;
        const error = e instanceof Error ? e.message : String(e);
        setState((prev) => (keeps(prev) ? prev : { status: "error", error, targetKey }));
      },
    );
    return () => { if (gen.current === mine) gen.current++; };
    // `targetKey` is derived from the four values already listed, so it never
    // changes without one of them changing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context, kind, namespace, name, tick]);

  // The gate itself. `reload` is handed back either way: it is stable, and a
  // caller must be able to retry the target it is asking about right now.
  const { targetKey: fetchedFor, ...current } = state;
  return fetchedFor === targetKey ? { ...current, reload, refresh } : { status: "loading", reload, refresh };
}
