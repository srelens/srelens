import type { ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface PaneIntroProps {
  title: ReactNode;
  /** What the pane holds, under its name. */
  description?: ReactNode;
  /** Actions on the whole pane, beside its name. */
  actions?: ReactNode;
  className?: string;
}

/**
 * The band that opens a pane made of ruled bands rather than cards — a
 * settings pane such as Skills or Workspaces: its name, a line on what it
 * holds, and the actions that apply to all of it.
 *
 * In a Panel head's voice (13px semibold title, the muted caption under it,
 * the same inset) without the card, since the pane is already the surface.
 */
export function PaneIntro({ title, description, actions, className }: PaneIntroProps) {
  return (
    <div className={cx("flex flex-wrap items-center gap-x-6 gap-y-2 border-b border-rule px-3 py-2", className)}>
      <div className="min-w-0 flex-1">
        <h2 className="text-[0.8125rem] font-semibold text-ink">{title}</h2>
        {filled(description) && (
          <p className="mt-0.5 max-w-[70ch] text-[0.75rem] leading-relaxed text-muted">{description}</p>
        )}
      </div>
      {filled(actions) && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}
