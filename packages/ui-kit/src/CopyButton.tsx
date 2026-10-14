import { Button } from "./Button";
import { cx } from "./cx";
import { useCopied } from "./useCopied";

export interface CopyButtonProps {
  /**
   * What lands on the clipboard — or a function asked for it at the click.
   *
   * A string is right when React holds the thing being copied: a command, a
   * transcript, an address. It is wrong when the thing lives somewhere React
   * is not told about, and `CodeEditor` is exactly that — CodeMirror owns its
   * document, `onChange` is optional, and a control reading a prop would hand
   * over the text from mount while the reader looks at what they have typed.
   * Hence the function form, evaluated when the button is pressed rather than
   * when it is rendered. (#656 review)
   */
  text: string | (() => string);
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

/** What the control says at each outcome. `idle` is the action, not a state. */
const WORD = { idle: "Copy", copied: "Copied", failed: "Copy failed" } as const;

/**
 * Put something on the clipboard, and say so briefly.
 *
 * Extracted from {@link CopyCommand}, which had this inline. A second copy of
 * the clipboard dance — the outcome, the timer that clears it, and what a
 * refusal is allowed to claim — is a second place to get the last of those
 * wrong.
 *
 * **A failed copy never says "Copied".** `navigator.clipboard` is unavailable
 * on a non-secure origin and can be refused outright, and a confirmation over
 * an empty clipboard is the outcome here that actually misleads.
 *
 * **It does not say nothing, either.** It used to: the first draft swallowed
 * the refusal on the grounds that silence at least does not lie. It does not
 * lie and it does not help — the reader walks away believing the manifest is
 * on their clipboard, which is the "copy that reported into a void" that
 * {@link useCopied} was extracted to stop happening again. So the word becomes
 * "Copy failed" and comes back after the same 1.4s, and the state is
 * `useCopied`'s rather than a second timer kept here. (#656 review)
 */
export function CopyButton({ text, label, iconOnly = false, className }: CopyButtonProps) {
  const { state, run } = useCopied();
  const copied = state === "copied";

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
        // The only place a sighted reader can be told a copy failed on the
        // icon-only form, where there is no word to change.
        title={state === "failed" ? WORD.failed : undefined}
        className={cx(className)}
        onClick={() => void run(() => navigator.clipboard.writeText(typeof text === "function" ? text() : text))}
      >
        {copied ? <CheckGlyph /> : <CopyGlyph />}
        {!iconOnly && WORD[state]}
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
          {state === "idle" ? "" : WORD[state]}
        </span>
      )}
    </>
  );
}
