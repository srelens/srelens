import type { ReactNode } from "react";
import type { ConfirmRequest } from "@srelens/core";
import { HostConfirmation, type ConfirmationApp } from "./HostConfirmation";
import { useConfirmationApp } from "./confirmationApp";
import { confirmSubject } from "./confirmRequest";

/**
 * One in-app MCP confirmation request, as the one host confirmation.
 *
 * The three surfaces a `mcp://confirm-request` can appear on — the new
 * design's `AgentConsent`, classic's `McpConfirmDialog` and the assistant
 * transcript's inline `ConfirmCard` — all render this, so the mapping from a
 * request to a question exists once. Before #552 each did its own, and the
 * card had already drifted: it drew the level as `· high impact` beside the
 * tool id while the two modals drew a badge, and neither named the cluster at
 * all.
 *
 * Each surface still owns its frame: `details` is whatever that frame adds
 * underneath (the tool id and the argument payload, a queue count, a failed
 * answer) and `actions` its own buttons, for a frame whose chrome has none.
 * Neither can change the question.
 *
 * **It names an app only when the host knows who asked.** "Requested by app
 * X (signed by Y)" is the host vouching for who asked, and for a request that
 * arrived over MCP the host has no grounds for it: `extensions.action` is
 * reachable there, the registry checks only that the call's `resource.id` and
 * `revision` name an installed, enabled app, and nothing authenticates the
 * caller AS that app. Attribution read off the arguments would be provenance
 * chosen by the party being vouched for — the spoof this whole component
 * exists to prevent — so an MCP request draws no requester line. The one
 * authenticated, host-owned execution context is an app's sidecar calling
 * back (#573): the supervisor started that process for one app and revision,
 * and the backend sends that reference as `requester`. Even then only the
 * reference crosses the wire; the name and publisher come from this window's
 * own inventory ({@link useConfirmationApp}), and a revision it no longer has
 * draws no line.
 *
 * **The seam for the patch.** {@link HostConfirmation} draws the exact patch
 * through the same collapsing diff renderer the Edit screen uses, and nothing
 * is passed here because no capability produces one yet: the declared action endpoint currently returns acceptance only. When a request carries the patch it is about to apply,
 * it arrives as `DiffRow[]` on `ConfirmRequest` and is handed straight to
 * `patch` — the component, its collapsing and its tests are already here.
 */
interface RequestConfirmationProps {
  request: ConfirmRequest;
  frame?: "dialog" | "card";
  details?: ReactNode;
  actions?: ReactNode;
}

export function RequestConfirmation(props: RequestConfirmationProps) {
  // Only a sidecar's request names its app, and only then is the inventory read:
  // an MCP request has no one to look up, and fetches nothing.
  return props.request.requester ? (
    <Requested {...props} requester={props.request.requester} />
  ) : (
    <Confirmation {...props} app={null} />
  );
}

/** A sidecar's request: the app it names, looked up in this window's inventory. */
function Requested(props: RequestConfirmationProps & { requester: { id: string; revision: number } }) {
  const app = useConfirmationApp(props.requester);
  return <Confirmation {...props} app={app} />;
}

function Confirmation({
  request,
  frame,
  details,
  actions,
  app,
}: RequestConfirmationProps & { app: ConfirmationApp | null }) {
  return (
    <HostConfirmation
      question={request.prompt ?? null}
      impact={request.impact ?? null}
      app={app}
      cluster={request.target?.cluster ?? null}
      subject={confirmSubject(request.target)}
      frame={frame}
      details={details}
      actions={actions}
    />
  );
}
