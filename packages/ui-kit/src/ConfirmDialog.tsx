import { useCallback, useEffect, useRef, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { Button } from "./Button";
import { useOpenLayer } from "./portal";
import { Spinner } from "./Spinner";

export interface ConfirmDialogProps {
  title: ReactNode;
  message: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Modal confirmation dialog for destructive actions. Mounted only while open,
 * so dismissing — Escape, the overlay, Cancel — routes to `onCancel`.
 *
 * Built on Radix's Dialog rather than by hand. The first version of this
 * component wrote the modal contract out itself and drew twenty-two review
 * findings, sixteen of them in one function deciding which controls the browser
 * treats as tab stops: hidden inputs, collapsed ancestors, radio groups, inert
 * subtrees, positive tab indexes, `<details>` with and without a summary. That
 * is a library-sized problem, and the library already exists.
 *
 * Radix is headless, so nothing about the appearance changes: the design's own
 * `.card`, `.card-head` and `.section-body` still do all the styling, and the
 * markup below is the same as the hand-written version's. What Radix supplies
 * is the behaviour — focus trapping, Escape, the portal, the scroll lock,
 * layering when dialogs stack, and the ARIA wiring.
 *
 * What stays ours: `busy` blocks every dismissal path, because the action is
 * already in flight and dismissing would strand it; and the message scrolls
 * while the head and actions hold their place, so a long confirmation cannot
 * push the buttons out of a clipped card.
 *
 * Two seams in Radix's focus handling are also ours, both because this dialog
 * is mounted only while open — the opener is a button elsewhere in the app, so
 * there is no `Dialog.Trigger` to render. They are handled at the two hooks
 * below, `onOpenAutoFocus`/`onCloseAutoFocus` and the `busy` effect. (#324)
 *
 * Inside a portal scope — one tab of a window that holds several — the question
 * belongs to its tab: it mounts in that tab, covers that tab and no more, and
 * marks that tab's own content `inert` rather than taking the whole document
 * out of the accessibility tree. Radix has no single flag for modal-within-one-
 * part-of-the-page, so it is composed out of three. `busy` is untouched by any
 * of it: the action is in flight and every way out of *this* dialog is still
 * closed. What is now open is the way out of the tab, which is not what `busy`
 * was ever protecting. Outside a scope nothing changes at all. (#357)
 */
export function ConfirmDialog({
  title,
  message,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  danger = false,
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const contentRef = useRef<HTMLDivElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  // Held for as long as this component is mounted, which is as long as the
  // question is on screen: it is what puts `inert` on the surface's content.
  const { container, scoped, showing } = useOpenLayer();
  const overlayRef = useRef<HTMLDivElement>(null);
  const wash = { background: "color-mix(in srgb, var(--canvas-deep) 72%, transparent)" };

  // Whether an interaction outside the card was an interaction with this tab.
  const onOverlay = useCallback((target: EventTarget | null) => {
    return target instanceof Node && overlayRef.current?.contains(target) === true;
  }, []);

  // Confirming disables the button that was just pressed, and the browser then
  // blurs it — with both controls disabled there is no tab stop left, so focus
  // lands on the document and a keyboard or screen-reader user sits outside the
  // modal for the whole request. Radix does not catch this: its focus scope
  // ignores a focusout with a null `relatedTarget`, which is exactly what a
  // disabled control produces, and its recovery watches for removed nodes
  // rather than changed attributes. So watch the transition here. The content
  // can hold focus: Radix's focus scope gives it `tabindex="-1"`. (#324 review)
  useEffect(() => {
    const content = contentRef.current;
    if (content && !content.contains(document.activeElement)) content.focus();
  }, [busy]);

  return (
    <Dialog.Root
      open
      modal={!scoped}
      onOpenChange={(open) => {
        if (!open && !busy) onCancel();
      }}
    >
      <Dialog.Portal container={container}>
        {scoped ? (
          // Radix's own Overlay renders nothing at all when `modal` is false,
          // so a scoped dialog draws its own. Nothing is lost with it: what
          // that component adds is the body scroll lock and the pointer-events
          // shield, and both are document-wide answers to a question that now
          // stops at the tab.
          <div ref={overlayRef} data-slot="dialog-overlay" className="absolute inset-0 z-50" style={wash} />
        ) : (
          <Dialog.Overlay data-slot="dialog-overlay" className="fixed inset-0 z-50" style={wash} />
        )}
        <Dialog.Content
          ref={contentRef}
          data-slot="dialog-content"
          // Unscoped, Radix isolates the background with aria-hidden on the
          // surrounding content, which is stronger than aria-modal — it removes
          // the page from the accessibility tree rather than asking for it to
          // be ignored. aria-modal is set anyway: it costs nothing and is what
          // older assistive technology looks for.
          //
          // Scoped, it is wrong in the same breath: aria-modal asks assistive
          // technology to ignore everything outside the dialog, and everything
          // outside includes the tab strip and the cluster rail, which are the
          // exact surfaces the reader is meant to keep. The isolation there is
          // the surface's own `inert`, which stops at the tab.
          aria-modal={scoped ? undefined : "true"}
          // The card centres on whatever the overlay covers, so it follows the
          // overlay: on the tab when there is one, on the window when there is
          // not. `fixed` inside a tab would centre the card on the window while
          // the wash behind it covered only the tab.
          className={`card rise ${scoped ? "absolute" : "fixed"} left-1/2 top-1/2 z-50 flex max-h-[calc(100%-3rem)] w-[calc(100%-3rem)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden outline-none`}
          style={{ maxWidth: 448 }}
          // Radix returns focus to `Dialog.Trigger` on close, and there is none
          // here: the dialog is mounted by whatever code decided to open it. Its
          // own fallback — the element focused before the dialog mounted — is
          // the right one, but the content cancels that fallback in favour of
          // the trigger it expects, so with no trigger to focus the opener
          // never gets focus back. Capture it and restore it. This hook fires
          // before focus moves in, so the opener is still the active element.
          // (#324 review)
          //
          // True in both modes, by two different routes: the modal content
          // cancels the fallback outright, and the non-modal one cancels it
          // unless this handler has already prevented the event — which it
          // does, because it has an opener of its own to restore. (#357)
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
          // Both dismissal paths are blocked while the action is in flight.
          // Radix listens for Escape on the document and routes it to the
          // layer opened last, which is not necessarily the one the reader can
          // see: a dialog left open on a tab they switched away from is still
          // mounted and still stacked. A hidden tab's dialog answering that key
          // press is the same bug as a portal escaping `hidden`. (#357)
          onEscapeKeyDown={(event) => {
            if (busy || !showing) event.preventDefault();
          }}
          onPointerDownOutside={(event) => {
            if (busy) event.preventDefault();
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
            if (busy || (scoped && !onOverlay(event.target))) event.preventDefault();
          }}
        >
          <div className="card-head shrink-0">
            <Dialog.Title className="card-title">{title}</Dialog.Title>
          </div>
          {/* The card is capped and clips, so a long message — a manifest
              preview, a stack of validation errors — would push the actions out
              of view. The message scrolls; the head and actions do not shrink. */}
          <Dialog.Description asChild>
            <div className="section-body min-h-0 flex-1 overflow-y-auto text-[0.8125rem] text-muted">
              {message}
            </div>
          </Dialog.Description>
          <div className="card-head flex shrink-0 justify-end gap-2">
            <Button variant="secondary" onClick={onCancel} disabled={busy}>
              {cancelLabel}
            </Button>
            <Button variant={danger ? "danger" : "primary"} onClick={onConfirm} disabled={busy}>
              {busy ? <Spinner label="Working" /> : confirmLabel}
            </Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
