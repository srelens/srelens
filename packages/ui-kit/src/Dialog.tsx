import { useCallback, useRef, type KeyboardEvent, type ReactNode } from "react";
import { Dialog as Modal } from "radix-ui";
import { cx } from "./cx";
import { useOpenLayer } from "./portal";

export interface DialogProps {
  /** Names the dialog, and is drawn as its heading. */
  title: ReactNode;
  children: ReactNode;
  /** Escape, the overlay, and the header's own control all arrive here. */
  onClose: () => void;
  /** The controls along the bottom edge. Left out, there is no footer at all. */
  footer?: ReactNode;
  /** How wide the card may grow, in px. */
  maxWidth?: number;
  /** Names the header's control, for a design that would rather say "Cancel". */
  closeLabel?: string;
  className?: string;
}

/**
 * A compact modal with a title, a body and a row of controls: the frame around
 * one small task — customise this, rename that — that is too much for a popover
 * and too little for a screen.
 *
 * {@link ConfirmDialog} is the same frame with the task written into it, and
 * this is deliberately not a refactor of it: that component's contract is a
 * question and two answers, with `busy` blocking every way out while the answer
 * is in flight, and folding it into a generic shell would put a `message`,
 * a `confirmLabel` and a `busy` on every dialog that has none of them. What the
 * two do share is the Radix reasoning — the focus trap, the portal, the scroll
 * lock, the layering and the ARIA wiring are library-sized problems that the
 * hand-written version of that component drew twenty-two review findings for.
 *
 * The two seams Radix leaves open for a dialog mounted only while open are
 * handled the way ConfirmDialog handles them, and for the same reason: there is
 * no `Dialog.Trigger` to render, because whoever decided to open this is
 * somewhere else in the app, so the opener is captured on the way in and
 * focused again on the way out. (#325)
 *
 * Inside a portal scope — one tab of a window that holds several — this is
 * modal within that tab and non-modal outside it, which Radix has no single
 * flag for, so it is composed: the layer mounts into the tab's own node, the
 * overlay is `absolute` and covers only the tab, and the tab marks its own
 * content `inert` while the layer is held open. Outside a scope nothing
 * changes at all: the plain document-wide modal is still the right shape for a
 * dialog with no surface to belong to, and every existing call site — the
 * gallery, the window chrome, this kit's own tests — is one of those.
 *
 * Composed in one more place than it first looked. Giving the reader the tab
 * strip back gave it back to the pointer only, because Radix's focus scope
 * loops Tab whether or not it is trapping — so a keyboard user was still shut
 * in a dialog that no longer had any right to hold them. See `onTabOut`. (#357)
 */

/**
 * Everything the browser would stop at on Tab, in document order.
 *
 * Narrower than a general tab-order model on purpose. This design writes no
 * positive `tabindex`, so document order *is* the order; and the only things
 * that take a control out of the sequence in this window are the three checked
 * here — `disabled`, the `hidden` attribute an inactive tab wears, and the
 * `inert` a surface puts on its own content while a layer covers it. A control
 * hidden by CSS alone would be missed, which is a real limit and a quiet one:
 * it is why the caller below leaves the loop in place rather than moving focus
 * when it is not sure, so the worst case is the behaviour we already had.
 *
 * Radix's own focus guards are the one thing removed rather than counted. It
 * plants a tabbable span at each end of the body to catch focus on its way out
 * to the browser's chrome; they show nothing and do nothing, and landing on one
 * is indistinguishable from focus having vanished. They are also, when the tab
 * holds nothing but the dialog, exactly what sits next to it.
 */
function tabOrder(): HTMLElement[] {
  const candidates = document.body.querySelectorAll<HTMLElement>(
    "a[href],button,input,select,textarea,[tabindex]",
  );
  return Array.from(candidates).filter(
    (el) =>
      el.tabIndex >= 0 &&
      !el.hasAttribute("disabled") &&
      !el.hasAttribute("data-radix-focus-guard") &&
      el.closest("[inert],[hidden]") === null,
  );
}

