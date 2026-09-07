import { useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { cx } from "./cx";
import { EmptyState } from "./EmptyState";
import { filled } from "./slot";

export interface DrillStep {
  id: string;
  /** What the move is called — "Signal", "Diagnose", "Act". */
  label: string;
  /** Everything the step shows. The card scrolls it; the app writes it. */
  content: ReactNode;
  /** The choice the step ends on: sideways, or forward to the next move. */
  actions?: ReactNode;
}

export interface DrillCardProps {
  steps: DrillStep[];
  /** The step being shown. One that names no step falls back to the first. */
  active: string;
  onActiveChange: (id: string) => void;
  title?: ReactNode;
  /** Beside the title — a live indicator, a severity badge. */
  headerAction?: ReactNode;
  /** Drop the card's border and fill the parent, for a drill that is a whole pane. */
  flush?: boolean;
  emptyLabel?: ReactNode;
  /** Names the step rail for assistive technology. */
  railLabel?: string;
  className?: string;
}

/**
 * The investigation card: a numbered rail of moves across the top, and the move
 * you are on underneath it.
 *
 * The rail is the point. An investigation in this product is always the same
 * short sequence — you see a signal, you work out what it is, you act — and the
 * numbers are what say which move you are on and that there are others. That
 * device is what comes into the kit; what filled it in the mock does not.
 *
 * Because what filled it was one particular incident, read straight out of the
 * app: the active incident and three step bodies out of a data module, the
 * console's `ask`, the router's `navigate`, a `ConfirmGate` over a proposal, and
 * the applied/rolled-back state of a change that had actually run. The kit may
 * not import `@srelens/core` and may not own that state, so a step is now
 * `{ id, label, content, actions }` and the app supplies all four. What is left
 * is smaller than the mock and deliberately so: this is a step rail with a
 * scrolling body, and every word in it is the caller's. `actions` is kept as its
 * own slot rather than folded into `content` because it is part of the device —
 * each move ends on a choice between going sideways and going forward, and the
 * two-up grid that presents that choice is the same in all three. (#320)
 *
 * The steps are tabs and are marked as such. They select which panel shows, they
 * can be visited in any order, and the mock already rendered exactly one body at
 * a time; the roles then promise arrow keys, a single Tab stop and a labelled
 * panel, so those are here, matching {@link Tabs} move for move. It is not
 * `Tabs`: that component's strip is a scrolling row of text labels and its
 * `label` is a string, while this one is a full-width grid whose numbers are
 * markup — and the numbers are the thing the design is doing. They are hidden
 * from assistive technology, which hears "tab, 1 of 3" and does not need to be
 * told "zero one" as well. The mock wrote them as `0{i + 1}`, which prints step
 * ten as "010"; they are padded rather than prefixed here.
 *
 * `active` naming a step that is not in `steps` falls back to the first rather
 * than showing an empty card, because that state is reachable in the ordinary
 * course of things — a drill reopened against a shorter list — and a card with a
 * rail and no body reads as a bug in the app rather than a stale id.
 *
 * `compact` from the mock does not come across: it trimmed a stat row from four
 * figures to two, which is a judgement about content this component no longer
 * holds. `flush` does, and is now orthogonal to the header — it drops the card's
 * chrome so the drill can be a pane, and the header appears whenever there is
 * something to put in it.
 */
export function DrillCard({
  steps,
  active,
  onActiveChange,
  title,
  headerAction,
  flush = false,
  emptyLabel = "Nothing to drill into",
  railLabel,
  className,
}: DrillCardProps) {
  const uid = useId();
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const tabId = (id: string) => `${uid}-tab-${id}`;
  const panelId = `${uid}-panel`;
  const current = steps.find((step) => step.id === active) ?? steps[0];

  function focus(id: string) {
    onActiveChange(id);
    // The new tab's tabIndex has not been updated yet, and focus() does not care.
    refs.current.get(id)?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    // From the focused step rather than from `active`: a controlled parent may
    // not have committed the change yet, and computing from stale state sends
    // the second arrow key off from a step the user has already left.
    const focused = steps.findIndex((step) => refs.current.get(step.id) === document.activeElement);
    const index = focused >= 0 ? focused : steps.findIndex((step) => step.id === current?.id);
    if (index < 0) return;
    let next: number | null = null;
    if (event.key === "ArrowRight") next = (index + 1) % steps.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + steps.length) % steps.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = steps.length - 1;
    if (next === null) return;
    // Otherwise the arrows also scroll the card and Home/End jump the page.
    event.preventDefault();
    focus(steps[next].id);
  }

  return (
    <div className={cx(flush ? "flex min-h-0 flex-1 flex-col" : "card overflow-hidden", className)}>
      {(filled(title) || filled(headerAction)) && (
        <div className="card-head">
          {/* A heading, not a div: a drill is a section of the page it sits on,
              and the mock's `card-title` div left it out of the outline
              entirely. Fixed at h2, as Panel's is, for the same reason. */}
          {filled(title) && <h2 className="card-title">{title}</h2>}
          {filled(headerAction) && (
            <div data-slot="header-action" className="ml-auto flex items-center gap-2">
              {headerAction}
            </div>
          )}
        </div>
      )}

      {current === undefined ? (
        <EmptyState title={emptyLabel} />
      ) : (
        <>
          <div className="rail rule-b" role="tablist" aria-label={railLabel} onKeyDown={onKeyDown}>
            {steps.map((step, index) => {
              const on = step.id === current.id;
              return (
                <button
                  key={step.id}
                  type="button"
                  role="tab"
                  className="rail-step"
                  data-active={on}
                  aria-selected={on}
                  // Only the current panel is rendered, so pointing the rest at
                  // it would name an element that is not there.
                  aria-controls={on ? panelId : undefined}
                  id={tabId(step.id)}
                  tabIndex={on ? 0 : -1}
                  ref={(node) => {
                    if (node) refs.current.set(step.id, node);
                    else refs.current.delete(step.id);
                  }}
                  onClick={() => onActiveChange(step.id)}
                >
                  <span className="idx" aria-hidden="true">
                    {String(index + 1).padStart(2, "0")}
                  </span>
                  <span>{step.label}</span>
                </button>
              );
            })}
          </div>

          <div
            // Keyed on the step so the entrance animation runs again on each
            // move, which is the mock's `rise` doing its one job.
            key={current.id}
            role="tabpanel"
            id={panelId}
            aria-labelledby={tabId(current.id)}
            // The panel scrolls, and a scrolling region has to be reachable by
            // keyboard or its content is unreadable without a pointer.
            tabIndex={0}
            className="rise scroll min-h-0 flex-1 p-4"
          >
            {current.content}
            {filled(current.actions) && (
              <div data-slot="actions" className="mt-6 grid gap-2 sm:grid-cols-2">
                {current.actions}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
