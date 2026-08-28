import type { ReactNode } from "react";
import { cx } from "./cx";
import { IconButton, type IconComponent } from "./IconButton";
import { filled } from "./slot";
import { toneColor, type Tone } from "./tone";

/*
 * Inline rather than an icon-set import: the kit takes no dependency on lucide,
 * and these are the only three glyphs it needs. The mock imported Check and X
 * from it. (#320)
 */
const CheckGlyph = () => (
  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" data-glyph="check">
    <path d="m5 13 4 4L19 7" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const WarningGlyph = () => (
  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" data-glyph="warning">
    <path d="M12 8v5m0 4h.01" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
  </svg>
);

const InfoGlyph = () => (
  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" aria-hidden="true" data-glyph="info">
    <path d="M12 11v6m0-10h.01" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
  </svg>
);

/** Shaped for {@link IconButton}, which sizes it and hides it from assistive technology. */
const DismissGlyph: IconComponent = ({ size = 14, className, "aria-hidden": ariaHidden }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} aria-hidden={ariaHidden}>
    <path d="M18 6 6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

export interface ToastProps {
  /** What happened, in one line. */
  title: ReactNode;
  /** The detail under it — which object, which context. */
  hint?: ReactNode;
  tone?: Tone;
  /** Shown only when given: a toast the app dismisses on a timer needs no control. */
  onClose?: () => void;
  /** Names the dismiss button apart when several toasts share a screen. */
  dismissLabel?: string;
  className?: string;
}

/**
 * The card a transient message is drawn in: a tone disc, a line saying what
 * happened, an optional line of detail, and a way to put it away.
 *
 * Presentation only, and deliberately. The app already installs a `sonner` sink
 * through its own `notify.ts` and goes on doing so — the kit owns what a toast
 * looks like, the app owns when one appears. So there is no toast library
 * behind this, no queue, no timer, no portal and no global state: it renders a
 * toast it is handed, once, where it is mounted. Anyone reaching for a
 * `toast.success()` here wants the app's sink, not this file. (#320)
 *
 * Which live region it lands in follows the tone, the way {@link Alert} decides
 * it: `sev` is an `alert`, assertive, cutting across whatever is being spoken,
 * because a failed scale is worth interrupting for; everything else is a
 * `status` and waits its turn. Nothing sets `aria-live` on top of the role,
 * which would be the same instruction twice.
 *
 * Two things about the mock's version could not come along. Its disc drew a
 * tick whatever the tone was, so a failure appeared as a red circle with a tick
 * in it — the shape saying one thing and the colour another, and the colour
 * being the half a colour-blind reader loses. The glyph follows the tone now.
 * And its dismiss was a bare `<button>`, which inside a form is a submit
 * button; it goes through {@link IconButton}, where `type="button"` and the
 * accessible name are already settled.
 */
export function Toast({ title, hint, tone = "ok", onClose, dismissLabel = "Dismiss", className }: ToastProps) {
  const hasTitle = filled(title);
  const hasHint = filled(hint);
  // The message is built from state, and state can be empty on the first
  // render. An empty toast is a blank card floating over the app.
  if (!hasTitle && !hasHint) return null;

  const Glyph = tone === "ok" ? CheckGlyph : tone === "sev" || tone === "warn" ? WarningGlyph : InfoGlyph;

  return (
    <div
      role={tone === "sev" ? "alert" : "status"}
      data-tone={tone}
      className={cx("console-shell flex items-start gap-2 p-2.5", className)}
      style={{ maxWidth: 340 }}
    >
      <span
        data-slot="toast-mark"
        className="mt-px flex h-4 w-4 shrink-0 items-center justify-center rounded-full"
        // The ink over a solid tone, the same pairing Badge's `solid` uses;
        // the mock pinned it to `#fff`, which is one theme's answer.
        style={{ background: toneColor(tone), color: "var(--surface)" }}
      >
        <Glyph />
      </span>
      <div className="min-w-0 flex-1">
        {hasTitle && (
          <div data-slot="toast-title" className="text-[0.8125rem] font-medium">
            {title}
          </div>
        )}
        {hasHint && (
          <div
            data-slot="toast-hint"
            className={cx("text-[0.75rem] text-muted", hasTitle && "mt-0.5")}
          >
            {hint}
          </div>
        )}
      </div>
      {onClose && (
        <IconButton icon={DismissGlyph} label={dismissLabel} onClick={onClose} className="-mr-1 shrink-0" />
      )}
    </div>
  );
}
