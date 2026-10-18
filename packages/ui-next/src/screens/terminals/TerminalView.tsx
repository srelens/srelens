import { useLayoutEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { terminalFor } from "../../lib/sessions";
import { dressTerminal, useTerminalDress } from "../../lib/terminalDress";

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
 * **One session, more than one pane.** A session can be on screen in a page's
 * terminal panel and in the Terminals tab (#942), and an emulator has one
 * element, so it can only be in one of them at a time. Whichever pane is
 * showing takes it: a pane that comes back into view (its tab selected again)
 * moves the element back into itself before it fits. A pane with no size is
 * hidden, and leaves it where it is.
 *
 * Every pane dresses the emulator in the app's tokens — colours, the
 * monospace family and size — so a shell reads the same wherever it is.
 *
 * **Never call `.dispose()` here.** The emulator, and its scrollback, belong
 * to the store — that is the whole reason a closed Terminals tab does not
 * lose the shell's history. Only `endSession` may dispose it.
 */
export function TerminalView({ sessionId }: { sessionId: number }) {
  const ref = useRef<HTMLDivElement>(null);
  useTerminalDress(sessionId);

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

    // Before the fit: the font size decides how many columns fit.
    dressTerminal(term);
    const fit = new FitAddon();
    term.loadAddon(fit);
    // Sets the emulator's cols/rows from this pane's own size, which fires
    // `onResize` — already wired, by the store, to the PTY.
    fit.fit();

    // A size change is also how a pane learns it is showing again: a hidden
    // tab's pane measures nothing, and measures its own size once selected.
    const onResize = () => {
      if (container.clientWidth === 0 && container.clientHeight === 0) return;
      if (term.element && term.element.parentElement !== container) container.appendChild(term.element);
      fit.fit();
    };
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(onResize);
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

  // `min-w-0` for the same reason as `min-h-0`: this box holds an element
  // xterm sizes in explicit pixels, and a flex item's implicit
  // `min-width: auto` would refuse to shrink below it — widening the pane
  // instead of letting `FitAddon` narrow the terminal to fit.
  //
  // The inset is on xterm's own element, not on this box: `FitAddon` measures
  // this box's whole height and width and subtracts only the padding of the
  // element inside it, so padding here would fit a row and a column too many.
  return <div ref={ref} className="min-h-0 min-w-0 flex-1 [&>.xterm]:px-2 [&>.xterm]:py-1" />;
}
