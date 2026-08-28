import { describe, it, expect } from "vitest";
import { mcpClientConfig, MCP_TOOLS } from "./mcpClients";

describe("mcpClientConfig", () => {
  it("emits a `claude mcp add` command for Claude Code (stdio)", () => {
    const c = mcpClientConfig("claude-code", "stdio", {});
    expect(c.format).toBe("shell");
    expect(c.snippet).toBe("claude mcp add srelens -- srelens --mcp-stdio");
  });

  it("emits an http `claude mcp add` command with the url and bearer token", () => {
    const c = mcpClientConfig("claude-code", "http", {
      url: "http://127.0.0.1:8765/mcp",
      token: "a".repeat(64),
    });
    expect(c.snippet).toContain("--transport http");
    expect(c.snippet).toContain("http://127.0.0.1:8765/mcp");
    expect(c.snippet).toContain(`--header "Authorization: Bearer ${"a".repeat(64)}"`);
  });

  it("emits an obvious placeholder instead of the token when none exists yet", () => {
    const c = mcpClientConfig("claude-code", "http", { url: "http://127.0.0.1:8765/mcp", token: null });
    expect(c.snippet).not.toContain("Bearer");
    expect(c.snippet).toContain("<enable the MCP server to generate a token>");
  });

  it("omits any token concern for stdio, even if one is passed", () => {
    const c = mcpClientConfig("claude-code", "stdio", { token: "a".repeat(64) });
    expect(c.snippet).toBe("claude mcp add srelens -- srelens --mcp-stdio");
  });

  it("emits an mcpServers JSON block for Claude Desktop / Cursor / Antigravity (stdio)", () => {
    for (const tool of ["claude-desktop", "cursor", "antigravity", "generic"] as const) {
      const c = mcpClientConfig(tool, "stdio", {});
      expect(c.format).toBe("json");
      const parsed = JSON.parse(c.snippet);
      expect(parsed.mcpServers.srelens).toEqual({ command: "srelens", args: ["--mcp-stdio"] });
    }
  });

  it("emits a url entry with a bearer header for JSON tools over http", () => {
    const c = mcpClientConfig("cursor", "http", {
      url: "http://127.0.0.1:9000/mcp",
      token: "b".repeat(64),
    });
    expect(JSON.parse(c.snippet).mcpServers.srelens).toEqual({
      url: "http://127.0.0.1:9000/mcp",
      headers: { Authorization: `Bearer ${"b".repeat(64)}` },
    });
  });

  it("emits a placeholder header for JSON tools over http with no token yet", () => {
    const c = mcpClientConfig("cursor", "http", { url: "http://127.0.0.1:9000/mcp", token: null });
    expect(JSON.parse(c.snippet).mcpServers.srelens).toEqual({
      url: "http://127.0.0.1:9000/mcp",
      headers: { Authorization: "<enable the MCP server to generate a token>" },
    });
  });

  it("emits TOML for Codex", () => {
    const c = mcpClientConfig("codex", "stdio", {});
    expect(c.format).toBe("toml");
    expect(c.snippet).toContain("[mcp_servers.srelens]");
    expect(c.snippet).toContain('command = "srelens"');
    expect(c.snippet).toContain('args = ["--mcp-stdio"]');
  });

  it("emits TOML for Codex over http with a headers table carrying the bearer token", () => {
    const c = mcpClientConfig("codex", "http", {
      url: "http://127.0.0.1:8765/mcp",
      token: "c".repeat(64),
    });
    expect(c.format).toBe("toml");
    expect(c.snippet).toContain('url = "http://127.0.0.1:8765/mcp"');
    expect(c.snippet).toContain("[mcp_servers.srelens.headers]");
    expect(c.snippet).toContain(`Authorization = "Bearer ${"c".repeat(64)}"`);
  });

  it("carries a hint about where the config goes for each tool", () => {
    for (const tool of MCP_TOOLS) {
      expect(mcpClientConfig(tool.id, "stdio", {}).hint).toBeTruthy();
    }
  });
});
