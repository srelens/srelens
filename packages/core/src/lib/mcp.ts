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

/** One atomic view of the in-app HTTP server and the sessions belonging to
 * that exact listener generation. A stopped server always has a null URL and
 * no clients; a running server can legitimately have an empty client list. */
export interface McpHttpSnapshot {
  running: boolean;
  url: string | null;
  clients: ConnectedMcpClient[];
}

/** Read server liveness, address, and HTTP client sessions together. */
export async function mcpHttpSnapshot(): Promise<McpHttpSnapshot> {
  return invokeCommand<McpHttpSnapshot>("mcp_http_snapshot");
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
