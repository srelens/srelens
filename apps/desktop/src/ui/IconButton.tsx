import React from "react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
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
 * The tooltip is a Radix one, not the native `title`: the browser's tooltip
 * has a fixed ~1 s delay that made a toolbar feel unresponsive (#376), and
 * rendering both would show two. The delay lives in the app's root
 * `TooltipProvider`.
 */
export function IconButton({ icon, label, onClick, danger, disabled, title }: IconButtonProps) {
  const Icon = icon;
  const button = (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      aria-label={label}
      onClick={onClick}
      disabled={disabled}
      className={danger ? "text-destructive hover:text-destructive" : undefined}
    >
      <Icon aria-hidden="true" />
    </Button>
  );
  return (
    <Tooltip>
      {disabled ? (
        // A disabled <button> receives no pointer events, so it can't open a
        // tooltip itself and the disabled reason in `title` would be lost.
        // Wrap it and let the wrapper be the trigger.
        <TooltipTrigger asChild>
          <span className="inline-flex">{button}</span>
        </TooltipTrigger>
      ) : (
        <TooltipTrigger asChild>{button}</TooltipTrigger>
      )}
      <TooltipContent side="bottom" sideOffset={4}>
        {title ?? label}
      </TooltipContent>
    </Tooltip>
  );
}
