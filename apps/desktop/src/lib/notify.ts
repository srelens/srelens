import { toast } from "sonner";

/**
 * Thin wrapper over the toast library so components depend on this stable
 * surface (and tests can mock it) rather than sonner directly. Toasts appear
 * top-right (see the `<Toaster>` mount in App).
 */
export const notify = {
  /** A completed operation, e.g. "Scaled web to 3". */
  success(message: string, description?: string): void {
    toast.success(message, description ? { description } : undefined);
  },
  /** A failed operation; `description` carries the error detail. */
  error(message: string, description?: string): void {
    toast.error(message, description ? { description } : undefined);
  },
  /** Neutral information. */
  info(message: string, description?: string): void {
    toast(message, description ? { description } : undefined);
  },
  /**
   * A newer app version is available. Carries a "View update" action that takes
   * the user to the Updates section; stays up for a while since it's passive.
   */
  updateAvailable(version: string, onView: () => void): void {
    toast("Update available", {
      description: `srelens ${version} is ready to install.`,
      action: { label: "View update", onClick: () => onView() },
      duration: 12000,
    });
  },
  /**
   * An OIDC-protected cluster needs an interactive sign-in. Carries a "Sign in"
   * action that starts the cluster login flow; stays up a while since it's an
   * action prompt the user may not react to immediately.
   */
  clusterSignIn(title: string, description: string, onSignIn: () => void): void {
    toast(title, {
      description,
      action: { label: "Sign in", onClick: () => onSignIn() },
      duration: 30000,
    });
  },
};
