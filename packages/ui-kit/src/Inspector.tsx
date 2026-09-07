import { useId, type KeyboardEvent, type ReactNode } from "react";
import { EmptyState } from "./EmptyState";
import { StatusPill, statusTone, type StatusKind } from "./StatusPill";
import { Tabs, type TabItem } from "./Tabs";
import { cx } from "./cx";
import { filled } from "./slot";
import { toneColor, type Tone } from "./tone";

export interface InspectorFact {
  /**
   * What the figure is — "Ready", "Restarts", "Age". Read by assistive
   * technology, not drawn: the design's header shows bare figures, so a value
   * that needs a word on screen carries its own ("9/12 ready", not "9/12").
   */
  label: string;
  value: ReactNode;
  /** Tints the figure. Emphasis only: the label and value carry the meaning. */
  tone?: Tone;
}

export interface InspectorProps {
  /** The subject's name — the heading of the peek, and the panel's own name. */
  name: ReactNode;
  /** The line beneath it, saying what the subject is (e.g. "Deployment · checkout"). */
  subtitle?: ReactNode;
  /**
   * Marks the subject as one the caller has singled out — draws the dot before
   * the name. Whether there is a dot, only; its colour comes from
   * {@link InspectorProps.statusKind}.
   */
  flagged?: boolean;
  /**
   * What being flagged means, for anyone who cannot see the dot. Keep it true
   * at every severity: the dot is amber for a warning subject and red for a
   * danger one, and this one string stands for both. The severity itself is
   * announced by `status` beside it.
   */
  flaggedLabel?: string;
  /** The subject's state, in words. */
  status?: ReactNode;
  /**
   * Tones the status word and the flag dot together. Defaults to `neutral` for
   * the word; a flag with no kind at all keeps the severity tone, since there
   * is no word for it to echo.
   */
  statusKind?: StatusKind;
  /** The figures read across the header under the status. */
  facts?: InspectorFact[];
  /** Controls the caller owns, shown at the top right before Close. */
  actions?: ReactNode;
  /** Omit when something outside already offers a way out (a Drawer does). */
  onClose?: () => void;
  /** The panes on offer. No tabs, no strip. */
  tabs?: TabItem[];
  activeTab?: string;
  onTabChange?: (id: string) => void;
  /** Names the strip for assistive technology (e.g. "Resource views"). */
  tabsLabel?: string;
  /** The active pane. */
  children?: ReactNode;
  /** What to say when the active pane has nothing in it. */
  emptyLabel?: ReactNode;
  /** The bar across the bottom — the actions that apply to the whole subject. */
  footer?: ReactNode;
  className?: string;
}

