import type { ReactNode } from "react";
import { cx } from "./cx";
import { IconButton, type IconComponent } from "./IconButton";
import { filled } from "./slot";
import { toneColor, toneWash, type Tone } from "./tone";

export interface AlertProps {
  tone?: Tone;
  title: ReactNode;
  /** The line under the title saying what to do about it. */
  children?: ReactNode;
  onDismiss?: () => void;
  /** Names the dismiss button when several alerts share a screen. */
  dismissLabel?: string;
  className?: string;
}

/*
 * Inline rather than an icon-set import: the kit takes no dependency on lucide,
 * and these are the only two glyphs it needs. The mock imported AlertTriangle,
 * Info and X from it. (#320)
 */
const WarningGlyph = () => (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    aria-hidden="true"
    data-glyph="warning"
  >
    <path
      d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinejoin="round"
    />
    <path d="M12 9v4m0 4h.01" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

const InfoGlyph = () => (
  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true" data-glyph="info">
    <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2" />
    <path d="M12 11v5m0-8h.01" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

/** Shaped for {@link IconButton}, which sizes it and hides it from assistive technology. */
const DismissGlyph: IconComponent = ({ size = 14, className, "aria-hidden": ariaHidden }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    className={className}
    aria-hidden={ariaHidden}
  >
    <path d="M18 6 6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

/**
 * A message about the thing on screen: a title, an optional line of detail, and
 * a way to put it away.
 *
 * Which live region it is follows the tone, because that is the only difference
 * that matters to someone who cannot see it appear. `sev` is `alert`, which is
 * assertive and cuts across whatever is being spoken; everything else is
 * `status`, which waits. A cordoned node is worth saying and not worth
 * interrupting for, and a component that shouted every time would train people
 * to turn it off. Worth knowing that both roles announce reliably only when the
 * region is already in the document before the message goes into it — an alert
 * mounted with the view it belongs to may be read as ordinary text.
 *
 * The dismiss button says `type="button"`. The kit's {@link Button}
 * deliberately leaves `type` alone (bd24d1a), so a bare button inside a form is
 * a submit button, and the mock's dismiss was one: closing a warning above a
 * form submitted the form. It is this component's button, not the caller's, so
 * the type is this component's to set — which is why it goes through
 * {@link IconButton}, where that is already settled, at the house hit size
 * rather than the mock's 16px. (#320)
 *
 * The title is a `strong`, not a heading. An alert is a remark about the
 * content next to it rather than a section of the page, it turns up at any
 * depth, and a fixed level would land wherever it was dropped;
 * {@link ErrorState} answered the same question the same way. (#320)
 */
export function Alert({
  tone = "info",
  title,
  children,
  onDismiss,
  dismissLabel = "Dismiss",
  className,
}: AlertProps) {
  const Glyph = tone === "sev" || tone === "warn" ? WarningGlyph : InfoGlyph;
  return (
    <div
      role={tone === "sev" ? "alert" : "status"}
      data-tone={tone}
      className={cx("flex items-start gap-2 rounded-md border p-2", className)}
      style={{
        // The wash Badge uses, so a severity tints the same wherever it turns
        // up; the border is that tone thinned over whatever it sits on.
        borderColor: `color-mix(in srgb, ${toneColor(tone)} 40%, transparent)`,
        background: toneWash(tone),
      }}
    >
      <span className="mt-px shrink-0" style={{ color: toneColor(tone) }}>
        <Glyph />
      </span>
      <div className="min-w-0 flex-1">
        <strong className="block text-[0.8125rem] font-medium">{title}</strong>
        {filled(children) && (
          <div data-slot="alert-body" className="mt-0.5 text-[0.75rem] leading-relaxed text-muted">
            {children}
          </div>
        )}
      </div>
      {onDismiss && (
        <IconButton
          icon={DismissGlyph}
          label={dismissLabel}
          onClick={onDismiss}
          className="-mt-px shrink-0"
        />
      )}
    </div>
  );
}