export function Dialog({ title, children, onClose, footer, maxWidth = 420, closeLabel = "Close", className }: DialogProps) {
  const openerRef = useRef<HTMLElement | null>(null);
  // Held for as long as this component is mounted, which is as long as the
  // dialog is open: it is what puts `inert` on the surface's own content.
  const { container, scoped, showing } = useOpenLayer();
  const overlayRef = useRef<HTMLDivElement>(null);
  const wash = { background: "color-mix(in srgb, var(--canvas-deep) 72%, transparent)" };

  // Whether an interaction outside the card was an interaction with this tab.
  const onOverlay = useCallback((target: EventTarget | null) => {
    return target instanceof Node && overlayRef.current?.contains(target) === true;
  }, []);

  // Tab off either edge of the card, when the card is only as modal as its tab.
  //
  // Radix hands its focus scope `loop: true` and hardcodes it — there is no
  // prop, and its handler does not check `defaultPrevented`, so the loop cannot
  // be turned off from out here. It can be out-run. Radix's handler acts only
  // when the *live* focus is on the first or last control in the card, and it
  // reads that at the moment it runs; this one runs first, because Radix's
  // `Slot` composes a child's handler ahead of its own. So moving focus out
  // here leaves Radix's handler looking at a focus that is on neither edge, and
  // it does nothing.
  //
  // Only inside a surface. A dialog with no tab around it is modal over the
  // whole document, and being the only thing you can reach is what that means.
  const onTabOut = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (!scoped || event.key !== "Tab" || event.altKey || event.ctrlKey || event.metaKey) return;
      const card = event.currentTarget;
      const order = tabOrder();
      const inside = order.filter((el) => card.contains(el));
      const edge = event.shiftKey ? inside[0] : inside[inside.length - 1];
      if (edge === undefined || document.activeElement !== edge) return;
      const at = order.indexOf(edge);
      const next = order[(at + (event.shiftKey ? -1 : 1) + order.length) % order.length];
      // Nothing outside the card to go to — the tab is all there is, or the
      // window's own chrome is not in the list. Leave the loop to it rather
      // than strand focus.
      if (next === undefined || card.contains(next)) return;
      event.preventDefault();
      next.focus();
    },
    [scoped],
  );

  return (
    <Modal.Root open modal={!scoped} onOpenChange={(open) => !open && onClose()}>
      <Modal.Portal container={container}>
        {scoped ? (
          // Radix's own Overlay renders nothing at all when `modal` is false,
          // so a scoped dialog draws its own. Nothing is lost with it: what
          // that component adds is the body scroll lock and the pointer-events
          // shield, and both are document-wide answers to a question that now
          // stops at the tab.
          <div ref={overlayRef} data-slot="dialog-overlay" className="absolute inset-0 z-50" style={wash} />
        ) : (
          <Modal.Overlay data-slot="dialog-overlay" className="fixed inset-0 z-50" style={wash} />
        )}
        <Modal.Content
          data-slot="dialog-content"
          // Unscoped, Radix isolates the background with aria-hidden, which is
          // stronger than aria-modal — it removes the page from the
          // accessibility tree rather than asking for it to be ignored.
          // aria-modal is set anyway: it costs nothing and is what older
          // assistive technology looks for.
          //
          // Scoped, it is wrong in the same breath: aria-modal asks assistive
          // technology to ignore everything outside the dialog, and everything
          // outside includes the tab strip and the cluster rail, which are the
          // exact surfaces the reader is meant to keep. The isolation there is
          // the surface's own `inert`, which stops at the tab.
          aria-modal={scoped ? undefined : "true"}
          // The body is the caller's, so there is nothing here that reliably
          // describes the dialog. Left undefined rather than pointed at the
          // whole body, which Radix would otherwise read out entire.
          aria-describedby={undefined}
          className={cx(
            "card rise left-1/2 top-1/2 z-50 flex max-h-[calc(100%-3rem)] w-[calc(100%-3rem)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden outline-none",
            // The card centres on whatever the overlay covers, so it follows
            // the overlay: on the tab when there is one, on the window when
            // there is not. `fixed` inside a tab would centre the card on the
            // window while the wash behind it covered only the tab.
            scoped ? "absolute" : "fixed",
            className,
          )}
          style={{ maxWidth }}
          // Radix returns focus to `Dialog.Trigger` on close and there is none
          // here. Its own fallback — whatever was focused before the dialog
          // mounted — is the right one, but the content cancels that fallback
          // in favour of the trigger it expects, so with no trigger the opener
          // never gets focus back. This hook fires before focus moves in, so
          // the opener is still the active element.
          //
          // True in both modes, by two different routes: the modal content
          // cancels the fallback outright, and the non-modal one cancels it
          // unless this handler has already prevented the event — which it
          // does, because it has an opener of its own to restore.
          onOpenAutoFocus={() => {
            const active = document.activeElement;
            openerRef.current = active instanceof HTMLElement ? active : null;
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const opener = openerRef.current;
            // A dialog can outlive the control that opened it. Focusing a
            // detached node does nothing, so leave focus where it falls.
            if (opener?.isConnected) opener.focus();
          }}
          // What counts as dismissing this dialog, now that the window around
          // it is live again. A non-modal Radix layer dismisses on *any*
          // outside pointer-down, and with the overlay no longer covering the
          // tab strip, switching tabs is one — so the dialog closed the moment
          // the reader clicked away, losing whatever they had typed on a
          // surface that is hidden rather than unmounted precisely so it would
          // survive. The overlay is this tab's own dismiss affordance; the
          // shell around it — the strip, the cluster rail, the status bar — is
          // not, so only the overlay answers. Unscoped there is no live chrome
          // outside the modal at all, and the old behaviour is right.
          //
          // This hook and not `onPointerDownOutside`, which would look like the
          // obvious place: Radix calls this one after it on the pointer path
          // *and* on its own when focus leaves for a control outside, so it is
          // the only one of the two that sees every way out. Mutation testing
          // is what showed the pair was one hook doing the work and one
          // watching. (#357)
          onInteractOutside={(event) => {
            if (scoped && !onOverlay(event.target)) event.preventDefault();
          }}
          // Radix listens for Escape on the document and routes it to the
          // layer opened last, which is not necessarily the one the reader can
          // see: a dialog left open on a tab they switched away from is still
          // mounted and still stacked. A hidden tab's dialog answering that key
          // press is the same bug as a portal escaping `hidden`. (#357)
          onEscapeKeyDown={(event) => {
            if (!showing) event.preventDefault();
          }}
          onKeyDown={onTabOut}
        >
          <div className="card-head shrink-0">
            <Modal.Title className="card-title min-w-0 truncate">{title}</Modal.Title>
            <button type="button" aria-label={closeLabel} onClick={onClose} className="icon-btn shrink-0">
              {/* Inline rather than an icon-set import: the kit takes no
                  dependency on lucide, and this is the only glyph it needs. */}
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                <path d="M18 6 6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
              </svg>
            </button>
          </div>
          {/* The card is capped and clips, so a tall body — a palette, a long
              form — would push the controls out of view. The body scrolls; the
              head and the footer do not shrink. */}
          <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
          {footer !== undefined && (
            <div data-slot="dialog-footer" className="card-head shrink-0 justify-end gap-2 border-b-0 border-t">
              {footer}
            </div>
          )}
        </Modal.Content>
      </Modal.Portal>
    </Modal.Root>
  );
}
