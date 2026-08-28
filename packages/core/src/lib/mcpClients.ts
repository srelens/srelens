/**
 * Ready-to-paste MCP client configuration for the tools people connect to
 * srelens. srelens runs as an MCP server via its own binary — `srelens
 * --mcp-stdio` for clients that spawn a subprocess, or the loopback HTTP
 * endpoint for clients that connect to a URL.
 */

export type McpTool = "claude-code" | "claude-desktop" | "cursor" | "codex" | "antigravity" | "generic";
export type McpTransport = "stdio" | "http";

export interface McpToolInfo {
  id: McpTool;
  label: string;
  /** Where the config lives / how to apply it. */
  hint: string;
}

export const MCP_TOOLS: McpToolInfo[] = [
  { id: "claude-code", label: "Claude Code", hint: "Run the command in your terminal." },
  {
    id: "claude-desktop",
    label: "Claude Desktop",
    hint: "Add to claude_desktop_config.json (Settings → Developer → Edit Config).",
  },
  { id: "cursor", label: "Cursor", hint: "Add to ~/.cursor/mcp.json (or a project .cursor/mcp.json)." },
  { id: "codex", label: "Codex", hint: "Add to ~/.codex/config.toml." },
  { id: "antigravity", label: "Antigravity", hint: "Add to the IDE's MCP config (mcpServers)." },
  { id: "generic", label: "Other (mcpServers JSON)", hint: "Most MCP clients accept this mcpServers block." },
];

export interface McpClientConfig {
  format: "shell" | "json" | "toml";
  snippet: string;
  hint: string;
}

const DEFAULT_URL = "http://127.0.0.1:8765/mcp";

/** Emitted instead of a real bearer value when http config is generated
 * before a token exists. Deliberately not a plausible-looking token: a
 * config that quietly 401s because it copied an empty/undefined value would
 * be worse than one that's obviously incomplete. */
const NO_TOKEN_PLACEHOLDER = "<enable the MCP server to generate a token>";

/** Pretty mcpServers JSON block for a stdio or http entry. */
function mcpServersJson(entry: Record<string, unknown>): string {
  return JSON.stringify({ mcpServers: { srelens: entry } }, null, 2);
}

/**
 * Config for connecting `tool` to srelens over `transport`. `opts.token` is
 * the current MCP bearer token (or `null`/absent if none has been generated
 * yet) — the HTTP transport requires it on every request, so an http config
 * without it would 401. Ignored for stdio, which needs no token at all.
 */
export function mcpClientConfig(
  tool: McpTool,
  transport: McpTransport,
  opts: { url?: string; token?: string | null },
): McpClientConfig {
  const url = opts.url || DEFAULT_URL;
  const authValue = transport === "http" ? (opts.token ? `Bearer ${opts.token}` : NO_TOKEN_PLACEHOLDER) : "";
  const hint = MCP_TOOLS.find((t) => t.id === tool)?.hint ?? "";

  if (tool === "claude-code") {
    const snippet =
      transport === "stdio"
        ? "claude mcp add srelens -- srelens --mcp-stdio"
        : `claude mcp add --transport http srelens ${url} --header "Authorization: ${authValue}"`;
    return { format: "shell", snippet, hint };
  }

  if (tool === "codex") {
    const snippet =
      transport === "stdio"
        ? `[mcp_servers.srelens]\ncommand = "srelens"\nargs = ["--mcp-stdio"]`
        : `[mcp_servers.srelens]\nurl = "${url}"\n\n[mcp_servers.srelens.headers]\nAuthorization = "${authValue}"`;
    return { format: "toml", snippet, hint };
  }

  // JSON mcpServers tools: Claude Desktop, Cursor, Antigravity, generic.
  const entry =
    transport === "stdio"
      ? { command: "srelens", args: ["--mcp-stdio"] }
      : { url, headers: { Authorization: authValue } };
  return { format: "json", snippet: mcpServersJson(entry), hint };
}
