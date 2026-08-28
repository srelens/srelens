import { useEffect, useRef, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { Button } from "./Button";
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
      onOpenChange={(open) => {
        if (!open && !busy) onCancel();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay
          data-slot="dialog-overlay"
          className="fixed inset-0 z-50"
          style={{ background: "color-mix(in srgb, var(--canvas-deep) 72%, transparent)" }}
        />
        <Dialog.Content
          ref={contentRef}
          data-slot="dialog-content"
          // Radix isolates the background with aria-hidden on the surrounding
          // content, which is stronger than aria-modal — it removes the page
          // from the accessibility tree rather than asking for it to be
          // ignored. aria-modal is set anyway: it costs nothing and is what
          // older assistive technology looks for.
          aria-modal="true"
          className="card rise fixed left-1/2 top-1/2 z-50 flex max-h-[calc(100%-3rem)] w-[calc(100%-3rem)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden outline-none"
          style={{ maxWidth: 448 }}
          // Radix returns focus to `Dialog.Trigger` on close, and there is none
          // here: the dialog is mounted by whatever code decided to open it. Its
          // own fallback — the element focused before the dialog mounted — is
          // the right one, but the modal content cancels that fallback in favour
          // of the trigger it expects, so with no trigger to focus the opener
          // never gets focus back. Capture it and restore it. This hook fires
          // before focus moves in, so the opener is still the active element.
          // (#324 review)
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
          onEscapeKeyDown={(event) => {
            if (busy) event.preventDefault();
          }}
          onPointerDownOutside={(event) => {
            if (busy) event.preventDefault();
          }}
          onInteractOutside={(event) => {
            if (busy) event.preventDefault();
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
