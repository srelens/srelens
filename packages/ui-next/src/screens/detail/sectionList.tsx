import { useEffect, useRef, useState } from "react";
import { FailureLine } from "../../lib/errorCopy";

/**
 * ONE ANSWER FOR "A BLOCK'S OWN LIST WAS REFUSED", for every detail block that
 * fetches a list of its own.
 *
 * Four blocks did, and all four had written the same three lines and lost the
 * same thing in them: `if (out.error) { setState({ status: "error" }) }` and
 * then `if (state.status === "error") return null`. The error STRING was
 * dropped on the floor — the state carried no room for it — and the block
 * disappeared. So a Deployment under RBAC that allows `get deployments` but not
 * `list pods` showed no Pods block at all, and the reader could not tell "this
 * Deployment has no pods" from "srelens was refused, or timed out", with
 * nothing on screen to retry.
 *
 * The `// a missing list shouldn't break the panel` comments those sites
 * carried were right about the thing they were defending: a sub-read that fails
 * must not replace the whole pane, and the surrounding blocks are real. They
 * were not a licence to discard the REASON. The branch's own bar is the
 * opposite everywhere it is stated out loud: `Overview.tsx` puts the reason on
 * the row that could not answer and says "Some kinds could not be checked"
 * rather than counting them as zero, and `Logs.tsx` keeps a per-target failure
 * precisely so "an absent container's lines look exactly like a quiet
 * container's" cannot happen.
 *
 * **Why here and not beside `useLoad` in `detailData.tsx`, which is where it
 * belongs.** `detailData.tsx` imports every body in this directory — it is the
 * table of per-kind bodies — so a body importing back from it is a cycle. This
 * module is what "beside `useLoad`" can actually be: no local imports but the
 * error vocabulary, so every body can reach it.
 *
 * It is deliberately NOT `useLoad`. That hook answers a different question — a
 * PANE's data, gated on the four-part `target` the held data was fetched for,
 * so a subject change can never paint one subject's heading over another's
 * data. A block inside a pane is already downstream of that gate; what it needs
 * is the plain generation guard below and a place to keep the reason.
 */
export interface SectionListState<T> {
  /**
   * `idle` is not a failure and not a "before": it is a block that was never
   * asked for on this kind at all — a StatefulSet has no revision history, so
   * `useDeployRevisions` fetches nothing for one. Such a block draws nothing,
   * which is the one case where drawing nothing is still honest.
   */
  status: "idle" | "loading" | "ready" | "error";
  data?: T;
  /** The refusal, raw, as the backend gave it — worded by `describeError` at
   *  the surface, per this package's error rule. */
  error?: string;
}

/**
 * A detail block's own list.
 *
 * `enabled` is "does this kind ask for this list at all" (see `idle` above),
 * not "is the object ready". `deps` is the list's identity: the effect re-runs
 * and the held data is dropped whenever any of them changes, so a block can
 * never go on showing the previous subject's list. `load` normalises to
 * core's own `{ data?, error? }` shape, which is what every `list*` in core
 * already returns, and a REJECTION is read as well as a returned `error` —
 * core's readers catch today, and a hook that handled only one of the two
 * would be one refactor away from the silence this exists to end.
 */
export function useSectionList<T>(
  enabled: boolean,
  deps: readonly unknown[],
  load: () => Promise<{ data?: T; error?: string }>,
): SectionListState<T> {
  const [state, setState] = useState<SectionListState<T>>({ status: enabled ? "loading" : "idle" });
  const gen = useRef(0);

  useEffect(() => {
    const mine = ++gen.current;
    if (!enabled) {
      setState({ status: "idle" });
      return;
    }
    setState({ status: "loading" });
    load().then(
      (result) => {
        if (gen.current !== mine) return;
        if (result.error) {
          setState({ status: "error", error: result.error });
          return;
        }
        setState({ status: "ready", data: result.data });
      },
      (e: unknown) => {
        if (gen.current !== mine) return;
        setState({ status: "error", error: e instanceof Error ? e.message : String(e) });
      },
    );
    return () => {
      if (gen.current === mine) gen.current++;
    };
    // `load` is a fresh closure every render and deliberately not a dependency
    // — `deps` is the identity of what it fetches, the same call `useLoad`
    // makes for the same reason.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, ...deps]);

  return state;
}

/**
 * What a block says when its own list could not be read, in the block, under
 * its heading.
 *
 * A LINE and not an `ErrorState` card: the heading above it and the blocks
 * either side of it are real, and the `.section + .section` adjacency that
 * draws every hairline between them is exactly what the old `return null` was
 * protecting. Through `FailureLine`, so the sentence is `describeError`'s — "The
 * cluster rejected your credentials", not an `ApiError: Unauthorized (Status {
 * metadata: Some(ListMeta { … })` struct — with the original folded away one
 * click behind it.
 */
export function SectionFailure({ error }: { error: unknown }) {
  return <FailureLine error={error} className="text-[0.8125rem] text-sev" />;
}
