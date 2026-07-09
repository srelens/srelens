import React from "react";
import { SquareTerminal } from "lucide-react";
import { ClusterUsage } from "./ClusterUsage";
import { ForwardsIndicator } from "./ForwardsIndicator";

/**
 * Thin status bar across the bottom of the app: connection state + active
 * cluster on the left; a terminal launcher, live cluster CPU/memory, the
 * current view, and the open-tab count on the right.
 */
export function StatusBar({
  activeCluster,
  activeLabel,
  tabCount,
  onOpenTerminal,
}: {
  activeCluster: string | null;
  activeLabel?: string;
  tabCount: number;
  /** Open a local kubectl terminal for the active cluster (omitted when disconnected). */
  onOpenTerminal?: () => void;
}) {
  return (
    <footer className="fl-statusbar col-[1/-1] flex h-6 items-center gap-3 border-t border-border bg-card px-3 text-xs text-muted-foreground">
      {activeCluster ? (
        <span className="fl-statusbar__cluster flex items-center gap-1.5">
          <span className="fl-statusbar__dot size-2 rounded-full bg-emerald-500" />
          <span className="truncate font-medium text-foreground">{activeCluster}</span>
        </span>
      ) : (
        <span className="fl-statusbar__cluster flex items-center gap-1.5">
          <span className="fl-statusbar__dot fl-statusbar__dot--muted size-2 rounded-full bg-muted-foreground/50" />
          Not connected
        </span>
      )}
      {activeLabel && <span className="fl-statusbar__label truncate">{activeLabel}</span>}

      <span className="fl-statusbar__meta ml-auto flex items-center gap-3">
        {onOpenTerminal && (
          <button
            type="button"
            className="fl-statusbar__terminal flex items-center gap-1 hover:text-foreground"
            onClick={onOpenTerminal}
            title={`Open a kubectl terminal for ${activeCluster}`}
            aria-label="Open kubectl terminal"
          >
            <SquareTerminal className="size-3.5" aria-hidden="true" />
            Terminal
          </button>
        )}
        <ForwardsIndicator />
        {activeCluster && <ClusterUsage context={activeCluster} />}
        <span className="tabular-nums">
          {tabCount} {tabCount === 1 ? "tab" : "tabs"}
        </span>
        <span className="opacity-70">srelens · Tauri</span>
      </span>
    </footer>
  );
}
