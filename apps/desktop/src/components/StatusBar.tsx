import React, { useState } from "react";
import { SquareTerminal } from "lucide-react";
import { ClusterUsage } from "./ClusterUsage";
import { ForwardsIndicator } from "./ForwardsIndicator";
import { Popover, PopoverAnchor, PopoverContent } from "@/components/ui/popover";

/** A context a terminal can be opened for. */
export interface TerminalContext {
  /** The context name passed to the shell. */
  name: string;
  /** What to call it on screen (short name / profile, when set). */
  label: string;
}

/**
 * Thin status bar across the bottom of the app: connection state + active
 * cluster on the left; a terminal launcher, live cluster CPU/memory, the
 * current view, and the open-tab count on the right.
 */
export function StatusBar({
  activeCluster,
  activeLabel,
  tabCount,
  terminalContexts = [],
  onOpenTerminal,
}: {
  activeCluster: string | null;
  activeLabel?: string;
  tabCount: number;
  /** Contexts a terminal can be opened for; more than one offers a choice. */
  terminalContexts?: readonly TerminalContext[];
  /** Open a local kubectl terminal for a context (omitted in web mode). */
  onOpenTerminal?: (context: string) => void;
}) {
  const [picker, setPicker] = useState(false);
  // The launcher used to be bound to the active tab's cluster, which meant it
  // vanished on Settings/Toolbox/Assistant tabs and a shell for a second
  // cluster needed a tab opened for it first (#257). It now offers whatever is
  // configured, wherever you are.
  const showTerminal = !!onOpenTerminal && terminalContexts.length > 0;
  const preferred =
    terminalContexts.find((c) => c.name === activeCluster) ?? terminalContexts[0];
  // The context you are already in is the one you most often want a shell for,
  // so it leads regardless of where it sits in the hotbar — on a machine with
  // twenty kubeconfig contexts it would otherwise be somewhere down the scroll.
  const ordered = preferred
    ? [preferred, ...terminalContexts.filter((c) => c.name !== preferred.name)]
    : terminalContexts;

  function launch() {
    if (terminalContexts.length === 1) {
      onOpenTerminal?.(terminalContexts[0].name);
      return;
    }
    setPicker((open) => !open);
  }

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
        {showTerminal && (
          <Popover open={picker} onOpenChange={setPicker}>
            <PopoverAnchor asChild>
              <button
                type="button"
                className="fl-statusbar__terminal flex items-center gap-1 hover:text-foreground"
                onClick={launch}
                title={
                  terminalContexts.length === 1
                    ? `Open a kubectl terminal for ${terminalContexts[0].label}`
                    : "Open a kubectl terminal — choose a context"
                }
                aria-label="Open kubectl terminal"
              >
                <SquareTerminal className="size-3.5" aria-hidden="true" />
                Terminal
              </button>
            </PopoverAnchor>
            {/* Capped and scrollable: a kubeconfig with twenty contexts would
                otherwise open a menu taller than the window. */}
            <PopoverContent
              align="end"
              side="top"
              role="menu"
              className="max-h-64 w-auto min-w-40 max-w-80 gap-0 overflow-y-auto p-1 text-xs"
            >
              {ordered.map((c) => (
                <button
                  key={c.name}
                  type="button"
                  role="menuitem"
                  title={`Open a kubectl terminal for ${c.label}`}
                  className="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-accent hover:text-accent-foreground"
                  onClick={() => {
                    setPicker(false);
                    onOpenTerminal?.(c.name);
                  }}
                >
                  <span
                    aria-hidden="true"
                    className={`size-1.5 shrink-0 rounded-full ${
                      c.name === activeCluster ? "bg-emerald-500" : "bg-muted-foreground/40"
                    }`}
                  />
                  <span className="min-w-0 truncate">{c.label}</span>
                </button>
              ))}
            </PopoverContent>
          </Popover>
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
