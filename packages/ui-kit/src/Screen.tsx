import type { ReactNode } from "react";
import { Toolbar } from "./Toolbar";
import { cx } from "./cx";
import { filled } from "./slot";

export interface ScreenProps {
  title: ReactNode;
  /** Small capitalised context above the title in the classic design, before it here. */
  eyebrow?: ReactNode;
  /** A sentence introducing the page, shown at the top of the body. */
  description?: ReactNode;
  /** Controls pinned to the right of the title bar. */
  actions?: ReactNode;
  /** Let the body fill instead of scroll, for content that scrolls itself. */
  fill?: boolean;
  children: ReactNode;
  className?: string;
}

/**
 * A full page: a title bar across the top, the content beneath it.
 *
 * This is the classic `PageShell` and `PageHeader` as one component. They were
 * never used apart — a shell always opened with a header — and the mock models
 * the pair as a single `Screen`, so keeping two names would preserve a seam
 * that only ever described how the markup was split. (#318)
 *
 * The title is an `h1`, which the mock's version is not. The mock draws it as a
 * styled span, and that costs the document outline: a screen-reader user
 * navigating by heading finds nothing on any page in the app. The look is the
 * mock's, the semantics are the classic header's.
 *
 * The description sits in the body rather than the bar. The bar is a
 * fixed-height strip sized for controls, and a sentence does not fit in it —
 * it also belongs to the content rather than the chrome.
 */
export function Screen({
  title,
  eyebrow,
  description,
  actions,
  fill = false,
  children,
  className,
}: ScreenProps) {
  return (
    <div className={cx("flex h-full min-h-0 flex-col", className)}>
      <Toolbar>
        {filled(eyebrow) && <span className="crumb shrink-0">{eyebrow}</span>}
        <h1 className="toolbar-title min-w-0 truncate">{title}</h1>
        {/* Pushes the actions to the far end without either side needing to
            know how wide the other is. */}
        <span className="flex-1" />
        {filled(actions) && (
          <div data-slot="screen-actions" className="flex shrink-0 items-center gap-1.5">
            {actions}
          </div>
        )}
      </Toolbar>
      <div className={fill ? "flex min-h-0 flex-1 flex-col" : "scroll min-h-0 flex-1 p-3"}>
        {filled(description) && (
          <p className="mb-3 text-[0.8125rem] text-muted">{description}</p>
        )}
        {children}
      </div>
    </div>
  );
}
