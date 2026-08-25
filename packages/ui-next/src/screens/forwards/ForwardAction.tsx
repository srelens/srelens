import { useState, type ReactNode } from "react";
import { Button } from "@srelens/ui-kit";
import { NewForwardDialog } from "./NewForwardDialog";

export interface ForwardActionProps {
  /** The cluster the forward is made in — a kubeconfig context NAME. */
  context: string;
  namespace: string;
  /** The KUBERNETES kind — `Service`, `Pod`. `kindToForwardTarget` inside the
   *  dialog turns it into `svc/`/`pod/`; nothing here spells either. */
  kind: string;
  name: string;
  /** The port on the far end: the one this control is standing next to. */
  remotePort: number;
  /**
   * The control's accessible name, which has to distinguish it from every
   * other forward on the surface — a Ports table has one per row and a
   * containers table one per port per container, and "Forward" four times over
   * names nothing at all.
   *
   * It must CONTAIN whatever {@link children} draws, so the name a reader sees
   * and the name a screen reader speaks are not two different labels for one
   * control.
   */
  label: string;
  /** What the control draws — a word on a table row, the port itself on a
   *  container's chip. */
  children: ReactNode;
}

/**
 * A control that opens §A.4's `New port forward` on the thing beside it,
 * prefilled.
 *
 * **The one affordance behind every door except the row menu's**, which
 * already has its own dialog slot (`useRowMenu`). A Service's Ports row and a
 * container's port both sit right next to the exact number the reader wants
 * forwarded, and until now both were text: the row menu offered `Port forward`
 * and minted a route that rendered a Placeholder, so there was no way to start
 * a forward from anywhere but the forwards screen's own header.
 *
 * **Nothing starts here.** This opens the dialog and stops. The local port is
 * the reader's to name (or the OS's to pick), the equivalent command is theirs
 * to read, and the browser switch is theirs to set — all of which live in the
 * dialog, and none of which a one-click "forward this now" would give them.
 *
 * The dialog is mounted only while open, per control. Radix portals it, so
 * where in the table this sits does not decide where it draws, and a control
 * that is never clicked costs a boolean.
 */
export function ForwardAction({
  context,
  namespace,
  kind,
  name,
  remotePort,
  label,
  children,
}: ForwardActionProps) {
  const [open, setOpen] = useState(false);

  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="xs"
        aria-label={label}
        onClick={() => setOpen(true)}
      >
        {children}
      </Button>
      {open && (
        <NewForwardDialog
          context={context}
          namespace={namespace}
          target={{ kind, name, remotePort }}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}
