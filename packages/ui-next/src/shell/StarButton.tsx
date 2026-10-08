import { useEffect, useState } from "react";
import { describeError, openExternal } from "@srelens/core";
import { Button } from "@srelens/ui-kit";
import { Icons } from "../lib/icons";
import {
  REPO_URL,
  answerNudge,
  formatStarCount,
  nudgeDue,
  useStarState,
} from "../lib/starOnGitHub";
import { useWorkspaceSealed } from "./LockGate";

/** Why a star is worth the click, in the words both the tooltip and the nudge use. */
const WHY = "it helps other people find the project";

/**
 * The top bar's way to the srelens repository, asking for a star (#850).
 *
 * A button that is always there, and a nudge that is there once. The button is
 * quiet: a glyph, one word and a count, the same weight as the controls beside
 * it. The nudge is the only part that asks, and it asks a single time, after
 * the app has been used enough for the answer to mean something — see
 * `nudgeDue`. Either of its actions, or Escape, answers it for good.
 *
 * Counting the launch and fetching the count are NOT done here: they are the
 * window's to start (`Window`), once, so that drawing this button in a test or
 * a gallery never reaches the network.
 *
 * Nothing is asked while the workspace is sealed. The button stays — it leads
 * out of the app, not into the workspace — but a callout over a lock screen is
 * one more thing between the reader and unlocking.
 *
 * The nudge is not a dialog: it does not take focus, trap it, or dim anything.
 * It is a labelled group beside the button, reachable in the tab order right
 * after it.
 */
export function StarButton() {
  const star = useStarState();
  const sealed = useWorkspaceSealed();
  // Read once: a nudge that was not due when the bar was drawn does not appear
  // an hour later in the middle of something.
  const [mountedAt] = useState(() => Date.now());
  const [failure, setFailure] = useState<string | null>(null);
  const asking = !sealed && nudgeDue(star, mountedAt);

  useEffect(() => {
    if (!asking) return undefined;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") answerNudge();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [asking]);

  if (!star.show) return null;

  function open() {
    setFailure(null);
    // Answered by going, whichever of the two buttons took the reader there.
    if (asking) answerNudge();
    openExternal(REPO_URL).catch((error: unknown) => {
      // Said beside the button: the reader pressed it and nothing opened.
      setFailure(describeError(error).title);
    });
  }

  return (
    <span className="relative inline-flex items-center" data-slot="star">
      <button
        type="button"
        className="btn btn-ghost"
        data-size="xs"
        aria-label="Star srelens on GitHub"
        title={failure ?? `Star srelens on GitHub — ${WHY}`}
        onClick={open}
      >
        <Icons.star size={12} aria-hidden="true" />
        {/* The word and the figure go first when the window is narrow; the
            glyph and the name stay, so the button never crowds the title. */}
        <span className="max-[860px]:hidden">Star</span>
        {star.count !== null && (
          <span data-slot="star-count" className="tabular-nums text-muted max-[860px]:hidden">
            {formatStarCount(star.count)}
          </span>
        )}
      </button>
      {failure !== null && (
        <span role="alert" className="sr-only">
          Could not open GitHub: {failure}
        </span>
      )}
      {asking && (
        <div
          role="group"
          aria-label="Enjoying srelens?"
          // `.popover` places itself with `position: fixed`; this one hangs
          // off the button instead, so it moves with the bar.
          className="popover flex w-[250px] flex-col gap-2 p-3 text-[0.8125rem]"
          style={{ position: "absolute", top: "calc(100% + 6px)", right: 0 }}
        >
          <p className="font-semibold text-ink">Enjoying srelens?</p>
          <p className="text-muted">A star on GitHub costs a click, and {WHY}.</p>
          <div className="flex justify-end gap-1.5">
            <Button variant="secondary" size="sm" onClick={answerNudge}>
              Not now
            </Button>
            <Button variant="primary" size="sm" onClick={open}>
              Star on GitHub
            </Button>
          </div>
        </div>
      )}
    </span>
  );
}
