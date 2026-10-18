import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { ConsolePrompt } from "./ConsolePrompt";
import { IconButton } from "./IconButton";
import { ResizeHandle } from "./ResizeHandle";
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
  /**
   * The output filling the host's content area rather than a strip under it
   * (#944). The dock draws the control and grows to fill its own box; the
   * host hides whatever it stacks above. Absent means no Maximize control.
   */
  maximized?: boolean;
  onMaximizedChange?: (maximized: boolean) => void;
  /**
   * The output's height, once the reader has dragged it (#944). Absent, the
   * output is as tall as its content up to 42% of the viewport, as before.
   * The height is the host's to keep: the kit reports it, never stores it.
   */
  bodyHeight?: number;
  /** The drag's bounds, in pixels. */
  bodyBounds?: { min: number; max: number };
  /** Every step of a drag on the dock's top edge. Absent means no grip. */
  onBodyResize?: (height: number) => void;
  /** Once, when the drag settles — the moment worth persisting. */
  onBodyResizeCommit?: (height: number) => void;
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
  maximized = false,
  onMaximizedChange,
  bodyHeight,
  bodyBounds = { min: 96, max: 640 },
  onBodyResize,
  onBodyResizeCommit,
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
  // The output's drawn height, for the grip to start a drag from while the
  // host has not set one — the content decides it until the first drag.
  const [drawnHeight, setDrawnHeight] = useState(0);
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

  const hasBody = open && (filled(children) || filled(emptyLabel));
  // Only over a body: a grip above a bare prompt would resize nothing.
  const resizable = onBodyResize !== undefined && hasBody && !composerOnly && !maximized;
  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (!resizable || !body) return undefined;
    const measure = () => setDrawnHeight(body.clientHeight);
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(body);
    return () => observer.disconnect();
  }, [resizable]);
  const clampBody = (h: number) => Math.max(bodyBounds.min, Math.min(bodyBounds.max, h));

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
    <section
      aria-label={label}
      data-maximized={maximized || undefined}
      className={cx("console-dock", maximized && "flex min-h-0 flex-1 flex-col", className)}
    >
      {resizable && (
        <ResizeHandle
          label={label.toLowerCase()}
          edge="top"
          width={clampBody(bodyHeight ?? drawnHeight)}
          minWidth={bodyBounds.min}
          maxWidth={bodyBounds.max}
          onResize={onBodyResize}
          onCommit={onBodyResizeCommit}
        />
      )}
      <div className={cx("flex min-w-0", maximized && "min-h-0 flex-1")}>
        <div className={cx("flex min-w-0 flex-1 flex-col", maximized && "min-h-0")}>
          {open && !composerOnly && (
            <>
              <div
                className="rule-b flex items-center justify-between gap-3 px-2.5 py-1"
                style={{ background: "var(--ground-sunk)" }}
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
                {/* Icons, as the terminal panel beside it has them (#944): the
                    two panes do the same jobs and should look it. Each is
                    named for assistive technology and titled for the pointer. */}
                <div className="flex shrink-0 items-center gap-0.5" data-slot="dock-controls">
                  {/* The prompt's chevron does the same, but it is a glyph at
                      the far end of the window from this header, where the
                      reader looks for the controls of what they are reading.
                      A conversation folded away is kept: the prompt opens it
                      again (#942). */}
                  <IconButton
                    icon={ChevronDownGlyph}
                    label={`Minimize ${label.toLowerCase()}`}
                    title="Minimize"
                    aria-controls={bodyId}
                    onClick={() => {
                      onMaximizedChange?.(false);
                      onOpenChange(false);
                    }}
                  />
                  {onMaximizedChange && (
                    <IconButton
                      icon={maximized ? RestoreGlyph : MaximizeGlyph}
                      label={maximized ? `Restore ${label.toLowerCase()} size` : `Maximize ${label.toLowerCase()}`}
                      title={maximized ? "Restore" : "Maximize"}
                      onClick={() => onMaximizedChange(!maximized)}
                    />
                  )}
                  {onExpand && (
                    <IconButton
                      icon={OpenGlyph}
                      label={`Open ${label.toLowerCase()} in the full view`}
                      title="Open in full view"
                      onClick={onExpand}
                    />
                  )}
                  {onClear && (
                    <IconButton
                      icon={ClearGlyph}
                      label={`Clear ${label.toLowerCase()}`}
                      title="Clear the conversation"
                      onClick={onClear}
                    />
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
          {hasBody && (
            <div
              id={bodyId}
              ref={bodyRef}
              className={cx(
                "scroll px-3 py-2.5",
                maximized ? "min-h-0 flex-1" : bodyHeight === undefined && "max-h-[42vh]",
                !composerOnly && !maximized && bodyHeight === undefined && "min-h-[132px]",
              )}
              style={!maximized && bodyHeight !== undefined ? { height: clampBody(bodyHeight) } : undefined}
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
              background: "var(--ground-surface)",
              borderLeft: "1px solid var(--rule)",
            }}
          />
        )}
      </div>
    </section>
  );
}

/**
 * The header's glyphs, inline: the kit takes no dependency on an icon set,
 * and these are the shapes the app's terminal panel draws for the same four
 * jobs (#944).
 */
function glyph(paths: string[]) {
  function Glyph({ size = 14, className }: { size?: number; className?: string }) {
    return (
      <svg
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        className={className}
      >
        {paths.map((d) => (
          <path key={d} d={d} />
        ))}
      </svg>
    );
  }
  return Glyph;
}

const ChevronDownGlyph = glyph(["m6 9 6 6 6-6"]);
const MaximizeGlyph = glyph(["M15 3h6v6", "M9 21H3v-6", "M21 3l-7 7", "M3 21l7-7"]);
const RestoreGlyph = glyph(["M4 14h6v6", "M20 10h-6V4", "M14 10l7-7", "M3 21l7-7"]);
const OpenGlyph = glyph(["M15 3h6v6", "M10 14 21 3", "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"]);
const ClearGlyph = glyph(["M3 6h18", "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6", "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"]);
