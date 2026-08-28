import React from "react";
import { cn } from "@/ui/utils";

export type StatusKind = "success" | "warning" | "danger" | "info" | "neutral";

const DOT: Record<StatusKind, string> = {
  success: "bg-emerald-500",
  warning: "bg-amber-500",
  danger: "bg-destructive",
  info: "bg-sky-500",
  neutral: "bg-muted-foreground",
};

export interface StatusPillProps {
  status: React.ReactNode;
  kind?: StatusKind;
}

/** A status indicator: a coloured dot followed by a label, theme-token driven. */
export function StatusPill({ status, kind = "neutral" }: StatusPillProps) {
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
      <span className={cn("size-2 shrink-0 rounded-[2px]", DOT[kind])} />
      {status}
    </span>
  );
}
