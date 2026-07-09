import React, { useEffect, useRef } from "react";
import { startLocalTerminal, type TerminalSession } from "../lib/terminal";

/**
 * A local shell (the user's login shell) rendered with xterm, pre-scoped to a
 * kube context so `kubectl` targets it by default. Runs on the user's machine —
 * distinct from the in-pod exec terminal. xterm is loaded dynamically so it
 * stays out of the jsdom test graph; verified live rather than in unit tests.
 */
export function LocalTerminal({
  context,
  kubeconfigFiles,
}: {
  context: string;
  /** Extra kubeconfig files the app has been told about (merged for the shell). */
  kubeconfigFiles: string[];
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cleanup = () => {};
    let disposed = false;

    void (async () => {
      const { Terminal } = await import("@xterm/xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      await import("@xterm/xterm/css/xterm.css");
      if (disposed || !ref.current) return;

      const term = new Terminal({
        convertEol: false,
        fontSize: 13,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        theme: { background: "#1b1f23", foreground: "#e6e6e6" },
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      term.open(ref.current);
      fit.fit();
      term.focus();
      term.write(`kubectl scoped to \x1b[1m${context}\x1b[0m — starting shell…\r\n`);

      let session: TerminalSession | null = null;
      void startLocalTerminal(
        context,
        kubeconfigFiles,
        (chunk) => term.write(chunk),
        () => {
          term.write("\r\n[shell exited]\r\n");
          session?.close();
        },
        { cols: term.cols, rows: term.rows },
      ).then((s) => {
        if (disposed) {
          s.close();
          return;
        }
        session = s;
        term.onData((d) => s.send(d));
        term.onResize(({ cols, rows }) => s.resize(cols, rows));
      });

      // Keep the PTY sized to the panel as the dock is resized.
      const onWindowResize = () => {
        try {
          fit.fit();
        } catch {
          /* fit can throw if the element is detached mid-teardown */
        }
      };
      window.addEventListener("resize", onWindowResize);
      const observer =
        typeof ResizeObserver !== "undefined" ? new ResizeObserver(onWindowResize) : null;
      if (observer && ref.current) observer.observe(ref.current);

      cleanup = () => {
        window.removeEventListener("resize", onWindowResize);
        observer?.disconnect();
        session?.close();
        term.dispose();
      };
    })();

    return () => {
      disposed = true;
      cleanup();
    };
  }, [context, kubeconfigFiles]);

  return (
    <div
      ref={ref}
      style={{ height: "100%", width: "100%", background: "#000", padding: 6, boxSizing: "border-box" }}
    />
  );
}
