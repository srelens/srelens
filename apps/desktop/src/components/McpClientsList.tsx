import {
  mcpHttpSnapshot,
  type McpHttpSnapshot,
} from "@srelens/core/lib/mcp";
import { relativeTime } from "@srelens/core/lib/relativeTime";
import { describeError } from "@srelens/core";
import { Badge, Button, ErrorState, Spinner } from "../ui";
import { RefreshCw } from "lucide-react";
import { Fragment, useCallback, useEffect, useRef, useState } from "react";

const REFRESH_MS = 5_000;

/**
 * The active Streamable HTTP sessions owned by the in-app MCP server.
 * Identity is self-reported `clientInfo`; the host contributes only time and
 * stream state, and intentionally keeps credentials and network data out.
 */
interface McpClientsListProps {
  /** Last status known by the parent, or null while no status is established. */
  running: boolean | null;
  /** Invalidates an in-flight read when a parent action changes generations. */
  serverUrl?: string | null;
  /** Lets the parent use the same atomic read for its main server status. */
  onServerSnapshot?: (snapshot: McpHttpSnapshot) => void;
}

export function McpClientsList({ running, serverUrl, onServerSnapshot }: McpClientsListProps) {
  const [snapshot, setSnapshot] = useState<McpHttpSnapshot | null>(null);
  const [error, setError] = useState<unknown>(null);
  const requestGeneration = useRef(0);
  const onServerSnapshotRef = useRef(onServerSnapshot);

  useEffect(() => {
    onServerSnapshotRef.current = onServerSnapshot;
  }, [onServerSnapshot]);

  const refresh = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const next = await mcpHttpSnapshot();
      if (generation !== requestGeneration.current) return;
      setSnapshot(next);
      setError(null);
      onServerSnapshotRef.current?.(next);
    } catch (nextError) {
      if (generation !== requestGeneration.current) return;
      setSnapshot(null);
      setError(nextError);
    }
  }, []);

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
  }, [refresh, running, serverUrl]);

  if (error !== null) {
    return (
      <ErrorState
        title="HTTP client sessions could not be read"
        detail={describeError(error, { domain: "local" }).detail}
        onRetry={() => void refresh()}
      />
    );
  }

  const effectiveRunning = snapshot?.running ?? running;
  if (effectiveRunning === null) {
    return <Spinner label="Checking MCP HTTP server" />;
  }

  if (!effectiveRunning) {
    return <p className="text-sm text-muted-foreground">The MCP HTTP server is stopped.</p>;
  }

  if (snapshot === null) {
    return <Spinner label="Loading HTTP client sessions" />;
  }

  const clients = snapshot.clients;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-start gap-3">
        <p className="text-xs text-muted-foreground">
          Names and versions are self-reported during each client&apos;s initialize handshake. This
          list keeps no client network addresses or credentials. A row is a server session, not
          proof that the client process is still running.
        </p>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto shrink-0"
          onClick={() => void refresh()}
          aria-label="Refresh HTTP client sessions"
        >
          <RefreshCw data-icon="inline-start" />
          Refresh
        </Button>
      </div>

      {clients.length === 0 ? (
        <p className="text-sm text-muted-foreground">No active HTTP client sessions.</p>
      ) : (
        <dl className="fl-kv">
          {clients.map((client, index) => (
            <Fragment key={`${client.name}\u0000${client.version ?? ""}\u0000${client.connectedAt}\u0000${index}`}>
              <dt>{client.name}</dt>
              <dd className="flex flex-wrap items-center gap-x-2 gap-y-1">
                {client.version && <code className="fl-mono">{client.version}</code>}
                <span
                  className="text-xs text-muted-foreground"
                  title={`Session started ${new Date(client.connectedAt * 1000).toLocaleString()}`}
                >
                  Last request {relativeTime(client.lastSeen * 1000, Date.now())}
                </span>
                <Badge variant={client.notificationsConnected ? "success" : "neutral"}>
                  {client.notificationsConnected ? "Notification stream open" : "No notification stream"}
                </Badge>
              </dd>
            </Fragment>
          ))}
        </dl>
      )}
    </div>
  );
}
