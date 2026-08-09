//! Per-agent identity and command construction. Pure: given a resolved binary
//! path, a server URL and a token, produce the argv and MCP-config file the CLI
//! needs. Spawning and PATH detection live in the desktop crate.

use serde::Serialize;

/// Which agent CLI. Only Claude ships in v1; the others are placeholders so
/// the picker and detection can already enumerate them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    Claude,
    Codex,
    Cursor,
}

impl AgentKind {
    /// The binary name to look for on PATH.
    pub fn binary(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Cursor => "cursor-agent",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Cursor => "Cursor",
        }
    }
}

/// What the WebView shows in the agent picker: each kind, whether it's usable,
/// and (if found) its resolved path and version.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub kind: AgentKind,
    pub label: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// Vendor install page, shown when `available` is false.
    pub install_url: String,
}

/// The `--mcp-config` file content: one HTTP MCP server named `srelens`.
#[derive(Debug, Clone, Serialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers")]
    servers: serde_json::Value,
}

impl McpConfig {
    pub fn http(url: &str, token: &str) -> Self {
        McpConfig {
            servers: serde_json::json!({
                "srelens": {
                    "type": "http",
                    "url": url,
                    "headers": { "Authorization": format!("Bearer {token}") }
                }
            }),
        }
    }
}

/// A resolved command line, ready for the desktop crate to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Build the Claude Code argv. `resume` carries a prior session id for a
/// follow-up turn. Prompt is the trailing positional so it can't be mistaken
/// for a flag value.
pub fn claude_command(
    binary: &str,
    prompt: &str,
    mcp_config_path: &str,
    resume: Option<&str>,
) -> AgentCommand {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--mcp-config".to_string(),
        mcp_config_path.to_string(),
        // srelens gates tool calls itself, so bypass Claude's own permission
        // prompt — otherwise every call blocks twice.
        "--dangerously-skip-permissions".to_string(),
    ];
    if let Some(id) = resume {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }
    args.push(prompt.to_string());
    AgentCommand { program: binary.to_string(), args }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_command_points_the_cli_at_our_mcp_server() {
        let cmd = claude_command("/usr/bin/claude", "Why is web-0 failing?", "/tmp/mcp.json", None);
        assert_eq!(cmd.program, "/usr/bin/claude");
        // Non-interactive, streaming, our config, permissions bypassed so tool
        // calls don't block on Claude's own allowlist (srelens gates them).
        assert!(cmd.args.contains(&"-p".to_string()));
        assert!(cmd.args.windows(2).any(|w| w == ["--output-format", "stream-json"]));
        assert!(cmd.args.windows(2).any(|w| w == ["--mcp-config", "/tmp/mcp.json"]));
        assert!(cmd.args.contains(&"--verbose".to_string()));
        // The prompt is the trailing positional argument.
        assert_eq!(cmd.args.last().unwrap(), "Why is web-0 failing?");
    }

    #[test]
    fn a_resume_id_adds_the_resume_flag() {
        let cmd = claude_command("/usr/bin/claude", "and now?", "/tmp/mcp.json", Some("sess-123"));
        assert!(cmd.args.windows(2).any(|w| w == ["--resume", "sess-123"]));
    }

    #[test]
    fn the_mcp_config_is_an_http_server_entry_with_the_bearer_token() {
        let cfg = McpConfig::http("http://127.0.0.1:8765/mcp", "deadbeef");
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["mcpServers"]["srelens"]["url"], "http://127.0.0.1:8765/mcp");
        assert_eq!(
            v["mcpServers"]["srelens"]["headers"]["Authorization"],
            "Bearer deadbeef"
        );
    }
}
