import { useCallback, useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from "react";

/**
 * The two keys a dialog scoped to one tab has to answer for itself.
 *
 * Radix owns Tab and Escape for a dialog, and both of its answers are right for
 * a dialog that owns the window: loop focus inside the card, and hand Escape to
 * whichever layer was opened last. Neither is right for a dialog that owns one
 * tab of a window that holds several, neither can be turned off from a prop,
 * and both replacements are small enough to get subtly wrong. {@link Dialog}
 * and {@link ConfirmDialog} need the same two, so they are here rather than
 * copied: the second copy is the one that does not get fixed. (#357)
 *
 * Nothing here is reachable without a portal scope. A dialog with no surface
 * around it — the gallery, the frozen classic app, most of this kit's tests —
 * registers nothing, listens to nothing, and is left entirely to Radix.
 */

/**
 * Everything the browser would stop at on Tab, in document order.
 *
 * Narrower than a general tab-order model on purpose. This design writes no
 * positive `tabindex`, so document order *is* the order; and the only things
 * that take a control out of the sequence in this window are the three checked
 * here — `disabled`, the `hidden` attribute an inactive tab wears, and the
 * `inert` a surface puts on its own content while a layer covers it.
 *
 * The limit that remains is a control hidden by CSS alone: `display: none`,
 * `visibility: hidden`, a zero-sized ancestor. Reading that back needs layout,
 * which is why it is not attempted rather than attempted badly. It is why the
 * caller below leaves the loop in place when it is unsure, so the worst case is
 * the behaviour we already had.
 *
 * `contenteditable` earns its place in the selector rather than in the list of
 * things a general model would want: the values editor in the Helm dialog is
 * CodeMirror, whose content DOM is a `contenteditable` with no tabindex, and it
 * sits inside a dialog this scoping applies to. The rest — a frame, a summary,
 * a media control — are here for completeness and cost one selector each.
 *
 * Radix's own focus guards are the one thing removed rather than counted. It
 * plants a tabbable span at each end of the body to catch focus on its way out
 * to the browser's chrome; they show nothing and do nothing, and landing on one
 * is indistinguishable from focus having vanished. They are also, when the tab
 * holds nothing but the dialog, exactly what sits next to it.
 */
const TABBABLE = [
  "a[href]",
  "area[href]",
  "button",
  "input",
  "select",
  "textarea",
  "iframe",
  "details > summary",
  "audio[controls]",
  "video[controls]",
  "[tabindex]",
  '[contenteditable]:not([contenteditable="false"])',
].join(",");

const EDITABLE = '[contenteditable]:not([contenteditable="false"])';

/**
 * Whether the browser stops at this element, for an element the selector above
 * already matched.
 *
 * `tabIndex` answers it for everything with a `tabindex` attribute, and for
 * everything the DOM has always agreed is focusable. It is not agreed on for a
 * `contenteditable` with no `tabindex` of its own — a browser reports 0, jsdom
 * reports -1 while still letting `focus()` land on it — so that one case is
 * read off the attribute, which everything agrees about.
 */
function tabbable(el: HTMLElement): boolean {
  if (el.hasAttribute("tabindex")) return el.tabIndex >= 0;
  return el.tabIndex >= 0 || el.matches(EDITABLE);
}

export function tabOrder(): HTMLElement[] {
  const candidates = document.body.querySelectorAll<HTMLElement>(TABBABLE);
  return Array.from(candidates).filter(
    (el) =>
      tabbable(el) &&
      !el.hasAttribute("disabled") &&
      !el.hasAttribute("data-radix-focus-guard") &&
      el.closest("[inert],[hidden]") === null,
  );
}

/**
 * Tab off either edge of a card that is only as modal as its tab.
 *
 * Radix hands its focus scope `loop: true` and hardcodes it — there is no prop,
 * the scope loops whether or not it is trapping, and its handler does not check
 * `defaultPrevented`, so the loop cannot be turned off from out here. It can be
 * out-run. Radix's handler acts only when the *live* focus is on the first or
 * last control in the card, and it reads that at the moment it runs; this one
 * runs first, because Radix's `Slot` composes a child's handler ahead of its
 * own. So moving focus out here leaves Radix's handler looking at a focus that
 * is on neither edge, and it does nothing.
 *
 * Returns a handler for the card's own `onKeyDown`. Inert outside a surface: a
 * dialog with no tab around it is modal over the whole document, and being the
 * only thing you can reach is what that means. Radix would enforce that anyway
 * — the focus scope is *trapping* there, and pulls focus straight back out of
 * anything this moved it to — so the check is what stops the two from fighting
 * over every Tab rather than what produces the behaviour. It is the one line
 * here a mutation cannot be made to show. (#357 review)
 */
export function useTabOut(scoped: boolean) {
  return useCallback(
    (event: ReactKeyboardEvent<HTMLElement>) => {
      if (!scoped || event.key !== "Tab" || event.altKey || event.ctrlKey || event.metaKey) return;
      const card = event.currentTarget;
      const order = tabOrder();
      const inside = order.filter((el) => card.contains(el));
      const edge = event.shiftKey ? inside[0] : inside[inside.length - 1];
      const step = event.shiftKey ? -1 : 1;
      let from: number;
      if (edge === undefined) {
        // No tab stop inside the card at all — every control in it disabled
        // while an action is in flight. Radix stops Tab dead there, so it is
        // the case where leaving matters most: there is nothing left in the
        // card to move to and nothing left in it to do. The card holds focus
        // itself then, on the `tabindex="-1"` Radix gives it, and it has no
        // slot in the order of its own, so it borrows the one it is travelling
        // away from.
        if (document.activeElement !== card) return;
        from =
          order.filter((el) => (card.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_PRECEDING) !== 0).length -
          (event.shiftKey ? 0 : 1);
      } else {
        if (document.activeElement !== edge) return;
        from = order.indexOf(edge);
      }
      const next = order[(from + step + order.length) % order.length];
      // Nothing outside the card to go to — the tab is all there is, or the
      // window's own chrome is not in the list. Leave the loop to it rather
      // than strand focus.
      if (next === undefined || card.contains(next)) return;
      event.preventDefault();
      next.focus();
    },
    [scoped],
  );
}

interface ScopedLayer {
  /** Whether this layer's surface is the one on screen. */
  showing: boolean;
  /** What answering Escape means for it, including deciding not to. */
  escape: () => void;
}

/** Every open scoped layer, in the order they mounted. */
const layers: ScopedLayer[] = [];

/** Escape key presses a hidden surface's layer refused without claiming. */
const declined = new WeakSet<Event>();

/**
 * What a layer on a hidden surface does with an Escape meant for another tab.
 *
 * Refusing to close is all Radix lets a layer say, and it is not enough: it
 * routes the key to the layer that mounted last and stops there, so a dialog
 * left open on a tab the reader switched away from silently swallows the key
 * from the dialog they opened afterwards on the tab they are looking at. What
 * that layer needs to say is that it is not the one being asked, which is not
 * expressible from `onEscapeKeyDown` — so it is said out here instead, and
 * {@link useScopedEscape} listens for it. (#357 review)
 */
export function declineEscape(event: KeyboardEvent): void {
  // Only when nothing above has already answered. `defaultPrevented` here means
  // a real handler — a select open inside the card, a window-wide modal over
  // it — took the key first, and declining after that would hand a key that is
  // already spent to a second dialog.
  if (!event.defaultPrevented) declined.add(event);
  event.preventDefault();
}

function answer(event: KeyboardEvent): void {
  // Answered for real by some layer of Radix's own, this dialog included when
  // it happened to be the one Radix routed to. Only a decline leaves the key
  // going spare.
  if (event.defaultPrevented && !declined.has(event)) return;
  const top = layers.filter((layer) => layer.showing).at(-1);
  top?.escape();
}

function onKeyDown(event: KeyboardEvent): void {
  if (event.key !== "Escape") return;
  // Deliberately after the dispatch rather than during it. Every layer decides
  // in this one dispatch, each from its own capture listener on the document,
  // in the order they mounted — so during it there is no way to know whether a
  // layer that has not run yet is about to take the key. Deferring to the end
  // of it is what makes this independent of that order.
  queueMicrotask(() => answer(event));
}

/** One listener for the whole window, for as long as any scoped layer is open. */
let listening = 0;

/**
 * Escape for a dialog that belongs to one tab.
 *
 * `showing` is what orders the answer: the last-mounted layer whose surface is
 * on screen is the one the reader is looking at. `onEscape` is called for that
 * layer alone, and is the caller's chance to decide the answer is no — a
 * confirmation with its action already in flight closes nothing.
 *
 * The one pairing this does not compose for: a dialog with no surface at all,
 * open at the same time as one inside a tab that has since been hidden. The
 * unscoped dialog is modal over the whole document, so it is the one that
 * should answer, and it is not in this list. Reaching it needs the reader to
 * switch tabs and open a second dialog while a document-wide modal holds the
 * window, which is the one thing such a modal does not let them do.
 */
export function useScopedEscape(scoped: boolean, showing: boolean, onEscape: () => void): void {
  const layer = useRef<ScopedLayer>({ showing, escape: onEscape }).current;

  // Every commit, because both change with a render rather than with the
  // registration: the surface is hidden and shown again while the layer stays
  // mounted, which is the whole point of it.
  useEffect(() => {
    layer.showing = showing;
    layer.escape = onEscape;
  });

  useEffect(() => {
    if (!scoped) return;
    layers.push(layer);
    if (listening++ === 0) document.addEventListener("keydown", onKeyDown, true);
    return () => {
      layers.splice(layers.indexOf(layer), 1);
      if (--listening === 0) document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [scoped, layer]);
}