/**
 * The peek: one subject, identified at the top, its panes beneath, its actions
 * along the bottom. Selecting a row fills it, and it never navigates — the full
 * view is a control in the header, which is the caller's to place.
 *
 * Two different things are called a mock in this component's history, so
 * neither is called that below. **The source pane** is the design-system HTML
 * this was ported from (#318, #320). **The design frames** are the two
 * screenshots the user supplied for the resource detail pane (#331). Where the
 * two disagreed, the frames won.
 *
 * The source pane is a pane and a Kubernetes workload viewer welded together:
 * it reads a `Workload`, calls `describe()` for the detail, and hard-codes five
 * panes of pods, containers, conditions and events. None of that can come into
 * the kit, and cutting it out is not a loss — the header is a name, a subtitle,
 * a state and some figures whatever the subject is, and the panes are content
 * the caller already has in hand. So the panes arrive as children and the
 * figures as {@link InspectorFact}s, the same line NavIcon drew when it stopped
 * knowing that pods are boxes. (#320)
 *
 * Sizing and docking are not here either. The source pane drags its own left
 * edge and remembers the width in localStorage; the kit already has that, with
 * focus handling and an Escape stack besides, in `Drawer`. Two components
 * owning one drag is how they drift, so this one is the contents and the caller
 * picks the container — a flex sibling in a `.panes` row, or a `Drawer` around
 * it, in which case its `onClose` is left off so there is only one Close. That
 * is also why it is a named region rather than the source pane's `aside`: a
 * region nests correctly inside whatever landmark it lands in, where a second
 * complementary beside Drawer's would just be noise.
 *
 * The strip is the kit's `Tabs` in its segmented variant. The design frames
 * draw a rounded segmented control; the source pane drew one too, as a row of
 * plain buttons with no keyboard contract at all. The kit took the look and
 * left the buttons — `Tabs` supplies the roving tabindex and the arrow keys,
 * and wears `.seg` to get the 7px radius as well. Nobody had to choose. (#331)
 *
 * The status word is tinted, so a bad state is coloured and a good one is not.
 * That is the frames' rule, and it is fixed here rather than offered as a prop
 * for the same reason the strip's variant is: this component is the frame. See
 * the call site for why a prop would have been worse than no prop. (#331)
 *
 * **The figures under the status are bare, and that was the user's call.**
 * This comment used to argue the other way: that "9/12 ready" and "84d" with no
 * word beside them are unreadable to anyone who did not already know which
 * column they came from, and so the kit rendered `Ready 9/12  Age 84d` instead.
 * The user supplied the frames and overruled it, and the design is theirs to
 * decide — so the visible line is now theirs. What the argument was actually
 * protecting survives underneath: the facts are still a description list, and
 * each label is still a `dt` beside its `dd`, only `sr-only`. A screen reader
 * hears "Age: 84d"; the screen shows "84d". The disagreement was about the ink,
 * not about the markup, and only the ink changed. (#331)
 */
