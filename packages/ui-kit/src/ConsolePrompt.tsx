import { forwardRef, useRef, type ClipboardEvent, type KeyboardEvent, type ReactNode } from "react";
import { Eyebrow } from "./Eyebrow";
import { Spinner } from "./Spinner";
import { cx } from "./cx";
import { filled } from "./slot";
import { toneColor } from "./tone";

export interface ConsolePromptProps {
  value: string;
  onValueChange: (value: string) => void;
  /** Enter, or the send control. Never called while `busy` or empty. */
  onSubmit: () => void;
  /** Keys the host wants first — a `/` menu's arrows, Escape. Returning true
   *  means handled, so this component leaves the key alone. */
  onKeyDown?: (e: KeyboardEvent<HTMLTextAreaElement>) => boolean;
  placeholder?: string;
  /** Names the input, since a placeholder is not a label. */
  label: string;
  busy?: boolean;
  /** Stop the turn in flight. Offered beside the working spinner, because
   *  `busy` is the only state in which it means anything. */
  onStop?: () => void;
  /** A keycap, `⌘K` — drawn only where the host actually binds one. */
  shortcutHint?: ReactNode;
  /** The dock opens itself when its input takes focus; a screen has nothing
   *  to open. */
  onFocus?: () => void;
  /**
   * What the question is about, as chips ABOVE the input — the cluster, the
   * namespace, the resource. Named `context` because that is what it is; the
   * host decides how much of it is worth saying.
   */
  context?: ReactNode;
  /** Attachments the host is holding — pasted images, as chips. */
  attachments?: ReactNode;
  /** Left of the footer: the dock's collapse chevron, the agent picker. */
  lead?: ReactNode;
  /** Right of the footer, before send. */
  trail?: ReactNode;
  /**
   * Images pasted into the input. Only called when the paste actually carries
   * one, so a text paste behaves normally.
   *
   * Absent means this host takes no attachments, and then the `+` is not drawn
   * either — a control that cannot do anything is worse than none.
   */
  onPasteImages?: (files: File[]) => void;
  /** Files chosen through the `+` control. Same handler, different door. */
  onPickImages?: (files: File[]) => void;
  /**
   * The collapsed shape: one line, no context row, no footer — §F's 34px
   * strip.
   *
   * The expanded composer is a box with a two-row input and a control row
   * beneath, which is right when a conversation is open and far too tall for a
   * dock that is meant to be out of the way. Reported as "closed view is still
   * too big".
   */
  compact?: boolean;
  className?: string;
}

/**
 * The one composer, wherever srelens asks for a question.
 *
 * A bordered box rather than a single line: context chips on top, a multi-line
 * input, and a footer of controls with send at the end. A one-line input could
 * not hold a pasted screenshot, an agent picker and a cluster all at once
 * without them fighting, which is what the dock's header and the agent
 * screen's own row were each doing differently.
 *
 * **Enter sends; Shift-Enter is a newline.** A question about a manifest or a
 * log line wants more than one line, and a textarea that submits on every
 * Enter cannot give it.
 */
