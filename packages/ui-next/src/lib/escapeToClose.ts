import { useEffect, useRef, type RefObject } from "react";

/**
 * A layer the reader opened on top of the page, which owns Escape while it is
 * there: a dialog, a menu, a popover, a listbox. Radix marks each with
 * `data-state="open"` on the element carrying the role.
 */
const OPEN_LAYER = [
  '[role="dialog"][data-state="open"]',
  '[role="alertdialog"][data-state="open"]',
  '[role="menu"][data-state="open"]',
  '[role="listbox"][data-state="open"]',
  "[data-radix-popper-content-wrapper]",
].join(",");

/** Shown to the reader: not inside a tab, or anything else, that is hidden. */
function shown(el: Element): boolean {
  return el.closest("[hidden]") === null;
}

/**
 * A tooltip floats on the same machinery a popover does, and is not a layer
 * the reader opened: it is there because the pointer is resting on a cell —
 * which, a moment after clicking a row, it usually is. Deferring to it would
 * make the first Escape after a click do nothing visible, the very thing this
 * hook exists to end.
 */
function isTooltip(layer: Element): boolean {
  return layer.querySelector('[role="tooltip"]') !== null;
}

/** A field, where Escape is the field's own: clearing a filter, leaving an editor. */
function editing(el: Element | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable;
}

/**
 * Close something on Escape wherever focus is on the screen that holds it
 * (#883).
 *
 * The detail panel beside a list handled Escape only for a key pressed INSIDE
 * it — its own `onKeyDown`. But the panel is opened by clicking a row, which
 * leaves focus on the row, in the list. So the one keypress a reader tries
 * after opening it went nowhere, and closing meant a trip to the small button
 * in the panel's corner, for every resource they looked at.
 *
 * This listens on the document instead, and steps aside for everything that
 * has a better claim to the key:
 *
 * - **a keypress something has already handled** — the panel's own handler
 *   when focus is inside it, or a field's. `defaultPrevented` is how they say
 *   so, and React's handlers run before a listener on the document does;
 * - **a field** — Escape in a filter box or an editor is the field's;
 * - **a layer open on top** — a menu, a popover, a dialog closes first, and
 *   only that: one layer per press, innermost first;
 * - **another tab** — every tab of the window stays mounted, hidden. A panel
 *   open in a tab the reader is not looking at must not close on a key
 *   pressed in the one they are. `within` is this screen's own element, and
 *   an element inside a hidden tab is not shown.
 *
 * `onClose` is read through a ref, so a caller passing a fresh closure each
 * render does not take the listener down and put it back on every one.
 */
export function useEscapeToClose(
  open: boolean,
  onClose: () => void,
  within: RefObject<Element | null>,
): void {
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });

  useEffect(() => {
    if (!open) return undefined;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      // A chord is some other command: ⌘Esc, Shift+Esc.
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      const here = within.current;
      if (!here || !shown(here)) return;
      if (editing(document.activeElement)) return;
      for (const layer of document.querySelectorAll(OPEN_LAYER)) {
        if (shown(layer) && !isTooltip(layer)) return;
      }
      close.current();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, within]);
}
