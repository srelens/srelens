import { useCallback, useEffect, useRef, useState } from "react";
import type { IconComponent } from "./IconButton";

/** One confirmation window for every copy affordance in the design system. */
export const COPY_FEEDBACK_MS = 1_400;

export type ClipboardCopyStatusKind = "idle" | "copied" | "failed";

export interface ClipboardCopyFeedback {
  key: string | null;
  status: ClipboardCopyStatusKind;
  /** Changes on every attempt, including a repeated copy of the same value. */
  revision: number;
}

export interface ClipboardCopyController {
  feedback: ClipboardCopyFeedback;
  statusFor: (key: string) => ClipboardCopyStatusKind;
  write: (key: string, text: string) => Promise<boolean>;
}

const IDLE: ClipboardCopyFeedback = { key: null, status: "idle", revision: 0 };

/**
 * Clipboard writing and its short-lived outcome, shared by every copy control.
 *
 * A revision, rather than a boolean, makes a second click restart the timer.
 * The request number makes the latest click authoritative when two browser
 * writes settle out of order. Neither a missing clipboard nor a rejected write
 * can ever enter the copied state.
 */
export function useClipboardCopy(): ClipboardCopyController {
  const [feedback, setFeedback] = useState<ClipboardCopyFeedback>(IDLE);
  const latest = useRef(0);
  const mounted = useRef(true);
  const revertTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      latest.current += 1;
    };
  }, []);

  useEffect(() => {
    if (feedback.status === "idle") return;
    const revision = feedback.revision;
    const timer = setTimeout(() => {
      revertTimer.current = null;
      setFeedback((current) => (current.revision === revision ? IDLE : current));
    }, COPY_FEEDBACK_MS);
    revertTimer.current = timer;
    return () => {
      clearTimeout(timer);
      if (revertTimer.current === timer) revertTimer.current = null;
    };
  }, [feedback]);

  const write = useCallback(async (key: string, text: string): Promise<boolean> => {
    const request = ++latest.current;
    // A second attempt owns the confirmation window immediately. Without
    // clearing here, a slow clipboard promise can let the first window expire
    // and briefly put "Copy" back while the repeat attempt is still pending.
    if (revertTimer.current !== null) {
      clearTimeout(revertTimer.current);
      revertTimer.current = null;
    }
    try {
      const clipboard = typeof navigator === "undefined" ? undefined : navigator.clipboard;
      if (!clipboard?.writeText) throw new Error("clipboard unavailable");
      await clipboard.writeText(text);
      if (mounted.current && latest.current === request) {
        setFeedback((current) => ({ key, status: "copied", revision: current.revision + 1 }));
      }
      return true;
    } catch {
      if (mounted.current && latest.current === request) {
        setFeedback((current) => ({ key, status: "failed", revision: current.revision + 1 }));
      }
      return false;
    }
  }, []);

  const statusFor = useCallback(
    (key: string): ClipboardCopyStatusKind =>
      feedback.key === key ? feedback.status : "idle",
    [feedback],
  );

  return { feedback, statusFor, write };
}

/** A copy result must be spoken independently of a button-name change. */
export function ClipboardCopyStatus({ feedback }: { feedback: ClipboardCopyFeedback }) {
  const message =
    feedback.status === "copied"
      ? "Copied to clipboard"
      : feedback.status === "failed"
        ? "Could not copy to clipboard"
        : "";

  if (!message) return null;

  return (
    <span className="sr-only" role="status" aria-live="polite" aria-atomic="true">
      {message}
    </span>
  );
}

export const CopySuccessGlyph: IconComponent = ({ size = 12, ...rest }) => (
  <svg
    className="copy-command-check"
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    {...rest}
  >
    <path
      d="m5 13 4 4 10-10"
      stroke="currentColor"
      strokeWidth="3"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

export const CopyFailureGlyph: IconComponent = ({ size = 12, ...rest }) => (
  <svg
    className="copy-command-failure"
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    {...rest}
  >
    <path d="M6 6l12 12M18 6 6 18" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
  </svg>
);
