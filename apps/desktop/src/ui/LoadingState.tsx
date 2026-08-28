import React from "react";
import { Spinner } from "./Spinner";
import { cn } from "@/ui/utils";

export interface LoadingStateProps {
  /** Text shown beneath the spinner; also the spinner's accessible label. */
  label?: string;
  className?: string;
}

/**
 * Prominent, centered loading placeholder for a content area whose data is
 * still being fetched. Wraps {@link Spinner} with a caption so an in-flight
 * load reads clearly instead of looking like an empty result.
 */
export function LoadingState({ label = "Loading", className }: LoadingStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 py-16 text-muted-foreground",
        className,
      )}
    >
      <Spinner label={label} className="size-8 text-primary" />
      <span className="text-sm">{label}</span>
    </div>
  );
}
