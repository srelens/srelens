import React from "react";
import { cn } from "@/ui/utils";

export interface SpinnerProps extends Omit<React.ComponentProps<"svg">, "aria-label"> {
  /** Accessible label; defaults to "Loading". */
  label?: string;
}

/**
 * Indeterminate loading spinner: a muted track ring with a spinning accent arc.
 * Inherits the current text colour so it blends wherever it sits inline with a
 * label, and takes `className` (e.g. `size-8 text-primary`) to scale/recolour.
 */
export function Spinner({ label = "Loading", className, ...props }: SpinnerProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      role="status"
      aria-label={label}
      className={cn("size-4 animate-spin", className)}
      {...props}
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2.5" className="opacity-20" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
    </svg>
  );
}
