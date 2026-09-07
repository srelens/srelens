import type { ReactNode } from "react";
import { cx } from "./cx";

export interface ToolbarProps {
  children: ReactNode;
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
export function Toolbar({ children, className }: ToolbarProps) {
  return <div className={cx("toolbar", className)}>{children}</div>;
}
