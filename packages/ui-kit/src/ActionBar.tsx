import { useEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "./Button";
import { cx } from "./cx";
import type { IconComponent } from "./IconButton";
import { Popover } from "./Popover";
import { filled } from "./slot";
import { toneColor } from "./tone";
import { useCopied, type CopyState } from "./useCopied";

export interface ActionBarAction {
  /** Unique within the bar. Identifies the action, never shown. */
  id: string;
  label: string;
  icon?: IconComponent;
  /** Draws it as destructive. The label still has to say what it does. */
  danger?: boolean;
  /**
   * Why the action cannot be taken right now — an RBAC verdict, a resource in
   * the wrong phase. Present means blocked; the string is the explanation, and
   * a blocked action with no explanation is worse than no action at all.
   */
  disabledReason?: string;
  /**
   * Turns this into an action that answers. After `onSelect` resolves — and
   * does not resolve `false` — the control shows this word with a check for a
   * moment, then goes back to `label`.
   *
   * Drawn wherever the action landed, bar or overflow. It was the bar only at
   * first, on the reasoning that a menu row closes on the pick and a
   * confirmation drawn on something already gone is no confirmation — which
   * mistook a consequence for a constraint. Whether an action sits on the bar
   * is a matter of how many actions come before it, so "Copy as kubectl"
   * confirmed on a ConfigMap and not on a Pod, in the same footer, for no
   * reason the reader could see. The menu now holds still for a confirming
   * pick instead. (#410, #413 review)
   */
  confirmLabel?: string;
  onSelect: () => void | boolean | Promise<void | boolean>;
}

export interface ActionBarProps {
  actions: ActionBarAction[];
  /** Names the group — "Pod actions", "Actions for nginx-7d4b". */
  label: string;
  /** How many stay on the bar before the rest fold into a menu. */
  max?: number;
  moreLabel?: string;
  /** A ruled-off row under the menu — copy commands, deep links. */
  menuFooter?: ReactNode;
  className?: string;
}

/** What the mock settled on, and what an unusable `max` falls back to. */
const DEFAULT_MAX = 4;

/** Dimming is not a message; this is the word that goes with it. */
const NO_ACCESS = "No access";

/**
 * The row of actions on a resource: the first few on the bar, the rest behind a
 * menu, and the ones you are not allowed to take marked as such rather than
 * hidden.
 *
 * The mock's version was welded to the product. It looked the action list up
 * from a resource kind, checked each verb against RBAC, opened its own dialogs
 * and raised its own toasts — none of which can come into a kit that may not
 * import the service layer. So the knowledge arrives as props: the caller
 * decides what the actions are, whether each is permitted, and what running one
 * does. What is left here is the shape, which is the part every screen was
 * repeating.
 *
 * Every button is `type="button"`. The mock's were bare, and a bare button
 * inside a form submits it — a detail page is a form, and "Restart" that
 * submits it instead is a real outcome. So was its overflow trigger, which was
 * worse: a `<span class="btn" title="More actions">`, which no keyboard can
 * reach, no screen reader announces as a control, and nothing but a mouse can
 * open. It is a real button here, named by text rather than by a `title` that
 * the accessibility tree only reads as a description.
 *
 * A blocked action keeps the reason reachable. The mock disabled the button and
 * wrapped it in a tooltip explaining why — but a disabled button takes no
 * focus, so a keyboard user could never reach the explanation, and in several
 * browsers it swallows pointer events too, so the tooltip did not fire reliably
 * for a mouse either. `aria-disabled` says the same thing to assistive
 * technology while leaving the button focusable, the click is refused here, and
 * the reason rides along as the accessible description. In the menu, where
 * there is room for it, the block is also stated in words: the mock dimmed the
 * row to 45% opacity, and opacity is not a message.
 *
 * `max` is clamped. It reads like a constant and is written as arithmetic at
 * the call site — how many buttons fit beside a title — and the mock handed the
 * result straight to `slice`. At zero the bar showed nothing and the menu
 * showed everything; at a negative it was worse than useless, because
 * `slice(0, -2)` keeps all but the last two and `slice(-2)` keeps only those
 * two, so a bar built from both silently dropped nothing but also duplicated
 * nothing while reordering the list into two overlapping halves. One action
 * always stays on the bar, and every action appears exactly once.
 *
 * The menu is `Popover`, which is where this kit keeps the anchoring, the flip,
 * the outside click and the Escape. Its rows are buttons rather than
 * `role="menuitem"`: a real menu owes the user arrow-key navigation and a
 * roving tab stop, and a handful of buttons in a dialog that Tab reaches in
 * order is honest about what it is. (#320)
 */
export function ActionBar({
  actions,
  label,
  max = DEFAULT_MAX,
  moreLabel = "More actions",
  menuFooter,
  className,
}: ActionBarProps) {
  // An empty group is still a group: a named box, with padding, announced to a
  // screen reader as containing nothing.
  if (actions.length === 0) return null;

  const limit = Number.isFinite(max) ? Math.max(1, Math.floor(max)) : DEFAULT_MAX;
  const head = actions.slice(0, limit);
  const rest = actions.slice(limit);

  return (
    <div
      role="group"
      aria-label={label}
      className={cx("flex flex-wrap items-center gap-1.5", className)}
    >
      {head.map((a) => <BarButton key={a.id} action={a} />)}

      {rest.length > 0 && (
        <Popover
          label={moreLabel}
          align="end"
          trigger={
            <span className="btn" data-size="icon-sm">
              {/* Inline rather than an icon-set import: the kit takes no
                  dependency on lucide, and this is the only glyph it needs. */}
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                <circle cx="5" cy="12" r="1.6" fill="currentColor" />
                <circle cx="12" cy="12" r="1.6" fill="currentColor" />
                <circle cx="19" cy="12" r="1.6" fill="currentColor" />
              </svg>
              {/* The trigger's name. A `title` is a description, not a name,
                  which is what left the mock's trigger anonymous. */}
              <span className="sr-only">{moreLabel}</span>
            </span>
          }
          className="w-[268px] p-0"
        >
          {(close) => (
            <div className="py-1">
              {rest.map((a) => <MenuRow key={a.id} action={a} close={close} />)}
              {filled(menuFooter) && (
                <div data-slot="menu-footer" className="rule-t mt-1 space-y-1.5 p-1.5">
                  {menuFooter}
                </div>
              )}
            </div>
          )}
        </Popover>
      )}
    </div>
  );
}

/* Inline rather than an icon-set import: the kit takes no dependency on lucide,
   and this is the only glyph it needs. */
function CheckGlyph() {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true">
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
 * One action on the bar.
 *
 * A component of its own rather than JSX inside the map, because an action that
 * confirms needs state and a hook cannot live in a loop — the number of actions
 * changes between renders, which is the same reason `Rail` reads its probes once
 * for the whole list. Each button owning its own confirmation is also what keeps
 * two of them from sharing one: copying and then restarting must not flash
 * "Copied" on Restart.
 */
function BarButton({ action: a }: { action: ActionBarAction }) {
  const { state, run } = useCopied();
  const blocked = filled(a.disabledReason);
  const Icon = a.icon;
  const confirming = filled(a.confirmLabel) && state === "copied";

  return (
    <Button
      type="button"
      variant={a.danger ? "danger" : "secondary"}
      // Not `disabled`: see the note above — the reason has to stay
      // reachable, and a disabled button cannot be focused to read it.
      aria-disabled={blocked || undefined}
      title={a.disabledReason}
      className="aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
      onClick={() => {
        if (blocked) return;
        // Only an action that asked to confirm is awaited for its answer;
        // every other one keeps the fire-and-forget call it had.
        if (filled(a.confirmLabel)) void run(a.onSelect);
        else void a.onSelect();
      }}
    >
      {confirming ? <CheckGlyph /> : Icon && <Icon size={12} aria-hidden="true" />}
      {actionWord(a, state)}
    </Button>
  );
}

/**
 * What the control says right now: the confirm word, the failure, or the label.
 *
 * One function for the bar and the menu so the two cannot drift, and it feeds
 * the accessible name as well as the visible text. That pairing is the point.
 * An earlier draft left the name pinned to `label` and put the outcome in a
 * live region beside it, which gave a screen reader the news twice and left the
 * button reading "Copied" under a name of "Copy as kubectl" — a visible word
 * that is not in the accessible name, which is the failure WCAG 2.5.3 is about
 * and which strands anyone driving the app by voice. A control with a word in
 * it confirms with that word; {@link CopyAnnounce} is for the icon-only
 * controls that have no word to change. (#413 review)
 */
function actionWord(a: ActionBarAction, state: CopyState): string {
  // `filled` answers "is there a word here", including the whitespace-only case
  // the rest of the kit treats as nothing; the `undefined` half is spelt out
  // beside it because `filled` returns a plain boolean and narrows nothing.
  const confirm = a.confirmLabel;
  if (confirm === undefined || !filled(confirm)) return a.label;
  if (state === "copied") return confirm;
  if (state === "failed") return `${a.label} failed`;
  return a.label;
}

/**
 * One row of the overflow menu.
 *
 * Its own component for the same reason {@link BarButton} is: an action that
 * confirms needs state, and a hook cannot live in a map.
 *
 * **A confirming pick does not shut the menu.** A row that runs and vanishes
 * takes its own answer with it, which is why "Copy as kubectl" confirmed
 * nothing on a Pod: the peek's footer keeps two actions on the bar, Copy is
 * fifth in the row menu's order, and everything past the second is here. It
 * worked on a ConfigMap, which has no logs or shell or forward ahead of it, so
 * the confirmation appeared or did not depending on the kind — in the same
 * footer, for no reason the reader could see. The row now holds still, swaps to
 * a check and the confirm word, and the menu closes when the confirmation is
 * spent. Every other row keeps the immediate close it had. (#410, #413 review)
 */
function MenuRow({ action: a, close }: { action: ActionBarAction; close: () => void }) {
  const { state, run } = useCopied();
  const blocked = filled(a.disabledReason);
  const confirms = filled(a.confirmLabel);
  const confirming = confirms && state === "copied";
  const Icon = a.icon;
  const word = actionWord(a, state);

  // Closed on the way back to idle rather than on a timer of its own, so the
  // menu and the word it is showing are governed by one clock: however long the
  // confirmation is worth looking at is exactly how long the menu is worth
  // holding open. `answered` is what tells the first idle — the state every row
  // mounts in — from the one the confirmation expires into.
  const answered = useRef(false);
  useEffect(() => {
    if (state !== "idle") {
      answered.current = true;
    } else if (answered.current) {
      answered.current = false;
      close();
    }
  }, [state, close]);

  return (
    <button
      type="button"
      // Named explicitly: otherwise the name is computed from everything in the
      // row, and a blocked action would be called "Delete No access". It
      // tracks the visible word rather than pinning to `label`, so the name and
      // the text never disagree — see {@link actionWord}.
      aria-label={word}
      aria-disabled={blocked || undefined}
      title={a.disabledReason}
      className="ns-row aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
      // The whole row, not just the glyph the mock tinted: on the bar
      // `.btn-danger` colours the label too, and a menu that marks the same
      // action more quietly is the inconsistency.
      style={a.danger ? { color: toneColor("sev") } : undefined}
      onClick={() => {
        // Left open on purpose: a menu that shuts looks like the action was
        // taken.
        if (blocked) return;
        // An action that answers keeps the menu until it has answered; see
        // above. Everything else closes on the pick, as it always did.
        if (confirms) {
          void run(a.onSelect);
          return;
        }
        close();
        a.onSelect();
      }}
    >
      {confirming ? <CheckGlyph /> : Icon && <Icon size={12} className="shrink-0" aria-hidden="true" />}
      <span className="flex-1 truncate">{word}</span>
      {blocked && <span className="path text-faint">{NO_ACCESS}</span>}
    </button>
  );
}