export const ConsolePrompt = forwardRef<HTMLTextAreaElement, ConsolePromptProps>(function ConsolePrompt(
  {
    value,
    onValueChange,
    onSubmit,
    onKeyDown,
    placeholder,
    label,
    busy = false,
    onStop,
    shortcutHint,
    onFocus,
    context,
    attachments,
    lead,
    trail,
    onPasteImages,
    onPickImages,
    compact = false,
    className,
  },
  ref,
) {
  const ready = value.trim().length > 0 && !busy;
  const fileInput = useRef<HTMLInputElement>(null);

  function handlePaste(e: ClipboardEvent<HTMLTextAreaElement>) {
    if (!onPasteImages) return;
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith("image/"));
    if (files.length === 0) return;
    // Only when it IS an image: a text paste must still land in the input.
    e.preventDefault();
    onPasteImages(files);
  }

  if (compact) {
    return (
      <div className={cx("flex h-[34px] min-w-0 items-center gap-2 px-2.5", className)}>
        {lead}
        <textarea
          ref={ref}
          rows={1}
          className="console-input min-w-0 resize-none !py-0 leading-[34px]"
          aria-label={`${label} prompt`}
          value={value}
          placeholder={placeholder}
          onFocus={onFocus}
          onChange={(e) => onValueChange(e.target.value)}
          onPaste={handlePaste}
          onKeyDown={(e) => {
            if (onKeyDown?.(e)) return;
            if (e.key === "Enter" && !e.shiftKey && ready) {
              e.preventDefault();
              onSubmit();
            }
          }}
        />
        {busy ? (
          <span className="flex shrink-0 items-center gap-1.5">
            <Spinner label="Working" className="size-3" style={{ color: toneColor("accent") }} />
            {onStop && (
              <button type="button" className="text-btn" aria-label="Stop" onClick={onStop}>
                <span className="eyebrow whitespace-nowrap text-[0.5625rem]">stop</span>
              </button>
            )}
          </span>
        ) : (
          <>
            {filled(shortcutHint) && <span className="kbd shrink-0">{shortcutHint}</span>}
            <button
              type="button"
              className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-md transition-opacity disabled:opacity-25"
              style={{ background: toneColor("accent"), color: "var(--accent-ink)" }}
              aria-label="Send"
              disabled={!ready}
              onClick={onSubmit}
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                <path
                  d="M12 19V5m0 0-7 7m7-7 7 7"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </button>
          </>
        )}
      </div>
    );
  }

  return (
    <div className={cx("console-prompt", className)}>
      {filled(context) && <div className="flex min-w-0 flex-wrap items-center gap-1.5">{context}</div>}
      {filled(attachments) && (
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">{attachments}</div>
      )}
      <textarea
        ref={ref}
        rows={2}
        className="console-input min-w-0 resize-none"
        // A placeholder is not a label: it is gone the moment anything is
        // typed, and there is nothing else for the input to go on.
        aria-label={`${label} prompt`}
        value={value}
        placeholder={placeholder}
        onFocus={onFocus}
        onChange={(e) => onValueChange(e.target.value)}
        onPaste={handlePaste}
        onKeyDown={(e) => {
          if (onKeyDown?.(e)) return;
          // Shift-Enter is a newline, so a question can be more than one line.
          if (e.key === "Enter" && !e.shiftKey && ready) {
            e.preventDefault();
            onSubmit();
          }
        }}
      />
      <div className="flex min-w-0 items-center gap-2">
        {onPickImages && (
          <>
            <button
              type="button"
              className="text-btn"
              aria-label="Attach an image"
              onClick={() => fileInput.current?.click()}
            >
              <span className="eyebrow whitespace-nowrap text-[0.6875rem]">+</span>
            </button>
            <input
              ref={fileInput}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              // Reset after every pick, or choosing the same file twice in a
              // row fires no `change` and the second attach silently does
              // nothing.
              onChange={(e) => {
                const files = Array.from(e.target.files ?? []);
                e.target.value = "";
                if (files.length > 0) onPickImages(files);
              }}
            />
          </>
        )}
        {lead}
        <span className="flex-1" />
        {busy ? (
          <span className="flex shrink-0 items-center gap-1.5">
            <Spinner label="Working" className="size-3" style={{ color: toneColor("accent") }} />
            <Eyebrow className="text-[0.5625rem]">working</Eyebrow>
            {onStop && (
              <button type="button" className="text-btn" aria-label="Stop" onClick={onStop}>
                <span className="eyebrow whitespace-nowrap text-[0.5625rem]">stop</span>
              </button>
            )}
          </span>
        ) : (
          <>
            {trail}
            {filled(shortcutHint) && <span className="kbd shrink-0">{shortcutHint}</span>}
            <button
              type="button"
              className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-md transition-opacity disabled:opacity-25"
              style={{ background: toneColor("accent"), color: "var(--accent-ink)" }}
              aria-label="Send"
              disabled={!ready}
              onClick={onSubmit}
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                <path
                  d="M12 19V5m0 0-7 7m7-7 7 7"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </button>
          </>
        )}
      </div>
    </div>
  );
});
