import type { ReactNode } from "react";
import { Button } from "./Button";
import { cx } from "./cx";
import type { IconComponent } from "./IconButton";
import { Popover } from "./Popover";
import { filled } from "./slot";
import { toneColor } from "./tone";

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
  onSelect: () => void;
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
      {head.map((a) => {
        const blocked = filled(a.disabledReason);
        const Icon = a.icon;
        return (
          <Button
            key={a.id}
            type="button"
            variant={a.danger ? "danger" : "secondary"}
            // Not `disabled`: see the note above — the reason has to stay
            // reachable, and a disabled button cannot be focused to read it.
            aria-disabled={blocked || undefined}
            title={a.disabledReason}
            className="aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
            onClick={() => {
              if (blocked) return;
              a.onSelect();
            }}
          >
            {Icon && <Icon size={12} aria-hidden="true" />}
            {a.label}
          </Button>
        );
      })}

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
              {rest.map((a) => {
                const blocked = filled(a.disabledReason);
                const Icon = a.icon;
                return (
                  <button
                    key={a.id}
                    type="button"
                    // Named explicitly: otherwise the name is computed from
                    // everything in the row, and a blocked action would be
                    // called "Delete No access".
                    aria-label={a.label}
                    aria-disabled={blocked || undefined}
                    title={a.disabledReason}
                    className="ns-row aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
                    // The whole row, not just the glyph the mock tinted: on the
                    // bar `.btn-danger` colours the label too, and a menu that
                    // marks the same action more quietly is the inconsistency.
                    style={a.danger ? { color: toneColor("sev") } : undefined}
                    onClick={() => {
                      // Left open on purpose: a menu that shuts looks like the
                      // action was taken.
                      if (blocked) return;
                      close();
                      a.onSelect();
                    }}
                  >
                    {Icon && <Icon size={12} className="shrink-0" aria-hidden="true" />}
                    <span className="flex-1 truncate">{a.label}</span>
                    {blocked && <span className="path text-faint">{NO_ACCESS}</span>}
                  </button>
                );
              })}
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
