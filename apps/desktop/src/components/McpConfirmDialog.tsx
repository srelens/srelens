import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { respondToConfirm, type ConfirmRequest } from "../lib/mcpSecurity";
import { notify } from "../lib/notify";
import { ConfirmDialog } from "../ui";

/**
 * Consent prompt for MCP tool calls. The Rust side blocks a destructive tool
 * call and emits `mcp://confirm-request`, raising/focusing the window first —
 * this listens app-wide (mounted once in App.tsx) since a request can arrive
 * from any view. Requests queue rather than replace one another: two agents
 * can call concurrently, and dropping one would hang that call until its
 * own timeout denies it.
 */
export function McpConfirmDialog() {
  const [queue, setQueue] = useState<ConfirmRequest[]>([]);
  const [busy, setBusy] = useState(false);
  const current = queue[0];

  useEffect(() => {
    const unlisten = listen<ConfirmRequest>("mcp://confirm-request", (event) => {
      setQueue((q) => [...q, event.payload]);
    });
    // The backend announces every resolution (answered here, answered on the
    // assistant's inline card, or timed out) — drop the request however it
    // settled, so this modal never lingers over an already-decided call.
    const unlistenResolved = listen<{ id: string }>("mcp://confirm-resolved", (event) => {
      setQueue((q) => q.filter((r) => r.id !== event.payload.id));
    });
    return () => {
      void unlisten.then((f) => f());
      void unlistenResolved.then((f) => f());
    };
  }, []);

  async function answer(approved: boolean) {
    if (!current) return;
    const { id } = current;
    setBusy(true);
    try {
      await respondToConfirm(id, approved);
    } catch (e) {
      // The request already timed out or was answered elsewhere (e.g. it
      // timed out server-side while queued behind another dialog) — the
      // user's click didn't actually take effect, and silently swallowing
      // this would let them believe they approved (or denied) a call that
      // was in fact already resolved without them. Surface it, then drop the
      // request and move to the next: there's nothing left here to retry.
      notify.error("Could not respond to that confirmation", String(e));
    } finally {
      setBusy(false);
      // Remove by id, not position: the `mcp://confirm-resolved` event may
      // have already removed this entry, and slicing blindly would then drop
      // the NEXT queued request instead.
      setQueue((q) => q.filter((r) => r.id !== id));
    }
  }

  if (!current) return null;

  return (
    <ConfirmDialog
      title="An agent wants to run a cluster action"
      message={
        <div className="flex flex-col gap-2">
          <p className="m-0">
            Tool: <code>{current.tool}</code>
          </p>
          <pre className="max-h-64 overflow-auto rounded-md border border-border bg-muted/40 p-3 text-xs">
            <code>{JSON.stringify(current.args, null, 2)}</code>
          </pre>
          {queue.length > 1 && (
            <p className="m-0 text-xs text-muted-foreground">
              {queue.length - 1} more request{queue.length - 1 === 1 ? "" : "s"} waiting
            </p>
          )}
        </div>
      }
      confirmLabel="Approve"
      cancelLabel="Deny"
      danger
      busy={busy}
      onConfirm={() => void answer(true)}
      onCancel={() => void answer(false)}
    />
  );
}
