import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { EmptyState } from "./EmptyState";
import { Eyebrow } from "./Eyebrow";
import { Spinner } from "./Spinner";
import { cx } from "./cx";
import { filled } from "./slot";
import { toneColor, toneWash } from "./tone";

/**
 * ⌘ on a Mac, Ctrl everywhere else — for the hint only. The handler answers to
 * both, because a user on either keyboard may be driving either machine.
 */
const MOD =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform ?? "")
    ? "⌘K"
    : "Ctrl K";

export interface ConsoleDockProps {
  /** Whether the output is showing. The prompt is always there. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The prompt's text. Controlled, so the caller can rewrite it as it is typed. */
  value: string;
  onValueChange: (value: string) => void;
  /** Called with the trimmed query. Never called empty, and never while busy. */
  onSubmit: (value: string) => void;
  /** The eyebrow at the top left — what the console is doing (e.g. "Agent"). */
  mode?: ReactNode;
  /** What it is pointed at (e.g. "prod-eu / checkout-api"). */
  context?: ReactNode;
  /** A quiet figure beside the context (e.g. "3 exchanges"). */
  status?: ReactNode;
  placeholder?: string;
  /** A query is in flight: send is withdrawn and Enter does nothing. */
  busy?: boolean;
  /** Shows the Clear control; the caller does the clearing. */
  onClear?: () => void;
  /** The output — a transcript, a command list, suggestions. */
  children?: ReactNode;
  /** What to say when there is no output yet. */
  emptyLabel?: ReactNode;
  /** Names the dock, its prompt and its output. */
  label?: string;
  /** The accelerator, as printed beside the prompt. */
  shortcutHint?: ReactNode;
  /**
   * Announce what arrives in the output. True for a transcript, which is the
   * console's normal content; false for a body that is a list filtered as the
   * user types, where every keystroke would otherwise be read out.
   */
  live?: boolean;
  className?: string;
}

/**
 * The console docked along the bottom of the window: a prompt that is always
 * there, and an output panel above it that opens when there is something to
 * read. A panel, not a floating overlay — it takes its own strip of the window
 * and the app resizes around it.
 *
 * The mock's version is this chrome wrapped around an entire agent: it reads
 * the console context, the open tab and a canned run out of module state,
 * fakes a streaming reply on a chain of timers, decides which commands match
 * the query, routes on the ones that do, and opens a confirmation gate for the
 * destructive ones. None of that is the kit's — it is app state, and the
 * provider it hangs off is going to `ui-next` rather than here. What is left is
 * a real component and the whole of the mock's chrome: collapse, the prompt
 * with its accelerator, the send control, the busy state and the output region.
 * The transcript, the command list and the suggestions are the caller's, and
 * arrive as children, which is the same seam `CodeEditor` drew when Kubernetes
 * schema resolution became an injected completion source. (#320)
 *
 * Being controlled is what makes that work: the query, the open state and the
 * output all belong to the caller, so a submitted query is theirs to clear or
 * keep. The dock reports; it does not remember.
 *
 * One toggle, not the mock's two. It renders a chevron in the header and
 * another beside the prompt, both collapsing the same panel — two tab stops
 * with the same name, one of them always on screen anyway.
 */
