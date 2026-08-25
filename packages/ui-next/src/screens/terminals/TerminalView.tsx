import { useLayoutEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { terminalFor } from "../../lib/sessions";

/**
 * The pane a live shell is drawn in — a pod exec, a node shell, or a local
 * terminal, whichever session is selected.
 *
 * The emulator itself is `terminalFor(sessionId)`, owned by `lib/sessions.ts`.
 * That store already wires the whole data path: `term.onData` reaches the
 * PTY, `term.onResize` reaches it too, and the backend's output is written
 * into the emulator as it arrives. This component's entire job is the DOM
 * side of that: attach the emulator to this pane, keep it fit to the pane's
 * size (which is what makes `onResize` fire with the right numbers), and let
 * go of it on unmount.
 *
 * **Never call `.dispose()` here.** The emulator, and its scrollback, belong
 * to the store — that is the whole reason a closed Terminals tab does not
 * lose the shell's history. Only `endSession` may dispose it.
 */
export function TerminalView({ sessionId }: { sessionId: number }) {
  const ref = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const term = terminalFor(sessionId);
    const container = ref.current;
    if (!term || !container) return undefined;

    if (term.element) {
      // Reattaching a session the reader left running: xterm's own `open()`
      // no-ops once an emulator already has an element (it only adjusts
      // which window it thinks it is in), so the existing node — with its
      // rendered scrollback — is moved into this pane by hand instead of
      // asking xterm to build a new one it would silently refuse to move.
      container.appendChild(term.element);
    } else {
      term.open(container);
    }

    const fit = new FitAddon();
    term.loadAddon(fit);
    // Sets the emulator's cols/rows from this pane's own size, which fires
    // `onResize` — already wired, by the store, to the PTY.
    fit.fit();

    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => fit.fit());
    observer?.observe(container);

    return () => {
      observer?.disconnect();
      // Unregisters the fit addon only — `FitAddon` has no state of its own
      // worth keeping across a remount, and leaving it loaded would pile a
      // fresh one onto the emulator every time this pane opens. The emulator
      // itself is untouched: no `term.dispose()`, ever, here.
      fit.dispose();
    };
  }, [sessionId]);

  return <div ref={ref} className="min-h-0 flex-1" />;
}