export function Inspector({
  name,
  subtitle,
  flagged = false,
  flaggedLabel = "Needs attention",
  status,
  // No default here on purpose: StatusPill supplies its own "neutral", and
  // the flag dot needs to tell "the caller said neutral" from "the caller
  // said nothing", which a default would erase.
  statusKind,
  facts = [],
  actions,
  onClose,
  tabs = [],
  activeTab,
  onTabChange,
  tabsLabel,
  children,
  emptyLabel = "Nothing to show",
  footer,
  className,
}: InspectorProps) {
  const headingId = useId();
  const active = activeTab ?? tabs[0]?.id;
  const activeLabel = tabs.find((t) => t.id === active)?.label;

  function onKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (event.key !== "Escape" || !onClose || event.defaultPrevented) return;
    // A field inside a pane owns its own Escape — a filter box clearing itself
    // should not also close the panel around it. Drawer bails on the same set,
    // and for the same reason.
    const el = event.target as HTMLElement | null;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) {
      return;
    }
    // Handled here, so a Drawer or a dialog further out does not close as well
    // on the one keypress. Listening on the window instead is what makes two
    // panels back out together.
    event.preventDefault();
    onClose();
  }

  return (
    <section
      aria-labelledby={headingId}
      className={cx("pane", className)}
      onKeyDown={onKeyDown}
    >
      <header className="rule-b px-3 py-2">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              {flagged && (
                <>
                  {/* THE RULE: `flagged` decides WHETHER there is a dot;
                      `statusKind` decides WHAT COLOUR it is. Do not put the
                      severity tone back as a constant.

                      It was one, and the two channels of a single fact could
                      then contradict each other — a red dot beside an amber
                      word. That is not an exotic pairing: core's `k8sStatus`
                      has `UNSETTLED = { health: "warning", flagged: true }`
                      and returns it for every warning-health workload, so a
                      mid-rollout Deployment is the ordinary path through it.

                      With no `statusKind` at all there is no word to echo, and
                      a flag falls back to the severity tone it has always
                      meant — a muted "needs attention" dot would be a worse
                      answer than the red it replaced. (#331) */}
                  <span
                    data-slot="inspector-flag"
                    aria-hidden="true"
                    className="h-1.5 w-1.5 shrink-0 rounded-full"
                    style={{ background: toneColor(statusKind ? statusTone(statusKind) : "sev") }}
                  />
                  {/* The source pane's dot says "this one" in colour alone, which is
                      nothing to a screen reader and nothing to a colour-blind
                      reader either. The word is the marker; the dot is how it
                      looks.

                      That holds now the dot carries a severity as well: this
                      text says only that the subject was singled out, which is
                      as true of an amber subject as of a red one, and the
                      severity itself is announced by the status word beside
                      it. So the label did not have to change with the rule. */}
                  <span className="sr-only">{flaggedLabel}</span>
                </>
              )}
              {/* The largest text on the pane. The peek is read name-first,
                  and at 14px the subject was the same size as its own tabs. */}
              <h2 id={headingId} className="truncate text-[1.25rem] font-semibold">
                {name}
              </h2>
            </div>
            {filled(subtitle) && <p className="path mt-px truncate">{subtitle}</p>}
          </div>
          {(filled(actions) || onClose) && (
            <div data-slot="inspector-actions" className="flex shrink-0 items-center gap-1">
              {actions}
              {onClose && (
                <button type="button" className="icon-btn" aria-label="Close inspector" onClick={onClose}>
                  {/* Inline rather than an icon-set import: the kit takes no
                      dependency on lucide, and this is the only glyph it needs. */}
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                    <path d="M18 6 6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
                  </svg>
                </button>
              )}
            </div>
          )}
        </div>

        {(filled(status) || facts.length > 0) && (
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1">
            {filled(status) && (
              // Tinted, always. The design colours the status word when the
              // state is bad and leaves it plain when it is not, and this
              // component is the frame that design draws — the same reason the
              // strip's variant is fixed here rather than asked of the caller.
              // It costs nothing to say so: `tinted` is a no-op for success,
              // info and neutral, so the only word it can ever colour is a
              // danger or a warning. Reaching it through a prop instead would
              // have meant a screen passing its own <StatusPill tinted> into
              // `status`, nesting a pill in a pill and drawing two dots. (#331)
              <StatusPill status={status} kind={statusKind} tinted />
            )}
            {facts.length > 0 && (
              // A description list, because that is what these are — the
              // pairing is what makes "84d" an age rather than a number. The
              // terms are hidden rather than dropped: the design shows the
              // figures bare, and a `dt` costs nothing to anyone reading with
              // their eyes while being the whole of the meaning to anyone who
              // is not.
              <dl className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                {facts.map((fact, index) => (
                  // Not keyed by the label alone: "Ready" is a plausible label
                  // twice on one subject — pod readiness and container readiness —
                  // and two rows sharing a key become one another's reconciliation
                  // target. The list is the caller's order and is never reordered.
                  <div key={`${index}:${fact.label}`} className="flex items-baseline gap-1">
                    <dt className="sr-only">{fact.label}</dt>
                    <dd
                      className="fact"
                      style={fact.tone ? { color: toneColor(fact.tone) } : undefined}
                    >
                      {fact.value}
                    </dd>
                  </div>
                ))}
              </dl>
            )}
          </div>
        )}
      </header>

      {tabs.length > 0 && (
        <div className="rule-b px-2 py-1.5">
          <Tabs
            tabs={tabs}
            active={active!}
            onChange={onTabChange ?? (() => {})}
            label={tabsLabel}
            variant="segmented"
          />
        </div>
      )}

      <div
        className="pane-body"
        // Named rather than pointed at: `Tabs` owns the strip and puts no ids
        // on its buttons, so `aria-labelledby` has nothing to reference. The
        // panel still says which pane it is, which is the part a reader needs.
        role={tabs.length > 0 ? "tabpanel" : undefined}
        aria-label={tabs.length > 0 ? activeLabel : undefined}
        // A scrolling region has to be reachable, or its content is unreadable
        // to anyone driving the page from the keyboard.
        tabIndex={tabs.length > 0 ? 0 : undefined}
      >
        {filled(children) ? children : <EmptyState title={emptyLabel} />}
      </div>

      {filled(footer) && (
        <footer className="rule-t flex items-center gap-1 px-2 py-1.5">{footer}</footer>
      )}
    </section>
  );
}
