import { useCallback, useEffect, useRef, useState } from "react";

export type ResourceStatus = "loading" | "ready" | "empty" | "error";
export interface Resource<T> { status: ResourceStatus; data?: T; error?: string; reload(): void }

const defaultEmpty = (v: unknown) => v == null || v === "" || (Array.isArray(v) && v.length === 0);

/**
 * One loading pattern for every screen: loading, ready, empty, error, and a
 * retry that re-invokes the loader. A result that arrives after the component
 * unmounted, or after a newer load began, is dropped — the `gen` counter is
 * what says which load is current.
 */
export function useResource<T>(load: () => Promise<T>, deps: unknown[], isEmpty: (v: T) => boolean = defaultEmpty): Resource<T> {
  const [state, setState] = useState<Omit<Resource<T>, "reload">>({ status: "loading" });
  const gen = useRef(0);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);

  useEffect(() => {
    const mine = ++gen.current;
    setState({ status: "loading" });
    load().then(
      (data) => { if (gen.current === mine) setState(isEmpty(data) ? { status: "empty", data } : { status: "ready", data }); },
      (e: unknown) => { if (gen.current === mine) setState({ status: "error", error: e instanceof Error ? e.message : String(e) }); },
    );
    return () => { if (gen.current === mine) gen.current++; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tick, ...deps]);

  return { ...state, reload };
}
