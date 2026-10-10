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

/**
 * A field that will take this Escape for itself.
 *
 * A text field, a text area, a select or an editor: Escape there is the
 * field's. A search field is the exception the kit already makes — `FilterBar`
 * claims Escape only while it has a filter to drop, and leaves it to "whatever
 * this list is inside" once it is empty, so that a reader is not trapped one
 * level down by a field that has no use for the key.
 */
function claimedByField(el: Element | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el instanceof HTMLInputElement && el.type === "search") return el.value !== "";
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable;
}

/** What stood between the keypress and the panel, read before anything acted on it. */
interface Before {
  /** A dialog, menu, listbox or popover the reader opened is on top. */
  layer: boolean;
  /** Only a tooltip is up. */
  tooltip: boolean;
  /** Focus is in a field that takes Escape for itself. */
  field: boolean;
}

function lookBefore(): Before {
  let layer = false;
  let tooltip = false;
  for (const el of document.querySelectorAll(OPEN_LAYER)) {
    if (!shown(el)) continue;
    if (isTooltip(el)) tooltip = true;
    else layer = true;
  }
  return { layer, tooltip, field: claimedByField(document.activeElement) };
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
 * - **a field** — Escape in a text field or an editor is the field's. A
 *   search field keeps it only while it has text to clear, the kit's own rule;
 * - **a layer open on top** — a menu, a popover, a dialog closes first, and
 *   only that: one layer per press, innermost first;
 * - **a keypress something else has handled** — the panel's own handler when
 *   focus is inside it, or anything that says so with `defaultPrevented`;
 * - **another tab** — every tab of the window stays mounted, hidden. A panel
 *   open in a tab the reader is not looking at must not close on a key
 *   pressed in the one they are. `within` is this screen's own element, and
 *   an element inside a hidden tab is not shown.
 *
 * **Two listeners, because of the tooltip.** The floating layers are Radix's,
 * and Radix takes Escape in the capture phase on the document, closes its top
 * layer and calls `preventDefault` — for a tooltip exactly as for a dialog.
 * So by the time a listener in the bubble phase runs, "handled" is true and
 * the layer that handled it is gone: there is no telling a dialog that just
 * closed from a tooltip that just closed. And a tooltip is very often up —
 * the pointer is resting on the row the reader has just clicked. Deferring to
 * it would make the first Escape after a click do nothing visible, which is
 * the bug this exists to end.
 *
 * So the page is read first, in the capture phase on the WINDOW, which runs
 * before the document's: what layers were open, whether focus was in a field
 * that takes the key. The decision is made later, in the bubble phase, when
 * React's handlers have had their say — and a `defaultPrevented` that only a
 * tooltip can account for is not treated as someone else's claim. One press
 * then closes the tooltip and the panel together, and a dialog or a menu
 * still closes alone.
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
    let before: Before | null = null;

    function onCapture(event: KeyboardEvent) {
      before = event.key === "Escape" ? lookBefore() : null;
    }

    function onKeyDown(event: KeyboardEvent) {
      const was = before;
      before = null;
      if (event.key !== "Escape" || was === null) return;
      // A chord is some other command: ⌘Esc, Shift+Esc.
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      const here = within.current;
      if (!here || !shown(here)) return;
      if (was.field || was.layer) return;
      // Handled by someone — unless the someone was a tooltip being dismissed,
      // which is not a claim on the key.
      if (event.defaultPrevented && !was.tooltip) return;
      close.current();
    }

    window.addEventListener("keydown", onCapture, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onCapture, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, within]);
}
