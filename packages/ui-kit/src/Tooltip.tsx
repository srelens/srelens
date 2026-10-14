import { createContext, useContext, useEffect, useState, isValidElement, type ComponentProps, type ReactNode } from "react";
import { Tooltip as RadixTooltip } from "radix-ui";
import { usePortalContainer } from "./portal";
import { filled } from "./slot";

export interface TooltipProps extends Omit<ComponentProps<typeof RadixTooltip.Trigger>, "children" | "asChild"> {
  /** The hint. Empty means there is nothing to say, and nothing is shown. */
  label: ReactNode;
  disabled?: boolean;
  /** What the hint is about. A single element becomes the target itself. */
  children: ReactNode;
  side?: "top" | "right" | "bottom" | "left";
}

const Grouped = createContext(false);

/** Neighbouring hints share the delay window; isolated Tooltip callers need no provider. */
export function TooltipGroup({ children }: { children: ReactNode }) {
  return <Grouped.Provider value={true}><RadixTooltip.Provider delayDuration={500} skipDelayDuration={300}>{children}</RadixTooltip.Provider></Grouped.Provider>;
}

/**
 * A short hint about the thing it is wrapped around.
 *
 * The mock was one span: `<span class="tip" data-tip={label}>`, with the text
 * drawn by `content: attr(data-tip)` on a `::after`. That is elegant and it is
 * also silent. A pseudo-element has no node, so it cannot carry an id, so
 * `aria-describedby` cannot point at it, and pseudo-element content is not
 * reliably exposed to a screen reader in any case. Sighted keyboard users were
 * served by `:focus-within`; screen-reader users were told nothing at all,
 * which on an icon-only button is the entire meaning of the control.
 *
 * The narrow fix — keep the CSS and add a visually hidden span for
 * `aria-describedby` to reference — was the tempting one, and it is not enough.
 * Three more failures come from the same `::after`, and none of them are fixed
 * by adding a node beside it. It is positioned inside its own wrapper, so any
 * clipping ancestor cuts it off, and this design clips almost everywhere:
 * `.popover` and the panes all set `overflow: hidden` or `auto`, and a hint on
 * a toolbar button inside one of them simply does not appear. It is
 * `white-space: nowrap` with no collision handling, so on the rightmost control
 * in a row it runs off the screen. And WCAG 1.4.13 asks that content shown on
 * hover or focus be dismissible without moving either, which CSS alone cannot
 * do. Portalling, flipping and Escape are three more library-sized problems on
 * top of the one we started with, so this takes Radix's Tooltip, which brings
 * all four — including the visually hidden `role="tooltip"` node that the
 * narrow fix was going to hand-roll. What is transcribed rather than reused is
 * the appearance: the bubble below is `.tip::after`'s own padding, radius,
 * size and tokens, minus its `pointer-events: none`, because 1.4.13 also asks
 * that a hint be hoverable.
 *
 * A standalone hint supplies its own provider. TooltipGroup shares the delay
 * window for neighbours such as document tabs, without changing other callers.
 * Trigger props and refs are forwarded so a ContextMenu can share the target.
 *
 * One more thing the mock got wrong: `.tip` is a plain span, so its
 * `:focus-within` fired only when the caller happened to wrap something
 * focusable — around a badge or a truncated string the hint was pointer-only.
 * A single element is used as the target as it stands, and anything else is
 * wrapped in a span that can take focus. (#320)
 *
 * Inside a portal scope — one tab of a window that holds several — the bubble
 * mounts into the tab's own node rather than the document body, so it is hidden
 * with the tab. This is the weakest of the kit's five portalled layers' cases
 * for that, and it is taken anyway: a hint lives and dies with hover and focus,
 * and hiding the tab it is in normally takes both away, so most of the time it
 * closes itself. Most of the time is the gap. A hint drawn beside a control
 * that is no longer on screen is a caption on the wrong picture, and one line
 * makes the question not arise. Outside a scope nothing changes. (#357)
 */
export function Tooltip({ label, children, side = "top", disabled = false, ...triggerProps }: TooltipProps) {
  const container = usePortalContainer();
  const grouped = useContext(Grouped);
  const [open, setOpen] = useState(false);
  useEffect(() => { if (disabled) setOpen(false); }, [disabled]);
  const content = (
      <RadixTooltip.Root open={open && !disabled} onOpenChange={setOpen}>
        <RadixTooltip.Trigger {...triggerProps} asChild>
          {isValidElement(children) ? (
            children
          ) : (
            // Raw content — a count, a truncated path — is not a tab stop, and
            // a hint a keyboard user cannot reach is the gap this component is
            // here to close.
            <span tabIndex={0} className="inline-flex">
              {children}
            </span>
          )}
        </RadixTooltip.Trigger>
        {/* `filled` rather than a truthiness check, and guarding the panel
            rather than the whole tree: a computed label that comes back empty
            drew a bare padded rectangle in the ink colour on the mock, and
            unmounting the tree instead would remount the caller's control —
            taking focus off it — every time the label went from empty to not. */}
        {filled(label) && (
          <RadixTooltip.Portal container={container}>
            <RadixTooltip.Content
              side={side}
              // `.tip::after` sat at `calc(100% + 5px)`.
              sideOffset={5}
              collisionPadding={8}
              className="z-50 whitespace-nowrap rounded-[5px] px-[0.4rem] py-[0.2rem] text-[0.6875rem]"
              // Tokens, so the bubble follows the theme: the ink colour on the
              // surface colour, inverted against the page exactly as the
              // stylesheet's own tooltip is.
              style={{ background: "var(--ink)", color: "var(--surface)" }}
            >
              {label}
            </RadixTooltip.Content>
          </RadixTooltip.Portal>
        )}
      </RadixTooltip.Root>
  );
  return grouped ? content : <RadixTooltip.Provider delayDuration={200}>{content}</RadixTooltip.Provider>;
}
