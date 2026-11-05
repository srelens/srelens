import { useCallback, useEffect, useRef, useState } from "react";
import {
  mcpHttpSnapshot,
  relativeTime,
  type McpHttpSnapshot,
} from "@srelens/core";
import { Badge, Button, LoadingState, Panel } from "@srelens/ui-kit";
import { FailureAlert } from "../../lib/errorCopy";

const REFRESH_MS = 5_000;

/**
 * The privacy-limited local view of active Streamable HTTP sessions.
 * `null` means the parent has not established whether the server is running;
 * it must not be collapsed into the definite claim that the server is stopped.
 */
interface McpConnectedClientsProps {
  running: boolean | null;
  serverUrl?: string | null;
  /** See the classic list: the ref fences reads before effect cleanup. */
  actionGeneration: number;
  actionGenerationRef: Readonly<{ current: number }>;
  onServerSnapshot?: (snapshot: McpHttpSnapshot) => void;
}

export function McpConnectedClients({
  running,
  serverUrl,
  actionGeneration,
  actionGenerationRef,
  onServerSnapshot,
}: McpConnectedClientsProps) {
  const [snapshot, setSnapshot] = useState<McpHttpSnapshot | null>(null);
  const [error, setError] = useState<unknown>(null);
  const requestGeneration = useRef(0);
  const onServerSnapshotRef = useRef(onServerSnapshot);

  useEffect(() => {
    onServerSnapshotRef.current = onServerSnapshot;
  }, [onServerSnapshot]);

  const refresh = useCallback(async () => {
    const generation = ++requestGeneration.current;
    const parentGeneration = actionGenerationRef.current;
    try {
      const next = await mcpHttpSnapshot();
      if (
        generation !== requestGeneration.current ||
        parentGeneration !== actionGenerationRef.current
      ) {
        return;
      }
      setSnapshot(next);
      setError(null);
      onServerSnapshotRef.current?.(next);
    } catch (nextError) {
      if (
        generation !== requestGeneration.current ||
        parentGeneration !== actionGenerationRef.current
      ) {
        return;
      }
      setSnapshot(null);
      setError(nextError);
    }
  }, [actionGenerationRef]);

  useEffect(() => {
    requestGeneration.current += 1;
    setSnapshot(null);
    setError(null);
    void refresh();
    const timer = window.setInterval(() => void refresh(), REFRESH_MS);
    return () => {
      requestGeneration.current += 1;
      window.clearInterval(timer);
    };
  }, [actionGeneration, refresh, running, serverUrl]);

  const effectiveRunning = snapshot?.running ?? running;
  const clients = snapshot?.clients ?? null;

  return (
    <Panel
      title="HTTP client sessions"
      description="Sessions held by this process's Streamable HTTP server, most recently used first."
    >
      <div className="flex flex-wrap items-start gap-2">
        <p className="min-w-0 flex-1 text-[0.75rem] leading-relaxed text-muted">
          Names and versions are self-reported during each client&apos;s initialize handshake. This
          view keeps no network addresses, credentials, or session IDs. A row is a server session,
          not proof that the client process is still running.
        </p>
        {effectiveRunning === true && error === null && clients !== null && (
          <Button variant="ghost" size="xs" onClick={() => void refresh()}>
            Refresh
          </Button>
        )}
      </div>

      {error !== null ? (
        <FailureAlert
          className="mt-3"
          title="HTTP client sessions could not be read"
          error={error}
          domain="local"
          onRetry={() => void refresh()}
        />
      ) : effectiveRunning === null ? (
        <p className="mt-3 text-[0.75rem] text-muted">
          Active sessions appear once the server status is known.
        </p>
      ) : effectiveRunning === false ? (
        <p className="mt-3 text-[0.75rem] text-muted">The MCP HTTP server is stopped.</p>
      ) : clients === null ? (
        <LoadingState compact className="mt-3" label="Loading HTTP client sessions" />
      ) : clients.length === 0 ? (
        <p className="mt-3 text-[0.75rem] text-muted">No active HTTP client sessions.</p>
      ) : (
        <ul className="mt-2">
          {clients.map((client, index) => (
            <li
              key={`${client.name}\u0000${client.version ?? ""}\u0000${client.connectedAt}\u0000${index}`}
              className="grid min-w-0 gap-2 border-b border-rule py-2 last:border-b-0 md:grid-cols-[minmax(0,1fr)_auto] md:items-center"
            >
              <div className="min-w-0">
                <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
                  <span className="break-words text-[0.8125rem] font-medium text-ink">
                    {client.name}
                  </span>
                  {client.version && (
                    <code className="code break-all rounded px-1.5 py-0.5 text-[0.6875rem]">
                      {client.version}
                    </code>
                  )}
                </div>
                <p
                  className="mt-0.5 text-[0.75rem] text-muted"
                  title={`Session started ${new Date(client.connectedAt * 1000).toLocaleString()}`}
                >
                  Last request {relativeTime(client.lastSeen * 1000, Date.now())}
                </p>
              </div>
              <div className="justify-self-start md:justify-self-end">
                <Badge tone={client.notificationsConnected ? "ok" : "muted"}>
                  {client.notificationsConnected
                    ? "Notification stream open"
                    : "No notification stream"}
                </Badge>
              </div>
            </li>
          ))}
        </ul>
      )}
    </Panel>
  );
}
