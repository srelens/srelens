import type { ReactNode } from "react";
import type { ConfirmRequest } from "@srelens/core";
import { HostConfirmation } from "./HostConfirmation";
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
 * **It names no app, deliberately.** "Requested by app X (signed by Y)" is
 * the host vouching for who asked, and for a request that arrived over MCP
 * the host has no grounds for it: `extensions.action` is reachable there, the
 * registry checks only that the call's `resource.id` and `revision` name an
 * installed, enabled app, and nothing authenticates the caller AS that app.
 * Attribution read off the request would be provenance chosen by the party
 * being vouched for — the spoof this whole component exists to prevent — so
 * the requester line is left off rather than filled in from the payload. An
 * app's own screens pass a real `app` to {@link HostConfirmation} because
 * they have host context for it; this adapter never does, and the line
 * returns here only when an authenticated host-owned execution context
 * carries the app (#549).
 *
 * **The seam for the patch.** {@link HostConfirmation} draws the exact patch
 * through the same collapsing diff renderer the Edit screen uses, and nothing
 * is passed here because no capability produces one yet: the writes behind
 * `k8s.gitOpsAction` are a closed `match` in core, not the declared
 * primitives of #549. When a request carries the patch it is about to apply,
 * it arrives as `DiffRow[]` on `ConfirmRequest` and is handed straight to
 * `patch` — the component, its collapsing and its tests are already here.
 */
export function RequestConfirmation({
  request,
  frame,
  details,
  actions,
}: {
  request: ConfirmRequest;
  frame?: "dialog" | "card";
  details?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <HostConfirmation
      question={request.prompt ?? null}
      impact={request.impact ?? null}
      cluster={request.target?.cluster ?? null}
      subject={confirmSubject(request.target)}
      frame={frame}
      details={details}
      actions={actions}
    />
  );
}
