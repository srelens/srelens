import { screenFor } from "../lib/routes";
import { Placeholder, type PlaceholderProps } from "./Placeholder";

/**
 * One tab's content: the screen registered for its route, or the Placeholder.
 * This is the whole of the router. Everything about which screens exist lives
 * in `screenFor`; this only asks.
 */
export function Body(props: PlaceholderProps) {
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
    />
  ) : (
    <Placeholder {...props} />
  );
}
