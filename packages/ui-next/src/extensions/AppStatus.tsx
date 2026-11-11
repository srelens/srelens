import { StatusPill, type StatusKind } from "@srelens/ui-kit";
import type { NormalizedStatus, ResolvedStatus } from "@srelens/core";
import { plainText } from "./displayText";

/** The kind each normalized status draws as (#541). */
export const STATUS_KIND: Record<NormalizedStatus, StatusKind> = {
  healthy: "success",
  warning: "warning",
  error: "danger",
  progressing: "info",
  suspended: "neutral",
  unknown: "neutral",
};

/** The host's own word for each status: the dashboard legend, and a status's fallback. */
export const STATUS_WORD: Record<NormalizedStatus, string> = {
  healthy: "Healthy",
  warning: "Warning",
  error: "Error",
  progressing: "Progressing",
  suspended: "Suspended",
  unknown: "Unknown",
};

/** The word a status is drawn with: the app's own, or the host's where it gave none. */
export function statusWord(resolved: ResolvedStatus): string {
  const known = Object.hasOwn(STATUS_KIND, resolved.status);
  return plainText(resolved.label ?? "").trim() || (known ? STATUS_WORD[resolved.status] : STATUS_WORD.unknown);
}

/** The kind a status is drawn in: its own, or neutral for a word the host doesn't know. */
export function statusKind(resolved: ResolvedStatus): StatusKind {
  return Object.hasOwn(STATUS_KIND, resolved.status) ? STATUS_KIND[resolved.status] : "neutral";
}

/**
 * A status an app's rules resolved, drawn as the standard status: a dot and a word.
 *
 * The word is always there — colour is never the only signal — and it and the
 * reason are an app's and a cluster's text, drawn through `plainText` so they
 * cannot reorder or hide the host's text around them. The reason is shown on
 * the row only where there is room for it (`showReason`); otherwise it is the
 * hover title and a visually hidden sentence, so a screen reader hears what a
 * pointer sees.
 */
export function AppStatus({ resolved, showReason = false }: { resolved: ResolvedStatus; showReason?: boolean }) {
  const known = Object.hasOwn(STATUS_KIND, resolved.status);
  const reason = resolved.reason ? plainText(resolved.reason) : undefined;
  return (
    <span className="extension-status" data-status={known ? resolved.status : "unknown"} title={reason}>
      <StatusPill status={statusWord(resolved)} kind={statusKind(resolved)} />
      {reason && (showReason
        ? <span className="extension-status-reason">{reason}</span>
        : <span className="sr-only">: {reason}</span>)}
    </span>
  );
}
