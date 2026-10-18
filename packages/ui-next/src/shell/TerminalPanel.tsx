import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { Alert, Badge, Button, IconButton, ResizeHandle, statusTone } from "@srelens/ui-kit";
import { Icons } from "../lib/icons";
import { endSession, getSessions, subscribeSessions, terminalFor } from "../lib/sessions";
import {
  closeTerminalPanel,
  forgetTerminalPanel,
  MIN_TERMINAL_PANEL_PX,
  requestTerminalsFocus,
  saveTerminalPanelShare,
  setTerminalPanelMaximized,
  setTerminalPanelMinimized,
  setTerminalPanelShare,
  useTerminalPanel,
  useTerminalPanelShare,
} from "../lib/terminalPanel";
import { openTab } from "../lib/tabsStore";
import { NOT_GATED, SESSION_VERDICT } from "../screens/terminals/SessionRail";
import { TerminalView } from "../screens/terminals/TerminalView";

/**
 * The page's own terminal, along its foot (#942).
 *
 * Drawn by `TabSurface` under the page it was opened on, so a shell started
 * from the Pods list sits under the Pods list. Half the page until the reader
 * drags its top edge; Maximize fills the page with it and leaves the page
 * mounted behind; Open in tab moves the same session — scrollback and all —
 * to the Terminals tab. Minimize folds it to its head, giving the page (and
 * an agent answer in the dock below) the room back while the shell runs on.
 * Closing it hides the panel and leaves the shell
 * running, listed in Terminals, where Detach ends it; Detach here does the
 * same.
 *
 * Laid out as the app's other panes are: a `.pane-head` with the session's
 * name and its verdict word, the transcript, and the same footer the
 * Terminals tab carries — the note that a shell is not gated, Clear, Detach.
 */
