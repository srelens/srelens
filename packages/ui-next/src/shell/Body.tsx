import type { ClusterContext } from "@srelens/core";
import { Button, EmptyState, Screen as AppScreen } from "@srelens/ui-kit";
import { describe, screenFor } from "../lib/routes";
import { reconnectCluster } from "../lib/openCluster";
import { Placeholder, type PlaceholderProps } from "./Placeholder";

export interface BodyProps extends PlaceholderProps {
  /**
   * The cluster pinned to this tab when it has been paused. Kept at the router
   * boundary so even hidden, mounted tabs cannot keep capability readers
   * alive after their cluster is disconnected.
   */
  pausedContext?: ClusterContext;
  /**
   * Raise the lock surface over the window. Forwarded to the screen untouched
   * — see `RoutedScreenProps.onLocked` for what the contract is and why it is
   * not wrapped the way `onSwitchToClassic` is. The Placeholder has no use for
   * it and is handed the rest.
   */
  onLocked: () => void;
}

/**
 * One tab's content: the screen registered for its route, or the Placeholder.
 * This is the whole of the router. Everything about which screens exist lives
 * in `screenFor`; this only asks.
 */
export function Body({ onLocked, pausedContext, ...props }: BodyProps) {
  if (pausedContext) {
    const title = describe(props.route, pausedContext.name).title;
    return (
      <AppScreen title={title} eyebrow={pausedContext.name} fill>
        <EmptyState
          title={`${pausedContext.name} is paused`}
          hint="Reconnect this cluster to resume this view."
          action={<Button variant="primary" onClick={() => reconnectCluster(pausedContext)}>Reconnect</Button>}
          className="flex-1"
        />
      </AppScreen>
    );
  }
  const Screen = screenFor(props.route);
  return Screen ? (
    <Screen
      route={props.route}
      // The same two the Placeholder beside it consumes, down the same path —
      // see `RoutedScreenProps`. `Settings`'s Appearance pane is the one screen
      // that needs them, and it needs them because it carries the design
      // toggle; a screen that ignores them costs nothing.
      ported={props.ported}
      // The route the screen is ON, and the cluster its tab is looking at.
      // Exactly the pair the Placeholder's own "Open in classic" sends, so a
      // reader leaving from a screen and a reader leaving from a placeholder
      // land in the same place in classic.
      onSwitchToClassic={() => props.onOpenInClassic(props.route, props.clusterName)}
      // Passed straight through, not curried. `onSwitchToClassic` above needs
      // this tab's route and cluster closed over it; a lock is about the window
      // and knows nothing about which tab asked.
      onLocked={onLocked}
    />
  ) : (
    <Placeholder {...props} />
  );
}
