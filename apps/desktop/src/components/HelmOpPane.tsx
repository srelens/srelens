import { useEffect, useRef, useState } from "react";
import { startHelmOp } from "@srelens/core";
import type { DockSession } from "./Dock";

export function HelmOpPane({ session }: { session: DockSession }) {
  const [lines, setLines] = useState<string[]>([]);
  const [status, setStatus] = useState<"running" | "done" | "error">("running");
  const [error, setError] = useState<string | null>(null);
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const args = session.helm?.args ?? [];
    let cancelled = false;
    let handle: { close: () => void } | null = null;
    void startHelmOp(
      session.context,
      args,
      (line) => setLines((prev) => [...prev, line]),
      (err) => {
        setStatus(err ? "error" : "done");
        setError(err);
        if (!err) session.helm?.onComplete?.();
      },
      session.kubeconfigFiles ?? [],
      session.helm?.values ?? "",
    )
      .then((h) => {
        if (cancelled) h.close();
        else handle = h;
      })
      .catch((e) => {
        setStatus("error");
        setError(String(e));
      });
    return () => {
      cancelled = true;
      handle?.close();
    };
  }, [session]);

  return (
    <div className="fl-helm-pane" role="log" aria-label={session.helm?.title ?? "Helm operation"}>
      <pre className="fl-helm-pane__out">{lines.join("\n")}</pre>
      <div className="fl-helm-pane__status">
        {status === "running" && <span>Running…</span>}
        {status === "done" && <span className="fl-ok">Completed</span>}
        {status === "error" && <span className="fl-err">Failed{error ? `: ${error}` : ""}</span>}
      </div>
    </div>
  );
}
