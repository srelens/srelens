import { cx } from "./cx";
import { filled } from "./slot";
import { Toast, type ToastProps } from "./Toast";

export interface SurfaceToastProps extends ToastProps {
  /**
   * `surface` pins the toast to the bottom-right of the nearest positioned
   * ancestor — the pane or panel the action happened in. That ancestor has to
   * be positioned; nothing here can check it. `window` pins it to the viewport,
   * clear of the status bar, for a message that belongs to no one pane.
   */
  anchor?: "surface" | "window";
}

/**
 * A {@link Toast} pinned to the corner of the thing it is about.
 *
 * A message about one pane belongs in that pane: a toast in the far corner of
 * the window, about a scale the reader triggered in a drawer, arrives detached
 * from what caused it. That is the whole of this component — where a toast
 * sits, given that something else decided it should appear.
 *
 * The mock's version read the host surface off React context and portalled into
 * it. Neither survives the move. Context would make the kit's toast depend on a
 * provider the kit does not ship, and a portal takes the toast out of its
 * subtree, which is where its stacking context, its theme attribute and any
 * scoped style live — and puts the kit in the business of managing where
 * toasts go, which is the app's. So the choice becomes one prop and the toast
 * renders where it is mounted. Like `Toast`, and for the reason given there,
 * there is no queue, no timer and no store behind this. (#320)
 *
 * Nothing is drawn when the toast has nothing to say: an empty positioned box
 * is invisible but still takes the clicks meant for whatever is under it.
 */
export function SurfaceToast({ anchor = "surface", className, ...toast }: SurfaceToastProps) {
  if (!filled(toast.title) && !filled(toast.hint)) return null;
  return (
    <div
      data-slot="toast-frame"
      data-anchor={anchor}
      className={cx(
        anchor === "surface" ? "absolute bottom-3 right-3 z-40" : "fixed bottom-16 right-4 z-50",
        className,
      )}
    >
      <Toast {...toast} />
    </div>
  );
}
