import { useEffect, useRef, useState } from "react";
import { describeError, openExternal } from "@srelens/core";
import { Button } from "@srelens/ui-kit";
import { Icons } from "../lib/icons";
import {
  REPO_URL,
  answerNudge,
  visitedRepository,
  formatStarCount,
  nudgeDue,
  useStarState,
} from "../lib/starOnGitHub";
import { useWorkspaceSealed } from "./LockGate";

/** Why a star is worth the click, in the words both the tooltip and the nudge use. */
const WHY = "it helps other people find the project";

/**
 * GitHub's mark, drawn inline: the kit's icon set carries no brand marks, and
 * this is the one place a brand is the message. Decorative — the button's name
 * already says GitHub.
 */
function GitHubMark() {
  return (
    <svg className="gh-star-mark" width="13" height="13" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <path d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z" />
    </svg>
  );
}

/**
 * The top bar's way to the srelens repository, asking for a star (#850).
 *
 * A button that is always there, and a nudge that is there once. The button is
 * the one control in the bar meant to be noticed: a pill in GitHub's own ink
 * with GitHub's mark and a gold star, where everything beside it is a grey
 * glyph (`.gh-star` in `next.css`). The star fills on hover, and the pill
 * breathes while the nudge is on screen. The nudge is the only part that asks, and it asks a single time, after
 * the app has been used enough for the answer to mean something — see
 * `nudgeDue`. Either of its actions, or Escape, answers it for good.
 *
 * Counting the launch and fetching the count are NOT done here: they are the
 * window's to start (`Window`), once, so that drawing this button in a test or
 * a gallery never reaches the network.
 *
 * Neither is drawn while the workspace is sealed. A locked window offers what
 * unlocks it and what makes it legible, and nothing else that can be pressed —
 * the rule the status bar and the Settings gear already follow. The question
 * is not used up by waiting behind the lock.
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
  const pill = useRef<HTMLButtonElement>(null);
  const callout = useRef<HTMLDivElement>(null);
  const asking = !sealed && nudgeDue(star, mountedAt);

  /**
   * Answer the question. If the keyboard was inside the callout, it goes to
   * the pill the callout hung from: the callout is about to leave the page,
   * and focus left on a removed button lands on the document body.
   */
  function answer(visited = false) {
    const within = callout.current?.contains(document.activeElement) === true;
    // Having gone to the repository answers it too, and is when the count is
    // about to change: see `visitedRepository`.
    if (visited) visitedRepository();
    else answerNudge();
    if (within) pill.current?.focus();
  }

  useEffect(() => {
    if (!asking) return undefined;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // As `answer` below: the keyboard goes to the pill if it was inside.
      const within = callout.current?.contains(document.activeElement) === true;
      answerNudge();
      if (within) pill.current?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [asking]);

  if (!star.show || sealed) return null;

  function open() {
    setFailure(null);
    // Answered by going, whether or not the question had been put yet — and
    // only by going: a browser that did not open took the reader nowhere, and
    // must not cost them the one time they are asked.
    openExternal(REPO_URL).then(() => answer(true), (error: unknown) => {
      // Said beside the button: the reader pressed it and nothing opened.
      setFailure(describeError(error).title);
    });
  }

  return (
    <span className="relative inline-flex items-center" data-slot="star">
      <button
        ref={pill}
        type="button"
        className="gh-star"
        data-asking={asking || undefined}
        aria-label="Star srelens on GitHub"
        title={failure ?? `Star srelens on GitHub — ${WHY}`}
        onClick={open}
      >
        <GitHubMark />
        <span className="gh-star-label">Star</span>
        {/* GitHub's own star button, in miniature: the word, a rule, the star
            and the figure. With no figure yet the star stands alone. */}
        <span className="gh-star-count">
          <Icons.star size={12} aria-hidden="true" className="gh-star-glyph" />
          {star.count !== null && <span data-slot="star-count">{formatStarCount(star.count)}</span>}
        </span>
      </button>
      {failure !== null && (
        <span role="alert" className="sr-only">
          Could not open GitHub: {failure}
        </span>
      )}
      {asking && (
        <div
          ref={callout}
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
            <Button variant="secondary" size="sm" onClick={() => answer()}>
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
