import type { IconComponent } from "./IconButton";
import { cx } from "./cx";

export interface NavIconProps {
  icon: IconComponent;
  className?: string;
}

/**
 * The small monochrome icon in a navigation row.
 *
 * Decorative by construction: every row it appears in carries its own label, so
 * the icon is hidden from assistive technology rather than named. It takes no
 * colour of its own either — the row it sits in tints it, so hover and the
 * active state reach the icon without this component knowing about them.
 *
 * Which icon stands for which resource is the app's business, not the kit's:
 * this renders whatever it is handed. (#318)
 */
export function NavIcon({ icon: Icon, className }: NavIconProps) {
  return <Icon size={13} className={cx("shrink-0", className)} aria-hidden="true" />;
}
