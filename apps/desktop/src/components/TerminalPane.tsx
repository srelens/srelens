import React, { useEffect, useRef, useState } from "react";
import { Eraser, RotateCw, Search, X } from "lucide-react";
import { IconButton } from "../ui";
import type { TerminalConnection, TerminalDriver } from "@srelens/core";
import {
  type ExitReason,
  type TermStatus,
  nextStatusOnExit,
  sessionEarnedRetryReset,
  reconnectDelayMs,
} from "@srelens/core";

/**
 * Shared xterm terminal host for every terminal kind (in-pod exec, local PTY,
 * node shell). Owns the xterm instance, PTY resize, scrollback search, and the
 * reconnect state machine; the session itself comes from an injected driver.
 *
 * xterm and its CSS are dynamically imported so they stay out of the jsdom test
 * graph — this component is verified live against a cluster. The reconnect
 * *policy* it drives is pure and unit-tested in `lib/terminalReconnect`.
 */
export function TerminalPane({ driver, banner }: { driver: TerminalDriver; banner?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<TermStatus>({ kind: "connecting" });
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const searchInputRef = useRef<HTMLInputElement>(null);
  // Manual controls, wired once xterm has loaded.
  const controls = useRef({
    reconnect() {},
    clear() {},
    focus() {},
    searchNext(_q: string) {},
    searchPrev(_q: string) {},
  });

  useEffect(() => {
    let disposed = false;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let term: any = null;
    let conn: TerminalConnection | null = null;
    let attempt = 0;
    /**
     * When the current session first produced output — the earliest proof a
     * shell was really there. Null until it does (and for one that never does).
     */
    let usableSince: number | null = null;
    let reconnectTimer: number | undefined;
    let cleanupExtra: (() => void) | undefined;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let searchAddon: any = null;

    const divider = (label: string) => `\r\n\x1b[2m── ${label} ──\x1b[0m\r\n`;

    const handleExit = (reason: ExitReason) => {
      if (disposed || !term) return;
      conn = null;
      // A shell that ran and then dropped gets its full retry budget back; one
      // that never became usable keeps counting, so a shell that can never
      // start stops retrying and shows why (#263).
      const usable = usableSince === null ? 0 : Date.now() - usableSince;
      usableSince = null;
      if (sessionEarnedRetryReset(usable)) attempt = 0;
      const next = nextStatusOnExit(reason, driver.reconnectable, attempt);
      setStatus(next);
      if (next.kind === "reconnecting") {
        attempt = next.attempt;
        // Show the cause on every attempt, not only once the budget runs out:
        // if the shell can never start, the reason is the useful part and the
        // user should not have to wait through the backoff to read it.
        term.write(divider(`reconnecting… (attempt ${attempt})`));
        if (reason.kind === "error" && reason.message) {
          term.write(`\x1b[2m${reason.message}\x1b[0m\r\n`);
        }
        reconnectTimer = window.setTimeout(() => void connect(true), reconnectDelayMs(attempt) ?? 0);
      } else if (reason.kind === "error") {
        term.write(`${divider("disconnected")}${reason.message}\r\n`);
      } else {
        term.write(divider("session ended"));
      }
    };

    const connect = async (isReconnect: boolean) => {
      if (disposed || !term) return;
      setStatus(isReconnect ? { kind: "reconnecting", attempt: Math.max(attempt, 1) } : { kind: "connecting" });
      let opened: TerminalConnection;
      usableSince = null;
      try {
        opened = await driver.connect({
          onData: (chunk) => {
            // First byte back is when the session became a shell. The connect
            // call resolving is not: the backend spawns the attach and reports
            // its failure later, so timing from there would count the wait for
            // a refusal as uptime.
            usableSince ??= Date.now();
            term?.write(chunk);
          },
          onExit: handleExit,
          initialSize: { cols: term.cols, rows: term.rows },
        });
      } catch (cause) {
        handleExit({ kind: "error", message: cause instanceof Error ? cause.message : String(cause) });
        return;
      }
      if (disposed) {
        opened.close();
        return;
      }
      conn = opened;
      if (isReconnect) term.write(divider("reconnected (new shell)"));
      // NOT a retry-budget reset: connecting only means the session was
      // spawned. Whether it worked is decided by what comes back on the wire,
      // which onData records and handleExit judges.
      setStatus({ kind: "live" });
    };

    controls.current = {
      reconnect: () => {
        // An explicit reconnect is a fresh decision by the user, so it does
        // start the budget over.
        if (reconnectTimer) clearTimeout(reconnectTimer);
        attempt = 0;
        void connect(true);
      },
      clear: () => term?.clear(),
      focus: () => term?.focus(),
      searchNext: (q: string) => searchAddon?.findNext(q),
      searchPrev: (q: string) => searchAddon?.findPrevious(q),
    };

    void (async () => {
      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      const { SearchAddon } = await import("@xterm/addon-search");
      await import("@xterm/xterm/css/xterm.css");
      if (disposed || !ref.current) return;

      term = new Terminal({
        convertEol: true,
        fontSize: 13,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        scrollback: 10000,
        theme: { background: "#1b1f23", foreground: "#e6e6e6" },
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      searchAddon = new SearchAddon();
      term.loadAddon(searchAddon);
      term.open(ref.current);
      fit.fit();
      term.focus();
      if (banner) term.write(`${banner}\r\n`);

      // Wire input/resize once; they read the CURRENT connection (which swaps on
      // reconnect), so no handler churn.
      term.onData((data: string) => conn?.send(data));
      term.onResize(({ cols, rows }: { cols: number; rows: number }) => conn?.resize(cols, rows));

      const onWindowResize = () => {
        try {
          fit.fit();
        } catch {
          /* fit can throw mid-teardown; ignore */
        }
      };
      window.addEventListener("resize", onWindowResize);
      const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(onWindowResize) : null;
      observer?.observe(ref.current);
      cleanupExtra = () => {
        window.removeEventListener("resize", onWindowResize);
        observer?.disconnect();
      };

      void connect(false);
    })();

    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      cleanupExtra?.();
      conn?.close();
      term?.dispose();
    };
  }, [driver, banner]);

  const restartLabel = driver.kind === "local" ? "Restart shell" : "Reconnect";

  function onPaneKeyDown(e: React.KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      setSearchOpen(true);
      requestAnimationFrame(() => searchInputRef.current?.focus());
    }
  }

  function onSearchKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      if (e.shiftKey) controls.current.searchPrev(query);
      else controls.current.searchNext(query);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setSearchOpen(false);
      controls.current.focus();
    }
  }

  return (
    <div className="flex h-full flex-col bg-[#1b1f23]" onKeyDown={onPaneKeyDown}>
      <div className="flex shrink-0 items-center gap-2 border-b border-white/10 px-2 py-1 text-xs text-white/70">
        {status.kind === "connecting" && <span className="text-white/50">connecting…</span>}
        {status.kind === "live" && (
          <span className="flex items-center gap-1 text-emerald-400">
            <span className="size-1.5 rounded-full bg-emerald-500" />
            live
          </span>
        )}
        {status.kind === "reconnecting" && (
          <span className="text-amber-400">reconnecting… (attempt {status.attempt})</span>
        )}
        {status.kind === "disconnected" && (
          <span className="flex items-center gap-2">
            <span className="text-red-400">{status.reason ? `disconnected: ${status.reason}` : "disconnected"}</span>
            <button
              type="button"
              onClick={() => controls.current.reconnect()}
              className="inline-flex items-center gap-1 rounded bg-white/10 px-1.5 py-0.5 text-white/80 hover:bg-white/20"
            >
              <RotateCw aria-hidden="true" className="size-3" />
              {restartLabel}
            </button>
          </span>
        )}

        <div className="ml-auto flex items-center gap-1">
          <IconButton icon={Search} label="Search" onClick={() => setSearchOpen((o) => !o)} />
          <IconButton icon={Eraser} label="Clear" onClick={() => controls.current.clear()} />
          <IconButton
            icon={RotateCw}
            label={restartLabel}
            onClick={() => controls.current.reconnect()}
          />
        </div>
      </div>

      {searchOpen && (
        <div className="flex shrink-0 items-center gap-1 border-b border-white/10 bg-[#22262c] px-2 py-1">
          <Search aria-hidden="true" className="size-3.5 text-white/40" />
          <input
            ref={searchInputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onSearchKeyDown}
            placeholder="Find in terminal… (Enter / Shift+Enter)"
            aria-label="Find in terminal"
            className="min-w-0 flex-1 bg-transparent text-xs text-white/90 placeholder:text-white/30 focus:outline-none"
          />
          <IconButton icon={X} label="Close search" onClick={() => { setSearchOpen(false); controls.current.focus(); }} />
        </div>
      )}

      <div ref={ref} className="min-h-0 flex-1" style={{ padding: 6, boxSizing: "border-box" }} />
    </div>
  );
}