export function ConsoleDock({
  open,
  onOpenChange,
  value,
  onValueChange,
  onSubmit,
  mode,
  context,
  status,
  placeholder,
  busy = false,
  onClear,
  children,
  emptyLabel = "Nothing yet",
  label = "Console",
  shortcutHint = MOD,
  live = true,
  className,
}: ConsoleDockProps) {
  const bodyId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  // Seeded with the current state so a dock that mounts open leaves focus
  // where it was. Only the transition from closed to open is an act of the
  // user's that focus should follow.
  const wasOpen = useRef(open);

  const ready = value.trim().length > 0 && !busy;

  function submit() {
    // Trimmed here so every caller does not have to, and blocked while busy so
    // Enter held down does not queue three copies of the same question.
    const query = value.trim();
    if (!query || busy) return;
    onSubmit(query);
  }

  useEffect(() => {
    if (open && !wasOpen.current) inputRef.current?.focus();
    wasOpen.current = open;
  }, [open]);

  // No accelerator is installed here. The hint beside the prompt says what the
  // key is, and the app binds it — a window-level shortcut belongs to whatever
  // owns the window, not to a component that might be mounted twice. Two docks
  // on one screen both answered the same ⌘K, and a kit component that grabs a
  // global key also fights whatever else the app has bound to it. The same line
  // Inspector's ⌘⏎ sits behind. (#320)

  // Keep the newest output in view. `scrollTop` rather than `scrollTo`, which
  // is not implemented on elements in jsdom — this would throw under test for
  // no gain.
  useEffect(() => {
    const body = bodyRef.current;
    if (body) body.scrollTop = body.scrollHeight;
  }, [children, open]);

  function onPromptKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      submit();
      return;
    }
    // Escape backs out of the panel, and only when there is one to back out of
    // — otherwise the console swallows a key something behind it wanted.
    if (event.key === "Escape" && open) {
      event.preventDefault();
      onOpenChange(false);
      inputRef.current?.blur();
    }
  }

  return (
    <section aria-label={label} className={cx("console-dock", className)}>
      {open && (
        <>
          <div
            className="rule-b flex items-center justify-between gap-3 px-2.5 py-1"
            style={{ background: "var(--surface-sunk)" }}
          >
            <div className="flex min-w-0 items-center gap-2">
              {filled(mode) && <Eyebrow>{mode}</Eyebrow>}
              {filled(context) && (
                <span
                  className="path truncate rounded px-1 py-px"
                  style={{ background: toneWash("accent"), color: toneColor("accent") }}
                >
                  {context}
                </span>
              )}
              {filled(status) && <Eyebrow className="text-[0.5625rem]">{status}</Eyebrow>}
            </div>
            {onClear && (
              <button
                type="button"
                className="icon-btn shrink-0"
                aria-label={`Clear ${label.toLowerCase()}`}
                onClick={onClear}
              >
                {/* The class rather than `Eyebrow`, which renders a div: a
                    button holds phrasing content, and a block inside one is
                    markup no browser is obliged to lay out sensibly. */}
                <span className="eyebrow text-[0.5625rem]">clear</span>
              </button>
            )}
          </div>

          <div
            id={bodyId}
            ref={bodyRef}
            className="scroll max-h-[42vh] min-h-[132px] px-3 py-2.5"
            role="log"
            aria-live={live ? "polite" : "off"}
            aria-label={`${label} output`}
          >
            {filled(children) ? children : <EmptyState title={emptyLabel} />}
          </div>
        </>
      )}

      <div className="flex h-[34px] items-center gap-2 px-2.5">
        <button
          type="button"
          className="agent-mark !h-[19px] !w-[19px] !rounded-[5px]"
          aria-label={open ? `Collapse ${label.toLowerCase()}` : `Expand ${label.toLowerCase()}`}
          aria-expanded={open}
          // Only while the panel exists: an `aria-controls` pointing at nothing
          // is a promise to assistive technology that cannot be kept.
          aria-controls={open ? bodyId : undefined}
          onClick={() => onOpenChange(!open)}
        >
          {/* Inline rather than an icon-set import: the kit takes no dependency
              on lucide, and these are the only glyphs it needs. */}
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path
              d={open ? "m6 9 6 6 6-6" : "m6 15 6-6 6 6"}
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>

        <input
          ref={inputRef}
          className="console-input !text-[0.8125rem]"
          // A placeholder is not a label: it is gone the moment anything is
          // typed, and the mock's input has nothing else to go on.
          aria-label={`${label} prompt`}
          value={value}
          placeholder={placeholder}
          onFocus={() => {
            if (!open) onOpenChange(true);
          }}
          onChange={(e) => onValueChange(e.target.value)}
          onKeyDown={onPromptKeyDown}
        />

        {busy ? (
          <span className="flex shrink-0 items-center gap-1.5">
            <Spinner label="Working" className="size-3" style={{ color: toneColor("accent") }} />
            <Eyebrow className="text-[0.5625rem]">working</Eyebrow>
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
              onClick={submit}
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
    </section>
  );
}
