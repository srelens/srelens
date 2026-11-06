import { Button } from "@srelens/ui-kit";
import { useScreenContext } from "./clusters";
import { detailRoute } from "./detailRoute";
import { openTab } from "./tabsStore";

/**
 * A node's name, as the way to that node (#822).
 *
 * Going from a pod to the node it runs on is one of the commonest steps in
 * working out why the pod is unwell, and the name was plain text wherever a
 * pod showed it: the reader carried it to Cluster → Nodes and found it again.
 *
 * Opens the node in a tab of its own — `detailRoute("Node", …)`, the route
 * Overview's node rows and the Nodes list both open — rather than a peek,
 * because the reader is leaving the pod's list for somewhere else and wants
 * the list still there to come back to.
 *
 * The cluster is the screen's, read here rather than passed in: a pinned
 * detail tab's own (#834) through `useScreenContext`, and the rail's
 * everywhere else, so the node opens on the cluster the pod was read off.
 * With no cluster resolved there is nowhere to open it, and the name is text.
 *
 * Inside a table row the click, the double-click and Enter/Space stop here,
 * for the reason `AskChip` gives: the row underneath peeks on a click and
 * opens ITS resource on a double-click or Enter, and the reader asked for the
 * node.
 */
export function NodeLink({ name }: { name: string }) {
  const context = useScreenContext();
  if (!context) return <>{name}</>;
  const label = `Open node ${name}`;
  return (
    <Button
      type="button"
      // The kit's link: underlined at rest, because a name with nothing drawn
      // on it reads as text, and nobody clicks text to find out.
      variant="link"
      className="max-w-full truncate text-left"
      title={label}
      aria-label={label}
      onClick={(e) => {
        e.stopPropagation();
        openTab(detailRoute("Node", null, name, { cluster: context.name }), { clusterName: context.name });
      }}
      onDoubleClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") e.stopPropagation();
      }}
    >
      {name}
    </Button>
  );
}
