import { useState, type ReactNode } from "react";
import { Popover as RadixPopover } from "radix-ui";
import { cx } from "./cx";
import { usePortalContainer } from "./portal";

export interface PopoverProps {
  /** What the trigger shows. It is the button's content, and so its name. */
  trigger: ReactNode;
  /** The panel's contents, given a way to dismiss itself if it wants one. */
  children: ReactNode | ((close: () => void) => ReactNode);
  /** Names the panel — what a screen reader says when it opens. */
  label: string;
  align?: "start" | "center" | "end";
  side?: "top" | "right" | "bottom" | "left";
  /** Goes on the panel, which is the part a caller sizes. */
  className?: string;
}

/**
 * A panel anchored to a trigger: filters, a detail peek, a small menu of
 * actions.
 *
 * The mock placed itself. It read the trigger's rect, guessed the panel's size,
 * decided whether there was room below, clamped both axes to the viewport with
 * an eight-pixel margin, and re-ran all of it on every scroll and resize — then
 * closed itself on an outside mousedown and on Escape. That is the same
 * library-sized problem `ConfirmDialog` describes, and this kit has already
 * settled how it answers it: presentational components are hand-written,
 * interactive ones wrap Radix. Nothing about the appearance changes — the
 * design's `.popover` and `.scroll` still do all the styling — and what Radix
 * supplies is the anchoring, the flip, the shift, the portal, the dismissal and
 * the `aria-expanded`/`aria-controls` pair.
 *
 * The reason this exists as a kit component rather than four lines at each call
 * site is the trap in the two lines of `style` below. `.popover` is written for
 * a panel that places itself, so it is `position: fixed`. Radix already fixes
 * and translates a wrapper around its content, and a fixed child leaves that
 * wrapper zero-sized — which is the box the collision logic measures, so the
 * panel flips and shifts against nothing. It has cost two people on this branch
 * a debugging session each, and `ColumnPicker` and `picker` both carry the fix
 * as a comment. Encapsulated here, no future component has to rediscover it.
 * `relative` rather than `static`, so the stylesheet's `z-index: 45` still
 * applies.
 *
 * Three things the mock got wrong beyond the positioning. Its trigger wore
 * `display: contents`, which removes the element's box — harmless there because
 * a separate wrapper div was measured instead, but Radix anchors on the trigger
 * itself, so a boxless trigger would be a zero-sized rect at the origin. Its
 * trigger also took `aria-label={label}`, renaming a trigger that already said
 * something; the label names the panel here, and the trigger names itself from
 * what it shows, which is what lets a speech-input user say it. And `trigger`
 * was a function of `open` — unnecessary now that Radix puts `data-state` on
 * the trigger, where a `data-[state=open]:` rule reads it without a re-render.
 * `children` keeps its render prop, because a panel ending in Apply or Clear
 * has to be able to dismiss itself and nothing else can tell it how; a plain
 * node is accepted too, since most panels are content rather than a form.
 * (#320)
 *
 * Inside a portal scope — one tab of a window that holds several — the panel
 * mounts into the tab's own node rather than the document body, so it is hidden
 * with the tab. A portal escapes the `hidden` attribute an inactive tab wears,
 * so a panel opened in one tab used to stay on screen over the next one, still
 * anchored to a trigger that had gone with the tab. That is the whole change: a
 * popover is already non-modal and already dismisses on an outside interaction,
 * which is the right behaviour for it and none of the dialog's business.
 * Outside a scope nothing changes. (#357)
 */
export function Popover({
  trigger,
  children,
  label,
  align = "start",
  side = "bottom",
  className,
}: PopoverProps) {
  // Held here rather than left to Radix, because `close` is the whole point of
  // the render prop and an uncontrolled root has nothing to hand out.
  const [open, setOpen] = useState(false);
  const container = usePortalContainer();

  return (
    <RadixPopover.Root open={open} onOpenChange={setOpen}>
      {/* Explicitly type="button": these stand in toolbars, a toolbar can stand
          in a form, and a button without a type submits it on the click that
          was meant to open the panel. `inline-flex` rather than the mock's
          `contents`, so the trigger has a box for the panel to be measured
          against. */}
      <RadixPopover.Trigger type="button" className="inline-flex">
        {trigger}
      </RadixPopover.Trigger>
      <RadixPopover.Portal container={container}>
        <RadixPopover.Content
          // The panel is a dialog by Radix's reckoning; `label` is what it is
          // called. Not repeated on the trigger — see the note above.
          aria-label={label}
          align={align}
          side={side}
          sideOffset={6}
          // The eight pixels the mock kept between the panel and the viewport
          // edge, handed to the collision logic instead of clamped by hand.
          collisionPadding={8}
          className={cx("popover scroll", className)}
          style={{
            // The trap. See the doc comment: a fixed child leaves Radix's
            // wrapper zero-sized, and that wrapper is what the collision logic
            // measures. Relative, not static, so `z-index: 45` survives.
            position: "relative",
            // The mock capped itself at the viewport height less its margins.
            // Same intent, measured by the popper rather than by us: a panel
            // taller than the room found for it scrolls, as `.scroll` says,
            // instead of running off the screen.
            maxHeight: "var(--radix-popover-content-available-height)",
          }}
        >
          {typeof children === "function" ? children(() => setOpen(false)) : children}
        </RadixPopover.Content>
      </RadixPopover.Portal>
    </RadixPopover.Root>
  );
}
