import { useCallback, useEffect, useRef, useState } from "react";
import { getObject, type K8sObject } from "@srelens/core";

export type ObjectStatus = "loading" | "ready" | "error";

export interface ObjectResource {
  object?: K8sObject;
  status: ObjectStatus;
  error?: string;
  reload(): void;
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
  const [state, setState] = useState<Omit<ObjectResource, "reload"> & { targetKey: string }>({
    status: "loading",
    targetKey,
  });
  const gen = useRef(0);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);

  useEffect(() => {
    const mine = ++gen.current;
    setState({ status: "loading", targetKey });
    getObject(context, kind, namespace, name).then(
      (result) => {
        if (gen.current !== mine) return;
        if (result.error) {
          setState({ status: "error", error: result.error, targetKey });
          return;
        }
        setState({ status: "ready", object: result.object, targetKey });
      },
      (e: unknown) => {
        if (gen.current !== mine) return;
        setState({ status: "error", error: e instanceof Error ? e.message : String(e), targetKey });
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
  return fetchedFor === targetKey ? { ...current, reload } : { status: "loading", reload };
}
