import { useEffect, useState } from "react";
import { Button } from "./Button";
import { cx } from "./cx";

/** How long the button stays flipped after a copy, from the design's §12. */
const COPIED_MS = 1400;

export interface CopyCommandProps {
  /** The command, shown in full and copied verbatim. */
  command: string;
  className?: string;
}

/* Inline rather than an icon-set import: the kit takes no dependency on
   lucide, and this is the only glyph it needs — the same reason
   `KubectlPreview` inlines its own. */
function CheckGlyph() {
  return (
    <svg
      className="copy-command-check"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="m5 13 4 4 10-10"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/**
 * A command the reader can take away: the line itself, and a control that puts
 * it on the clipboard.
 *
 * The design's §A.9 names this, and six screens ask for it — §12's "Fetch it
 * yourself" rail, §13, §20, §21, §22 and §23. It exists as a component because
 * the first call site had already grown a code face, a clipboard write, a
 * try/catch and a two-state label, and the second would have copied all four.
 *
 * **This is NOT {@link KubectlPreview}, and the two are not one component with
 * a flag.** That one prints "Equivalent kubectl:" ahead of the command, which
 * is exactly right where it lives — inside a confirm dialog, beside an action
 * the app is about to perform, saying *this is what we are doing on your
 * behalf*. Here there is no action to be equivalent to: the command IS the
 * content, the reader is being handed something to run themselves, and
 * announcing it as an equivalent to nothing reads as a mistake. The label is
 * bare JSX there with no prop and no slot, and two suites pin the string, so
 * this is a second component rather than a widened first one.
 *
 * **THE COMMAND WRAPS. IT DOES NOT TRUNCATE, AND IT CARRIES NO `title`.** The
 * mock draws it clipped, and that is a mock drawn at one width. A command you
 * cannot finish reading is not one you can retype, and this lands in a 264px
 * rail where clipping starts early. The usual repair — truncate and put the
 * whole string in a `title` — is the disclosure hole removed from `PairList`
 * and from `KV`: a second, unredacted copy sitting in the DOM. There is no prop
 * to put either back. `KubectlPreview` reached the same conclusion for itself.
 *
 * The clipboard write lives here rather than behind an `onCopy` prop, which is
 * how `KubectlPreview` does it. That prop is what made the first call site
 * write `navigator.clipboard.writeText` in a try/catch of its own, and six of
 * those is six chances to forget the catch — a non-secure origin has no
 * `navigator.clipboard` at all, and an uncaught rejection there is a broken
 * button and a console error. It is not a breach of the kit's no-app-state
 * rule: the clipboard is a browser API, like the `ResizeObserver` in
 * `ResizeHandle`, not this application's data and not storage the kit reads
 * back.
 *
 * The confirmation is a word AND an ok-toned check, and it reverts after 1.4s.
 * A label that never comes back reads as a control already spent; the check is
 * what makes "Copied" land as confirmation rather than as the button's new
 * name. The button's accessible name is its own visible text — no `aria-label`
 * over the top of it, which would be a second string saying the same thing.
 */
export function CopyCommand({ command, className }: CopyCommandProps) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), COPIED_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
    } catch {
      // No clipboard on a non-secure origin, and nothing to recover: the
      // command is rendered in full beside the button and can be selected.
      // Saying "Copied" when nothing was copied would be the only real harm.
    }
  }

  return (
    <div className={cx("copy-command", className)}>
      <code className="code copy-command-text">{command}</code>
      <Button variant="ghost" size="xs" onClick={() => void copy()}>
        {copied && <CheckGlyph />}
        {copied ? "Copied" : "Copy"}
      </Button>
    </div>
  );
}
