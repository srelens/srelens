import { useEffect, useState } from "react";
import { Button } from "./Button";
import { cx } from "./cx";

const COPIED_MS = 1400;

export interface CopyButtonProps {
  /** What lands on the clipboard. */
  text: string;
  /**
   * What the control is FOR — "Copy the answer", "Copy the command".
   *
   * Used as the accessible name ONLY in `iconOnly` form. A label alongside
   * visible text overrides that text, which silenced the "Copied"
   * confirmation for anyone listening rather than looking.
   */
  label: string;
  /** Icon only, for a control sitting beside content that is already labelled
   *  — a transcript turn. The accessible name is still `label`. */
  iconOnly?: boolean;
  className?: string;
}

function CheckGlyph() {
  return (
    // `copy-command-check` kept, not renamed: it carries a real rule in the
    // components layer (the ok tone), and the name is about a copy
    // confirmation rather than about `CopyCommand` in particular.
    <svg
      className="copy-command-check"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path d="m5 13 4 4L19 7" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function CopyGlyph() {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <rect x="9" y="9" width="11" height="11" rx="2" stroke="currentColor" strokeWidth="2" />
      <path d="M5 15V5a2 2 0 0 1 2-2h8" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

/**
 * Put something on the clipboard, and say so briefly.
 *
 * Extracted from {@link CopyCommand}, which had this inline. A second copy of
 * the clipboard dance — the `copied` flag, the timer that clears it, and the
 * silence when there is no clipboard at all — is a second place to get the
 * last of those wrong.
 *
 * **A failed copy says nothing.** `navigator.clipboard` is unavailable on a
 * non-secure origin and can be refused outright, and "Copied" over an empty
 * clipboard is the only outcome here that actually misleads.
 */
export function CopyButton({ text, label, iconOnly = false, className }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), COPIED_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch {
      // Nothing to recover and nothing to say: see the note above.
    }
  }

  return (
    <>
      <Button
        variant="ghost"
        size="xs"
        // Only when there is no text to be the name. With a label AND visible
        // text, the label wins — so "Copied" stopped being announced at all,
        // and the state change became invisible to a screen reader.
        //
        // The name STAYS the action, either way. A control that renames itself
        // to its own outcome mid-interaction is the other half of that same
        // defect.
        aria-label={iconOnly ? label : undefined}
        className={cx(className)}
        onClick={() => void copy()}
      >
        {copied ? <CheckGlyph /> : <CopyGlyph />}
        {!iconOnly && (copied ? "Copied" : "Copy")}
      </Button>
      {/*
        The icon-only form has no visible word to change, and both glyphs are
        `aria-hidden` — so a successful copy was something only a sighted
        reader learned about. A live region says it instead, leaving the
        button's own name alone.

        A SIBLING, not a child: content inside a button contributes to its
        accessible name, and `.sr-only` is absolutely positioned so it costs no
        layout. Only in the icon-only case — where there IS a visible word, it
        changes to "Copied" already, and a second announcement would be two.
      */}
      {iconOnly && (
        <span role="status" aria-live="polite" className="sr-only">
          {copied ? "Copied" : ""}
        </span>
      )}
    </>
  );
}
