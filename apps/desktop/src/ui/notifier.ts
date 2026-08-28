import { toast } from "sonner";
import { setNotifier, type Notifier } from "@srelens/core";

/**
 * The sonner-backed implementation of the service layer's notifier.
 *
 * `@srelens/core` declares what to say; this decides how to show it. Keeping
 * sonner on this side is what lets the main barrel stay free of React — sonner
 * is a React component library, so importing it in a service module made every
 * consumer of the barrel need React and ReactDOM.
 *
 * Toasts appear top-right (see the `<Toaster>` mount in App).
 */
export const toastNotifier: Notifier = {
  success(message, description) {
    toast.success(message, description ? { description } : undefined);
  },
  error(message, description) {
    toast.error(message, description ? { description } : undefined);
  },
  info(message, description) {
    toast(message, description ? { description } : undefined);
  },
  updateAvailable(version, onView) {
    toast("Update available", {
      description: `srelens ${version} is ready to install.`,
      action: { label: "View update", onClick: () => onView() },
      duration: 12000,
    });
  },
  clusterSignIn(title, description, onSignIn) {
    toast(title, {
      description,
      action: { label: "Sign in", onClick: () => onSignIn() },
      duration: 30000,
    });
  },
};

/** Wire the service layer to sonner. Called once, before the app renders. */
export function installToastNotifier(): () => void {
  return setNotifier(toastNotifier);
}
