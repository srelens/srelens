import type { ComponentType } from "react";
import { cx } from "./cx";

/**
 * Structural rather than lucide's `LucideIcon`, so the kit does not take a
 * dependency on an icon set to describe a hole an icon goes in. Lucide's icons
 * satisfy this, and so does a plain SVG component.
 */
export type IconComponent = ComponentType<{
  size?: number;
  className?: string;
  "aria-hidden"?: boolean | "true" | "false";
}>;

export interface IconButtonProps {
  icon: IconComponent;
  /** Accessible name + default tooltip (e.g. "Logs", "Delete"). */
  label: string;
  onClick?: () => void;
  /** Tints the icon with the danger colour (e.g. Delete). */
  danger?: boolean;
  disabled?: boolean;
  /** Tooltip override (defaults to `label`; used to explain a disabled reason). */
  title?: string;
  className?: string;
}

/**
 * A compact icon-only button.
 *
 * The label is both the accessible name and the hover tooltip unless `title`
 * overrides it, so the icon never stands alone — carried over from the classic
 * component along with the rest of its API. (#318)
 *
 * `type="button"` is explicit: these sit inside forms throughout the app, and a
 * delete icon that submits the form it is standing in loses work.
 */
export function IconButton({
  icon: Icon,
  label,
  onClick,
  danger,
  disabled,
  title,
  className,
}: IconButtonProps) {
  return (
    <button
      type="button"
      className={cx("icon-btn", className)}
      aria-label={label}
      title={title ?? label}
      onClick={onClick}
      disabled={disabled}
      style={danger ? { color: "var(--sev)" } : undefined}
    >
      <Icon size={14} aria-hidden="true" />
    </button>
  );
}
