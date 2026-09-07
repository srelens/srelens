import { cx } from "./cx";
import { filled } from "./slot";

export interface RawErrorProps {
  /**
   * The message as the backend actually sent it — `describeError`'s `raw`.
   * Empty renders nothing, so a caller can pass whatever it has.
   */
  text: string;
  /** The word on the disclosure. */
  label?: string;
  className?: string;
}

/**
 * The original error, folded away behind a word.
 *
 * A classified failure is shown to the reader as a sentence they can act on —
 * "The cluster rejected your credentials" — and the string the cluster
 * actually sent is 300 characters of `Status { metadata: Some(ListMeta { … })`.
 * Both are needed by different people at different moments, and neither can
 * stand in for the other: the sentence is useless in a bug report and the
 * struct is useless on screen.
 *
 * **A disclosure, and specifically not a `title` attribute.** The kit removed
 * value-carrying `title`s from `PairList` and `KV` after a `kubectl
 * apply`-managed Secret leaked through one, and there is no prop to put them
 * back (4132850, 255bcc7). The rule those two settled is that a second,
 * unredacted copy of a string must never sit in the markup where nothing on
 * screen says it is there. A `details` is the opposite of that in every
 * respect: it is one copy, the reader asks for it, it is a Tab stop, it
 * announces itself as expandable, and the text can be selected and pasted —
 * which is the entire reason anyone wants it.
 *
 * A `Tooltip` was the other candidate, and it is what `ClusterRail` uses for
 * a failure with 46px to say it in. It loses on this surface for the reason it
 * wins on that one: a hint is short, transient and hard to copy from, and this
 * is a long string whose only use is being copied.
 *
 * Closed by default, and it stays closed on its own — no state, no memory. The
 * reader who wants it opens it; every other reader sees one quiet word.
 */
export function RawError({ text, label = "Original error", className }: RawErrorProps) {
  if (!filled(text)) return null;
  return (
    <details data-slot="raw" className={cx("text-left", className)}>
      {/* `list-none` because the marker's own triangle is drawn by the UA in a
          colour and size nothing here controls; the caret is the caller's
          type, which follows the theme. */}
      <summary className="cursor-pointer list-none text-[0.75rem] text-faint underline decoration-dotted underline-offset-2">
        {label}
      </summary>
      {/* `pre` rather than a paragraph: this is machine output, and the line
          breaks a Rust struct or a stack of causes arrives with are part of
          reading it. `whitespace-pre-wrap` keeps them without also letting a
          400-character single line push the surface sideways, and the cap
          means a long chain scrolls inside the disclosure rather than
          swallowing the panel it opened in. */}
      <pre className="scroll mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-canvas-deep p-2 font-mono text-[0.6875rem] leading-relaxed text-faint">
        {text}
      </pre>
    </details>
  );
}