export function TerminalPanel({ tabId }: { tabId: string }) {
  const panel = useTerminalPanel(tabId);
  const sessions = useSyncExternalStore(subscribeSessions, getSessions, getSessions);
  const session = panel ? sessions.find((s) => s.id === panel.shellId) : undefined;
  const share = useTerminalPanelShare();
  const ref = useRef<HTMLDivElement>(null);
  // The height of the surface the panel shares, for the grip's pixel bounds.
  const [room, setRoom] = useState(0);

  // A session ended anywhere — Detach here, or in the Terminals tab — is no
  // longer this tab's to show or to remember.
  const gone = panel !== undefined && session === undefined;
  useEffect(() => {
    if (gone) forgetTerminalPanel(tabId);
  }, [gone, tabId]);

  const open = panel?.open === true && session !== undefined;
  useLayoutEffect(() => {
    const surface = ref.current?.parentElement;
    if (!open || !surface) return undefined;
    const measure = () => setRoom(surface.clientHeight);
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(surface);
    return () => observer.disconnect();
  }, [open]);

  if (!open || !panel || !session) return null;
  const verdict = SESSION_VERDICT[session.state];
  const maximized = panel.maximized;
  const minimized = panel.minimized;
  const maxHeight = Math.max(MIN_TERMINAL_PANEL_PX, room - MIN_TERMINAL_PANEL_PX);
  // The saved share, held to the grip's pixel bounds on this page: a share
  // kept from a tall window would otherwise leave a short one a panel with
  // no room for a line of transcript. The share itself is left as saved, so
  // the window growing again gives the reader back their size.
  const height = Math.min(maxHeight, Math.max(MIN_TERMINAL_PANEL_PX, Math.round(share * room)));
  const toShare = (px: number) => (room > 0 ? px / room : share);

  return (
    <section
      ref={ref}
      aria-label={`Terminal: ${session.title}`}
      data-slot="terminal-panel"
      data-maximized={maximized || undefined}
      data-minimized={minimized || undefined}
      className={
        maximized
          ? "relative flex min-h-0 min-w-0 flex-1 flex-col"
          : "relative flex min-h-0 min-w-0 shrink-0 flex-col border-t border-rule"
      }
      // Minimized is the head alone, at its own height.
      // A share until the page has been measured, then pixels within bounds.
      style={maximized || minimized ? undefined : { height: room > 0 ? height : `${share * 100}%` }}
    >
      {!maximized && !minimized && room > 0 && (
        <ResizeHandle
          label="the terminal"
          edge="top"
          width={height}
          minWidth={MIN_TERMINAL_PANEL_PX}
          maxWidth={maxHeight}
          onResize={(px) => setTerminalPanelShare(toShare(px))}
          onCommit={(px) => saveTerminalPanelShare(toShare(px))}
        />
      )}
      <div className="pane-head">
        <Icons.terminal size={12} aria-hidden />
        <span>Terminal</span>
        {/* Normal case, as the Terminals tab's head has it: a context and a
            pod name are identifiers, and uppercasing one makes it a different
            string on sight. */}
        <span
          data-slot="session-name"
          className="min-w-0 truncate whitespace-nowrap font-mono normal-case tracking-normal text-[0.75rem] font-normal text-ink"
        >
          {session.title} · {session.context}
        </span>
        <span className="flex-1" />
        <Badge tone={statusTone(verdict.kind)}>{verdict.word}</Badge>
        <IconButton
          icon={minimized ? Icons.raisePanel : Icons.minimizePanel}
          label={minimized ? "Restore terminal" : "Minimize terminal"}
          title={minimized ? "Restore" : "Minimize — the shell keeps running"}
          aria-expanded={!minimized}
          onClick={() => setTerminalPanelMinimized(tabId, !minimized)}
        />
        <IconButton
          icon={maximized ? Icons.restorePanel : Icons.maximizePanel}
          label={maximized ? "Restore terminal size" : "Maximize terminal"}
          title={maximized ? "Restore" : "Maximize"}
          onClick={() => setTerminalPanelMaximized(tabId, !maximized)}
        />
        <IconButton
          icon={Icons.openTab}
          label="Open terminal in a tab"
          title="Open in tab"
          onClick={() => {
            forgetTerminalPanel(tabId);
            requestTerminalsFocus(session.id);
            openTab("/terminals", { clusterName: session.context });
          }}
        />
        <IconButton
          icon={Icons.close}
          label="Close terminal panel"
          title="Close — the shell keeps running"
          onClick={() => closeTerminalPanel(tabId)}
        />
      </div>
      {/* Hidden, not unmounted, while minimized: the pane takes its emulator
          back and fits it when it has a size again. */}
      <div data-slot="terminal-panel-body" hidden={minimized} className="flex min-h-0 min-w-0 flex-1 flex-col">
      {session.error && (
        <Alert tone="sev" title="This session ended" className="m-3 mb-0">
          {session.error}
        </Alert>
      )}
      <TerminalView key={session.id} sessionId={session.id} />
      <div className="flex shrink-0 items-center gap-2 border-t border-rule bg-[var(--ground-sunk)] px-2.5 py-1.5">
        <span className="min-w-0 truncate text-[0.75rem] text-muted">{NOT_GATED}</span>
        <span className="flex-1" />
        <Button variant="secondary" size="xs" onClick={() => terminalFor(session.id)?.clear()}>
          Clear
        </Button>
        <Button variant="danger" size="xs" onClick={() => endSession(session.id)}>
          Detach
        </Button>
      </div>
      </div>
    </section>
  );
}

/**
 * A tab's page with its terminal panel under it. While the panel is maximized
 * the page is hidden, not unmounted: its scroll, selection and watches are
 * where they were when the panel is restored.
 */
export function PageWithTerminal({ tabId, children }: { tabId: string; children: ReactNode }) {
  const panel = useTerminalPanel(tabId);
  const sessions = useSyncExternalStore(subscribeSessions, getSessions, getSessions);
  const maximized = panel?.open === true && panel.maximized && sessions.some((s) => s.id === panel.shellId);
  return (
    <>
      <div data-slot="tab-page" hidden={maximized} className="flex min-h-0 min-w-0 flex-1 flex-col">
        {children}
      </div>
      <TerminalPanel tabId={tabId} />
    </>
  );
}
