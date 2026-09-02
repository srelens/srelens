import {
  useEffect,
  useId,
  useRef,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { ConsolePrompt } from "./ConsolePrompt";
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
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad/.test(navigator.platform ?? "")
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
  /** Stop that query. Drawn beside the working spinner; see
   *  {@link ConsolePromptProps.onStop}. */
  onStop?: () => void;
  /**
   * Pixels to leave clear on the right, for a screen whose own rail runs down
   * that edge.
   *
   * The dock is window-wide chrome and sits BELOW everything, so on a screen
   * with a rail the composer ran the full width underneath it and read as
   * crossing into the sidebar. The host knows which screens have a rail and
   * how wide; this component cannot.
   *
   * The dock keeps the window's full width; the reserved strip is drawn as a
   * CONTINUATION of the rail's own column — same surface, same left rule — so
   * the sidebar reads as reaching the bottom of the window.
   *
   * Two earlier attempts got this wrong and each produced its own report. A
   * `margin-right` pulled the dock's surface and top border in and left a hole
   * in the corner ("there is still a gap"). A `padding-right` filled the
   * chrome but left the strip blank, so the sidebar still stopped short
   * ("side bar should go till bottom"). The rail lives inside the screen and
   * cannot grow past it, so the only thing that can continue that column down
   * here is this.
   */
  insetRight?: number;
  /** The host's own footer controls, beside the collapse chevron — the agent
   *  picker lives here. */
  promptLead?: ReactNode;
  /** Chips above the input saying what the question is about. */
  promptContext?: ReactNode;
  /** Attachments the host is holding, as chips. */
  attachments?: ReactNode;
  /** Images pasted into, or picked for, the prompt. Absent means this dock
   *  takes none, and the `+` is not drawn. */
  onPasteImages?: (files: File[]) => void;
  onPickImages?: (files: File[]) => void;
  /** Shows the Clear control; the caller does the clearing. */
  onClear?: () => void;
  /**
   * Shows the "open full view" control; the caller does the navigating.
   *
   * The dock is a compact view of a conversation that also has a full screen,
   * and there was no way between them — a reader had to know the left nav had
   * an Agent entry. Optional, because `ConsoleDock` is a kit component and a
   * dock with no fuller view has nothing to offer here.
   */
  onExpand?: () => void;
  /** The output — a transcript, a command list, suggestions. */
  children?: ReactNode;
  /** What to say when there is no output yet. */
  /**
   * What to say in the body when the caller has nothing to put there.
   *
   * No default, deliberately. It used to be `"Nothing yet"`, which meant an
   * unused dock drew a 132px panel with a placeholder in it — reported as
   * blank space under the prompt, and asked to go ("make the dock clean").
   * With nothing given there is no body region at all, and the dock is its
   * composer. A caller that genuinely wants an empty state asks for one.
   */
  emptyLabel?: ReactNode;
  /**
   * The dock is a composer and nothing else: no header, and a body only when
   * there is genuinely something to put in it.
   *
   * For the one mount where the SCREEN is the output — `/agent`, which draws
   * the transcript itself and carries its own `New question` — a header
   * repeating the context pill and a body holding an empty transcript are two
   * copies of what is already on screen, the second of them 132px of blank.
   * The command palette and an error still open the body, because those have
   * nowhere else to appear.
   */
  composerOnly?: boolean;
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
  onStop,
  insetRight = 0,
  promptLead,
  promptContext,
  attachments,
  onPasteImages,
  onPickImages,
  onClear,
  onExpand,
  children,
  emptyLabel,
  composerOnly,
  label = "Console",
  shortcutHint = MOD,
  live = true,
  className,
}: ConsoleDockProps) {
  const bodyId = useId();
  // A textarea now: the prompt is a multi-line composer, not a one-line input.
  const inputRef = useRef<HTMLTextAreaElement>(null);
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

  /** Returns true for a key this host has dealt with, so `ConsolePrompt`
   *  leaves it alone — Enter it handles itself. */
  function onPromptKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): boolean {
    // Escape backs out of the panel, and only when there is one to back out of
    // — otherwise the console swallows a key something behind it wanted.
    if (event.key === "Escape" && open) {
      event.preventDefault();
      onOpenChange(false);
      inputRef.current?.blur();
      return true;
    }
    return false;
  }

  return (
    <section aria-label={label} className={cx("console-dock", className)}>
      <div className="flex min-w-0">
        <div className="flex min-w-0 flex-1 flex-col">
          {open && !composerOnly && (
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
                      style={{
                        background: toneWash("accent"),
                        color: toneColor("accent"),
                      }}
                    >
                      {context}
                    </span>
                  )}
                  {filled(status) && (
                    <Eyebrow className="text-[0.5625rem]">{status}</Eyebrow>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  {onExpand && (
                    <button
                      type="button"
                      className="text-btn shrink-0"
                      aria-label={`Open ${label.toLowerCase()} in the full view`}
                      onClick={onExpand}
                    >
                      <span className="eyebrow whitespace-nowrap text-[0.5625rem]">
                        full view
                      </span>
                    </button>
                  )}
                  {onClear && (
                    <button
                      type="button"
                      className="text-btn shrink-0"
                      aria-label={`Clear ${label.toLowerCase()}`}
                      onClick={onClear}
                    >
                      {/* The class rather than `Eyebrow`, which renders a div: a
                    button holds phrasing content, and a block inside one is
                    markup no browser is obliged to lay out sensibly. */}
                      <span className="eyebrow whitespace-nowrap text-[0.5625rem]">
                        clear
                      </span>
                    </button>
                  )}
                </div>
              </div>

            </>
          )}

          {/* The body is its own region, not part of the header's fragment: a
              composer-only dock still has to open it for the command palette
              and for an error, both of which have nowhere else to go. What it
              must NOT do is hold a floor of blank space under a transcript the
              screen is already drawing. */}
          {open && (filled(children) || filled(emptyLabel)) && (
            <div
              id={bodyId}
              ref={bodyRef}
              className={cx("scroll max-h-[42vh] px-3 py-2.5", !composerOnly && "min-h-[132px]")}
              role="log"
              aria-live={live ? "polite" : "off"}
              aria-label={`${label} output`}
            >
              {filled(children) ? children : <EmptyState title={emptyLabel} />}
            </div>
          )}

          {/*
        The row itself is `ConsolePrompt` — the same component the `/agent`
        screen's composer uses. It was written inline here, and the agent
        screen grew its own; extracting it and leaving this copy behind was the
        duplication the extraction was for. The collapse chevron is this host's
        `lead`, which is also what stops it floating over the prompt text.
      */}
          <ConsolePrompt
            ref={inputRef}
            value={value}
            onValueChange={onValueChange}
            onSubmit={submit}
            onKeyDown={onPromptKeyDown}
            placeholder={placeholder}
            label={label}
            compact={!open}
            busy={busy}
            onStop={onStop}
            context={promptContext}
            attachments={attachments}
            onPasteImages={onPasteImages}
            onPickImages={onPickImages}
            shortcutHint={shortcutHint}
            onFocus={() => {
              if (!open) onOpenChange(true);
            }}
            lead={
              <>
                <button
                  type="button"
                  className="agent-mark !h-[19px] !w-[19px] !rounded-[5px]"
                  aria-label={
                    open
                      ? `Collapse ${label.toLowerCase()}`
                      : `Expand ${label.toLowerCase()}`
                  }
                  aria-expanded={open}
                  // Only while the panel exists: an `aria-controls` pointing at
                  // nothing is a promise to assistive technology that cannot be kept.
                  aria-controls={open ? bodyId : undefined}
                  onClick={() => onOpenChange(!open)}
                >
                  {/* Inline rather than an icon-set import: the kit takes no
                dependency on lucide, and these are the only glyphs it needs. */}
                  <svg
                    width="11"
                    height="11"
                    viewBox="0 0 24 24"
                    fill="none"
                    aria-hidden="true"
                  >
                    <path
                      d={open ? "m6 9 6 6 6-6" : "m6 15 6-6 6 6"}
                      stroke="currentColor"
                      strokeWidth="2.5"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                </button>
                {promptLead}
              </>
            }
          />
        </div>
        {insetRight > 0 && (
          // Decorative: it carries no content, and a screen reader announcing
          // an empty region would be noise. What it does is continue the
          // rail's column to the bottom of the window.
          <div
            aria-hidden
            className="shrink-0 self-stretch"
            style={{
              width: insetRight,
              background: "var(--surface)",
              borderLeft: "1px solid var(--rule)",
            }}
          />
        )}
      </div>
    </section>
  );
}
