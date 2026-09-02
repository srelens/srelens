import { useEffect, useState } from "react";

/** How long a control stays flipped after a copy, from the design's §12. */
export const COPIED_MS = 1400;

/**
 * What the last copy did. `idle` is also what every control starts as, and what
 * it returns to — a confirmation that never leaves reads as a control already
 * spent rather than as an answer to the click.
 */
export type CopyState = "idle" | "copied" | "failed";

/**
 * The state behind a copy control: run a copy, say whether it worked, and take
 * it back after a moment.
 *
 * It exists because the confirmation was written once, in {@link CopyCommand},
 * and four other copy affordances did without it — the detail bar's "Copy as
 * kubectl" and the row menu's, `KubectlPreview`'s icon button, and the port
 * forward address. Copying the fifteen lines four more times is four chances to
 * get the failure rule wrong, and the failure rule is the half that matters.
 *
 * **A failed copy never says "Copied".** `run` reports `failed` when the copy
 * throws AND when it resolves `false`, so a caller whose copy helper reports
 * failure by return value rather than by rejecting is not silently treated as a
 * success. Saying "Copied" when nothing was copied is the one outcome worse
 * than saying nothing, which is what these controls did before.
 *
 * **A failure is not silent either.** `failed` is a state a control is expected
 * to draw, not a signal to swallow — the whole complaint that started this was
 * a copy that reported into a void.
 *
 * **The state is held in an object, and that wrapper is load-bearing.** Clicking
 * a second time while "Copied" is still up records the same `CopyState` value;
 * as a bare `useState<CopyState>` React bails out of the identical update, the
 * effect never re-runs, the first click's timer keeps running, and the second
 * confirmation vanishes early on the first one's schedule. A fresh object every
 * time is never equal to the last, so the effect re-runs and the window
 * restarts. Turning this back into `useState<CopyState>` reintroduces the bug;
 * "restarts the window on a rapid second copy rather than expiring early", in
 * this module's tests, is the guard that catches it.
 *
 * An earlier draft carried a `seq` counter here and credited it with that,
 * which was wrong — the object literal did the work either way, so the counter
 * could be frozen with the suite still green. It is gone rather than left as a
 * comment describing a mechanism that was not the one running. (#413 review)
 *
 * The timer is cleared by the effect's own cleanup, so a control unmounted
 * mid-confirmation — the peek closing while "Copied" is up — leaves nothing
 * behind to fire into a dead component.
 */
export function useCopied(ms: number = COPIED_MS): {
  state: CopyState;
  /** Runs a copy and records what happened. Never rejects. */
  run: (copy: () => unknown | Promise<unknown>) => Promise<void>;
} {
  // An object, not a bare `CopyState`: a new one every time is what keeps two
  // copies that landed on the same state from being folded into one update —
  // see above.
  const [result, setResult] = useState<{ state: CopyState }>({ state: "idle" });

  useEffect(() => {
    if (result.state === "idle") return;
    const timer = setTimeout(() => setResult({ state: "idle" }), ms);
    return () => clearTimeout(timer);
  }, [result, ms]);

  async function run(copy: () => unknown | Promise<unknown>): Promise<void> {
    let state: CopyState;
    try {
      // `false` is a refusal, not a value: a helper that reports failure by
      // returning rather than throwing must not be read as having worked.
      state = (await copy()) === false ? "failed" : "copied";
    } catch {
      state = "failed";
    }
    setResult({ state });
  }

  return { state: result.state, run };
}
