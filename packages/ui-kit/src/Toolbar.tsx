import type { ReactNode } from "react";
import { cx } from "./cx";

export interface ToolbarProps {
  children: ReactNode;
  /**
   * Let the controls run onto further rows when the strip is too narrow for
   * them, rather than squeezing them. Off by default: Screen's head truncates
   * its title instead.
   */
  wrap?: boolean;
  className?: string;
}

/**
 * A horizontal strip of controls above the content it acts on — filters, a
 * search box, a refresh button.
 *
 * The same chrome {@link Screen} puts at the top of a page, exposed on its own
 * for the strips that sit further down: a fixed-height flex row, ruled off from
 * what follows it. Layout of the contents is the caller's, since a toolbar of
 * filters and a toolbar of buttons space themselves differently. (#318)
 */
export function Toolbar({ children, wrap = false, className }: ToolbarProps) {
  return <div className={cx("toolbar", wrap && "flex-wrap gap-y-1.5 py-1", className)}>{children}</div>;
}
