import { mcpConnectedClients, type ConnectedMcpClient } from "@srelens/core/lib/mcp";
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
export function McpClientsList({ running }: { running: boolean }) {
  const [clients, setClients] = useState<ConnectedMcpClient[] | null>(null);
  const [error, setError] = useState<unknown>(null);
  const requestGeneration = useRef(0);

  const refresh = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const next = await mcpConnectedClients();
      if (generation !== requestGeneration.current) return;
      setClients(next);
      setError(null);
    } catch (nextError) {
      if (generation !== requestGeneration.current) return;
      setClients(null);
      setError(nextError);
    }
  }, []);

  useEffect(() => {
    if (!running) {
      requestGeneration.current += 1;
      setClients([]);
      setError(null);
      return;
    }

    setClients(null);
    setError(null);
    void refresh();
    const timer = window.setInterval(() => void refresh(), REFRESH_MS);
    return () => {
      requestGeneration.current += 1;
      window.clearInterval(timer);
    };
  }, [refresh, running]);

  if (!running) {
    return <p className="text-sm text-muted-foreground">The MCP HTTP server is stopped.</p>;
  }

  if (error !== null) {
    return (
      <ErrorState
        title="Active MCP clients could not be read"
        detail={describeError(error, { domain: "local" }).detail}
        onRetry={() => void refresh()}
      />
    );
  }

  if (clients === null) {
    return <Spinner label="Loading active MCP clients" />;
  }

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
          aria-label="Refresh active MCP clients"
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
                  title={`Connected ${new Date(client.connectedAt * 1000).toLocaleString()}`}
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
