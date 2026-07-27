import React, { useSyncExternalStore } from "react";
import { ArrowLeftRight, CircleStop, Copy } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";
import {
  getForwards,
  subscribeForwards,
  stopPortForward,
  forwardAddress,
  type ActiveForward,
} from "../lib/forward";

/** Subscribe to the active port-forwards store. */
export function useForwards(): ActiveForward[] {
  return useSyncExternalStore(subscribeForwards, getForwards, getForwards);
}

/**
 * Status-bar control listing active port-forwards. Hidden when none are
 * running; otherwise a count opens a popover to copy the forward's address or
 * stop each forward.
 */
export function ForwardsIndicator() {
  const forwards = useForwards();
  if (forwards.length === 0) return null;

  return (
    <Popover>
      <PopoverTrigger
        className="flex items-center gap-1 rounded-sm px-1 text-foreground hover:bg-accent"
        aria-label={`${forwards.length} active port forwards`}
      >
        <ArrowLeftRight className="fl-statusbar__icon" aria-hidden="true" />
        <span className="tabular-nums">{forwards.length}</span>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-0">
        <div className="border-b border-border px-3 py-2 text-xs font-medium text-muted-foreground">
          Port forwards
        </div>
        <ul className="max-h-72 overflow-auto py-1">
          {forwards.map((f) => (
            <li key={f.id} className="flex items-center gap-2 px-3 py-1.5 text-xs">
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium text-foreground">
                  {f.name}
                  <span className="text-muted-foreground"> · {f.kind.toLowerCase()}</span>
                </div>
                <div className="truncate font-mono text-muted-foreground">
                  {forwardAddress(f)} → {f.remotePort}
                </div>
              </div>
              <button
                type="button"
                className="fl-forward-action rounded-sm px-1.5 py-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                onClick={() => void navigator.clipboard?.writeText(forwardAddress(f))}
                title="Copy address"
              >
                <Copy aria-hidden="true" />
                Copy
              </button>
              <button
                type="button"
                className="fl-forward-action rounded-sm px-1.5 py-0.5 text-destructive hover:bg-destructive/10"
                onClick={() => void stopPortForward(f.id)}
                title="Stop forward"
              >
                <CircleStop aria-hidden="true" />
                Stop
              </button>
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  );
}
