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
 * The reset is keyed on a counter rather than on the state alone, and that is
 * load-bearing: clicking a second time while "Copied" is still up sets the same
 * state value, React bails out of an identical update, the effect never re-runs
 * and the original timer keeps running — so the confirmation for the second
 * click would vanish early, on the first click's schedule. The counter makes
 * every copy a distinct value, so the window restarts.
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
  // `seq` distinguishes two copies that landed on the same state — see above.
  const [result, setResult] = useState<{ state: CopyState; seq: number }>({ state: "idle", seq: 0 });

  useEffect(() => {
    if (result.state === "idle") return;
    const timer = setTimeout(() => setResult((r) => ({ state: "idle", seq: r.seq })), ms);
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
    setResult((r) => ({ state, seq: r.seq + 1 }));
  }

  return { state: result.state, run };
}
