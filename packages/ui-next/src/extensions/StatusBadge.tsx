import { Badge, type BadgeTone } from "@srelens/ui-kit";
import type { NormalizedStatus, ResolvedStatus } from "@srelens/core";
import { plainText } from "./displayText";

/** The kit tone each normalized status draws in (#541). */
export const STATUS_TONE: Record<NormalizedStatus, BadgeTone> = {
  healthy: "ok",
  warning: "warn",
  error: "sev",
  progressing: "info",
  suspended: "muted",
  unknown: "muted",
};

/** The host's own word for each status: the dashboard legend, and a badge's fallback. */
export const STATUS_WORD: Record<NormalizedStatus, string> = {
  healthy: "Healthy",
  warning: "Warning",
  error: "Error",
  progressing: "Progressing",
  suspended: "Suspended",
  unknown: "Unknown",
};

/**
 * A status an app's rules resolved, drawn as a word in a toned badge.
 *
 * The word is always there — colour is never the only signal — and it and the
 * reason are an app's and a cluster's text, drawn through `plainText` so they
 * cannot reorder or hide the host's text around them. The reason is shown on
 * the row only where there is room for it (`showReason`); otherwise it is the
 * hover title and a visually hidden sentence, so a screen reader hears what a
 * pointer sees.
 */
export function StatusBadge({ resolved, showReason = false }: { resolved: ResolvedStatus; showReason?: boolean }) {
  const known = Object.hasOwn(STATUS_TONE, resolved.status);
  const word = plainText(resolved.label ?? "").trim() || (known ? STATUS_WORD[resolved.status] : STATUS_WORD.unknown);
  const reason = resolved.reason ? plainText(resolved.reason) : undefined;
  return (
    <span className="extension-status" data-status={known ? resolved.status : "unknown"} title={reason}>
      <Badge tone={known ? STATUS_TONE[resolved.status] : "muted"}>{word}</Badge>
      {reason && (showReason
        ? <span className="extension-status-reason">{reason}</span>
        : <span className="sr-only">: {reason}</span>)}
    </span>
  );
}
