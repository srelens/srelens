import React from "react";
import { Button } from "@/components/ui/button";
import type { LucideIcon } from "lucide-react";

export interface IconButtonProps {
  icon: LucideIcon;
  /** Accessible name + default tooltip (e.g. "Logs", "Delete"). */
  label: string;
  onClick?: () => void;
  /** Tints the icon with the danger colour (e.g. Delete). */
  danger?: boolean;
  disabled?: boolean;
  /** Tooltip override (defaults to `label`; used to explain a disabled reason). */
  title?: string;
}

/**
 * A compact icon-only button (shadcn Button, ghost). The label is both the
 * accessible name and the hover tooltip (unless `title` overrides it), so the
 * icon never stands alone.
 *
 * The tooltip is `Button`'s own `title` handling — a Radix tooltip, not the
 * native attribute with its fixed ~1 s delay (#376) — so a disabled button's
 * reason stays reachable and there is one mechanism for every button.
 */
export function IconButton({ icon, label, onClick, danger, disabled, title }: IconButtonProps) {
  const Icon = icon;
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      aria-label={label}
      title={title ?? label}
      onClick={onClick}
      disabled={disabled}
      className={danger ? "text-destructive hover:text-destructive" : undefined}
    >
      <Icon aria-hidden="true" />
    </Button>
  );
}
