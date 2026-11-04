import { invokeCommand } from "../transport/transport";

/** Start (or restart) the in-app MCP HTTP server on `port`; returns its URL. */
export async function startMcpHttp(port: number): Promise<string> {
  return invokeCommand<string>("mcp_http_start", { port });
}

/** Stop the in-app MCP HTTP server. */
export async function stopMcpHttp(): Promise<void> {
  await invokeCommand("mcp_http_stop");
}

/** The MCP HTTP server's URL if it's running, else null. */
export async function mcpHttpStatus(): Promise<string | null> {
  return invokeCommand<string | null>("mcp_http_status");
}

/** One active Streamable HTTP session. Client identity is self-reported by
 * the initialize handshake; the host deliberately omits session IDs, tokens,
 * headers, and network addresses from this local Settings snapshot. */
export interface ConnectedMcpClient {
  name: string;
  version: string | null;
  connectedAt: number;
  lastSeen: number;
  notificationsConnected: boolean;
}

/** Active HTTP client sessions, most recently used first. */
export async function mcpConnectedClients(): Promise<ConnectedMcpClient[]> {
  return invokeCommand<ConnectedMcpClient[]>("mcp_connected_clients");
}

export interface CliStatus {
  installed: boolean;
  path: string;
  links_to: string | null;
  /** Whether the install directory is on the current $PATH. */
  on_path: boolean;
}

/** Symlink the srelens binary onto PATH; returns the install path. */
export async function installSrelensCli(): Promise<string> {
  return invokeCommand<string>("install_srelens_cli");
}

/** Whether the `srelens` CLI is installed on PATH and where it points. */
export async function srelensCliStatus(): Promise<CliStatus> {
  return invokeCommand<CliStatus>("srelens_cli_status");
}
