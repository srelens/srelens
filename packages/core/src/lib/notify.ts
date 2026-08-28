/**
 * How the service layer tells a human something happened.
 *
 * This used to call `sonner` directly, which quietly required React and
 * ReactDOM of anything importing the main barrel — a worker or CLI importing a
 * pure mapper would have pulled a toast component in with it, contradicting the
 * boundary `@srelens/core` advertises (#311 review).
 *
 * So the service layer declares *what* to say and the UI decides *how* to show
 * it: the app installs a notifier at startup, and until it does, notifications
 * go nowhere. Silence is the right default — a headless consumer has no screen
 * to render a toast on, and should not crash for want of one.
 */

export interface Notifier {
  /** A completed operation, e.g. "Scaled web to 3". */
  success(message: string, description?: string): void;
  /** A failed operation; `description` carries the error detail. */
  error(message: string, description?: string): void;
  /** Neutral information. */
  info(message: string, description?: string): void;
  /**
   * A newer app version is available. Carries a "View update" action that takes
   * the user to the Updates section; stays up for a while since it's passive.
   */
  updateAvailable(version: string, onView: () => void): void;
  /**
   * An OIDC-protected cluster needs an interactive sign-in. Carries a "Sign in"
   * action that starts the cluster login flow; stays up a while since it's an
   * action prompt the user may not react to immediately.
   */
  clusterSignIn(title: string, description: string, onSignIn: () => void): void;
}

const silent: Notifier = {
  success: () => {},
  error: () => {},
  info: () => {},
  updateAvailable: () => {},
  clusterSignIn: () => {},
};

let sink: Notifier = silent;

/**
 * Install the implementation that actually renders. Called once by the app;
 * returns a function that restores the previous sink, which is what tests use
 * to avoid leaking a spy into the next file.
 */
export function setNotifier(next: Notifier): () => void {
  const previous = sink;
  sink = next;
  return () => {
    sink = previous;
  };
}

/**
 * The surface every service module calls. Deliberately an object with the same
 * shape as before, so nothing that used it had to change.
 */
export const notify: Notifier = {
  success: (message, description) => sink.success(message, description),
  error: (message, description) => sink.error(message, description),
  info: (message, description) => sink.info(message, description),
  updateAvailable: (version, onView) => sink.updateAvailable(version, onView),
  clusterSignIn: (title, description, onSignIn) => sink.clusterSignIn(title, description, onSignIn),
};
