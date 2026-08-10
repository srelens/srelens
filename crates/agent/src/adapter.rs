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
    /// Installed but not yet selectable — its sandbox story isn't solved yet.
    /// Distinct from `available: false` ("not installed").
    pub gated: bool,
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
    /// Environment variables the desktop must set on the child process.
    /// Used for secrets (e.g. Codex's MCP bearer token) that must never
    /// appear in argv, where they'd be visible to anyone who can list
    /// processes on the machine.
    pub env: Vec<(String, String)>,
}

/// The only tools the agent may call: srelens's own MCP tools, exposed under
/// the `srelens` server name configured in `McpConfig::http`. Kept for
/// documentation of intent — see `DISALLOWED_TOOLS` for the flag that
/// actually enforces this under `--dangerously-skip-permissions`.
const ALLOWED_TOOLS: &str = "mcp__srelens__*";

/// Built-in Claude Code tools that must never run — the agent operates the
/// cluster only through srelens's MCP tools. This, not `ALLOWED_TOOLS`, is
/// the real box: allow rules (`--allowedTools`) have no effect under
/// `--dangerously-skip-permissions` because bypass mode already approves
/// everything, but deny rules (`--disallowedTools`) remain effective even in
/// bypass mode. Covers shell (`Bash`), file read/write
/// (`Read`/`Edit`/`Write`/`NotebookEdit`), file enumeration
/// (`Glob`/`Grep`), network (`WebFetch`/`WebSearch`), and subagent spawning
/// (`Task`, which could itself invoke any of the above).
///
/// Residual risk: this is a deny-list, not deny-by-default — a new built-in
/// tool added in a future Claude Code version would not be covered until
/// this list is updated. Tracked as a known gap, not solved here.
const DISALLOWED_TOOLS: &str = "Bash Read Edit Write NotebookEdit Glob Grep WebFetch WebSearch Task";

/// Establishes the assistant's identity and scope. Deliberately names no
/// local path and makes no mention of srelens's own source, repo, or
/// branches — it must not leak anything about the machine srelens runs on,
/// only the cluster-operating role the agent is boxed into.
pub const BASE_SYSTEM_PROMPT: &str = "You are srelens's Kubernetes assistant. Investigate and operate the selected cluster(s) ONLY through the srelens MCP tools (the mcp__srelens__* tools). You have no access to the local filesystem, shell, git, or network beyond those tools; do not attempt to read files or run commands. Be concise. Anything that changes cluster state will prompt the user for confirmation.";

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
        // prompt — otherwise every call blocks twice. Safe only because
        // --allowedTools below restricts the agent to srelens's own MCP
        // tools, whose destructive subset is still gated by srelens's
        // confirm dialog.
        "--dangerously-skip-permissions".to_string(),
        "--allowedTools".to_string(),
        ALLOWED_TOOLS.to_string(),
        "--disallowedTools".to_string(),
        DISALLOWED_TOOLS.to_string(),
        "--append-system-prompt".to_string(),
        BASE_SYSTEM_PROMPT.to_string(),
    ];
    if let Some(id) = resume {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }
    // `--` ends option parsing so a prompt that itself looks like a flag
    // (e.g. a pasted `--disallowedTools none`) can never be reinterpreted by
    // clap as overriding the flags above — it is always treated as the
    // trailing positional message. Verified live against the real CLI.
    args.push("--".to_string());
    args.push(prompt.to_string());
    // Claude passes its MCP bearer token via the config file at
    // `mcp_config_path`, never via env.
    AgentCommand { program: binary.to_string(), args, env: Vec::new() }
}

/// Escape a string for embedding inside a double-quoted TOML value (used for
/// the `-c mcp_servers.srelens.*` fragments below). Defensive: `mcp_url` is a
/// trusted localhost URL today, but this prevents a malformed or attacker-
/// controlled URL from breaking out of the quoted literal and injecting
/// extra TOML keys.
fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The environment variable name the desktop must set on the Codex child
/// process to carry the MCP bearer token. Never passed via argv (see
/// `codex_command`).
pub const CODEX_TOKEN_ENV: &str = "SRELENS_MCP_TOKEN";

/// Build the Codex CLI argv. This is the security-critical sandbox: these
/// exact flags were verified live against the real `codex` CLI to box it to
/// srelens's MCP tools with no other escape.
///
/// `--disable shell_tool --disable unified_exec` removes Codex's shell — its
/// only whole-disk-read escape hatch. What remains is Codex's internal file
/// tool, which is confined to the empty directory passed via `-C` under
/// `-s read-only` (so it can read nothing outside that empty workspace and
/// write nothing at all). MCP tools (srelens's own) still work normally,
/// configured via the two `-c mcp_servers.srelens.*` TOML overrides below.
/// Do not change these flags without re-verifying against the real CLI.
///
/// The bearer token is passed only via `env` (`CODEX_TOKEN_ENV`), never as an
/// argv value — argv is visible to anyone who can list processes on the
/// machine, whereas `env` is only visible to the process's own user/owner.
pub fn codex_command(
    binary: &str,
    prompt: &str,
    mcp_url: &str,
    token: &str,
    empty_cwd: &str,
    image_paths: &[String],
) -> AgentCommand {
    let mut args = vec![
        "exec".to_string(),
        "--json".to_string(),
        "--skip-git-repo-check".to_string(),
        "--disable".to_string(),
        "shell_tool".to_string(),
        "--disable".to_string(),
        "unified_exec".to_string(),
        "-s".to_string(),
        "read-only".to_string(),
        "-C".to_string(),
        empty_cwd.to_string(),
        "-c".to_string(),
        format!("mcp_servers.srelens.url=\"{}\"", escape_toml_string(mcp_url)),
        "-c".to_string(),
        format!("mcp_servers.srelens.bearer_token_env_var=\"{CODEX_TOKEN_ENV}\""),
    ];
    for path in image_paths {
        args.push("-i".to_string());
        args.push(path.clone());
    }
    // `--` ends option parsing so a prompt that itself looks like a flag
    // (e.g. a pasted `-s danger-full-access`) can never be reinterpreted by
    // clap as overriding `-s read-only`/`-C <empty_cwd>` above — it is always
    // treated as the trailing positional message. Verified live against the
    // real CLI.
    args.push("--".to_string());
    args.push(prompt.to_string());
    AgentCommand {
        program: binary.to_string(),
        args,
        env: vec![(CODEX_TOKEN_ENV.to_string(), token.to_string())],
    }
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
    fn builtin_tools_are_denied_since_allow_rules_are_inert_under_bypass() {
        let cmd = claude_command("/usr/bin/claude", "and now?", "/tmp/mcp.json", None);
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w[0] == "--disallowedTools" && w[1] == DISALLOWED_TOOLS));
        assert!(DISALLOWED_TOOLS.contains("Bash"));
        assert!(DISALLOWED_TOOLS.contains("Read"));
        assert!(DISALLOWED_TOOLS.contains("WebFetch"));
    }

    #[test]
    fn only_srelens_mcp_tools_are_allowed() {
        let cmd = claude_command("/usr/bin/claude", "and now?", "/tmp/mcp.json", None);
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w == ["--allowedTools", "mcp__srelens__*"]));
    }

    #[test]
    fn the_base_system_prompt_is_appended() {
        let cmd = claude_command("/usr/bin/claude", "and now?", "/tmp/mcp.json", None);
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w[0] == "--append-system-prompt" && w[1] == BASE_SYSTEM_PROMPT));
    }

    #[test]
    fn the_prompt_stays_the_trailing_positional_with_all_new_flags() {
        let cmd = claude_command("/usr/bin/claude", "Why is web-0 failing?", "/tmp/mcp.json", None);
        assert_eq!(cmd.args.last().unwrap(), "Why is web-0 failing?");
    }

    #[test]
    fn resume_still_only_appears_when_some() {
        let cmd = claude_command("/usr/bin/claude", "and now?", "/tmp/mcp.json", None);
        assert!(!cmd.args.contains(&"--resume".to_string()));
    }

    #[test]
    fn base_system_prompt_scopes_the_agent_without_leaking_local_details() {
        assert!(BASE_SYSTEM_PROMPT.contains("mcp__srelens__"));
        assert!(BASE_SYSTEM_PROMPT.contains("cluster"));
        // Must not leak a local path, srelens's own repo, or its branches.
        assert!(!BASE_SYSTEM_PROMPT.contains("/Users"));
        assert!(!BASE_SYSTEM_PROMPT.contains("/home"));
        assert!(!BASE_SYSTEM_PROMPT.to_lowercase().contains("repo"));
        assert!(!BASE_SYSTEM_PROMPT.to_lowercase().contains("branch"));
        assert!(!BASE_SYSTEM_PROMPT.to_lowercase().contains("filesystem-access"));
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

    fn codex_cmd() -> AgentCommand {
        codex_command(
            "/usr/bin/codex",
            "Why is web-0 failing?",
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &[],
        )
    }

    #[test]
    fn codex_command_disables_the_shell_and_unified_exec_tools() {
        let cmd = codex_cmd();
        assert!(cmd.args.windows(2).any(|w| w == ["--disable", "shell_tool"]));
        assert!(cmd.args.windows(2).any(|w| w == ["--disable", "unified_exec"]));
    }

    #[test]
    fn codex_command_is_sandboxed_to_a_read_only_empty_workspace() {
        let cmd = codex_cmd();
        assert!(cmd.args.windows(2).any(|w| w == ["-s", "read-only"]));
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w[0] == "-C" && w[1] == "/tmp/srelens-empty-cwd"));
    }

    #[test]
    fn codex_command_points_the_mcp_config_at_our_server() {
        let cmd = codex_cmd();
        assert!(cmd.args.windows(2).any(|w| {
            w[0] == "-c" && w[1] == "mcp_servers.srelens.url=\"http://127.0.0.1:8765/mcp\""
        }));
        assert!(cmd.args.windows(2).any(|w| {
            w[0] == "-c"
                && w[1] == "mcp_servers.srelens.bearer_token_env_var=\"SRELENS_MCP_TOKEN\""
        }));
    }

    #[test]
    fn codex_command_carries_the_token_only_in_env_never_in_argv() {
        let cmd = codex_cmd();
        assert_eq!(cmd.env, vec![(CODEX_TOKEN_ENV.to_string(), "deadbeef".to_string())]);
        assert!(!cmd.args.iter().any(|a| a.contains("deadbeef")));
    }

    #[test]
    fn codex_command_turns_each_image_path_into_an_i_flag_pair_in_order() {
        let cmd = codex_command(
            "/usr/bin/codex",
            "describe these",
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &["/tmp/img1.png".to_string(), "/tmp/img2.png".to_string()],
        );
        let i_pairs: Vec<&[String]> =
            cmd.args.windows(2).filter(|w| w[0] == "-i").collect();
        assert_eq!(i_pairs.len(), 2);
        assert_eq!(i_pairs[0][1], "/tmp/img1.png");
        assert_eq!(i_pairs[1][1], "/tmp/img2.png");
    }

    #[test]
    fn codex_command_keeps_the_prompt_as_the_trailing_positional() {
        let cmd = codex_command(
            "/usr/bin/codex",
            "Why is web-0 failing?",
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &["/tmp/img1.png".to_string()],
        );
        assert_eq!(cmd.args.last().unwrap(), "Why is web-0 failing?");
    }

    #[test]
    fn claude_command_sets_no_env_since_its_token_travels_via_config_file() {
        let cmd = claude_command("/usr/bin/claude", "and now?", "/tmp/mcp.json", None);
        assert!(cmd.env.is_empty());
    }

    #[test]
    fn claude_command_ends_options_before_the_prompt_so_it_cannot_inject_flags() {
        let prompt = "--disallowedTools none";
        let cmd = claude_command("/usr/bin/claude", prompt, "/tmp/mcp.json", None);
        assert_eq!(cmd.args[cmd.args.len() - 2], "--");
        assert_eq!(cmd.args[cmd.args.len() - 1], prompt);
    }

    #[test]
    fn claude_command_ends_options_before_the_prompt_even_with_a_resume_id() {
        let prompt = "--allowedTools Bash";
        let cmd = claude_command("/usr/bin/claude", prompt, "/tmp/mcp.json", Some("sess-123"));
        assert_eq!(cmd.args[cmd.args.len() - 2], "--");
        assert_eq!(cmd.args[cmd.args.len() - 1], prompt);
    }

    #[test]
    fn codex_command_ends_options_before_the_prompt_so_it_cannot_inject_flags() {
        let prompt = "-s danger-full-access reply";
        let cmd = codex_command(
            "/usr/bin/codex",
            prompt,
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &[],
        );
        assert_eq!(cmd.args[cmd.args.len() - 2], "--");
        assert_eq!(cmd.args[cmd.args.len() - 1], prompt);
    }

    #[test]
    fn codex_command_ends_options_before_the_prompt_even_with_images() {
        let prompt = "-C /evil";
        let cmd = codex_command(
            "/usr/bin/codex",
            prompt,
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &["/tmp/img1.png".to_string()],
        );
        assert_eq!(cmd.args[cmd.args.len() - 2], "--");
        assert_eq!(cmd.args[cmd.args.len() - 1], prompt);
    }

    #[test]
    fn codex_command_escapes_quotes_and_backslashes_in_the_mcp_url_toml_fragment() {
        let cmd = codex_command(
            "/usr/bin/codex",
            "hi",
            "http://127.0.0.1:8765/mcp?x=\"evil\"\\",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &[],
        );
        let expected =
            "mcp_servers.srelens.url=\"http://127.0.0.1:8765/mcp?x=\\\"evil\\\"\\\\\"";
        assert!(cmd.args.windows(2).any(|w| w[0] == "-c" && w[1] == expected));
    }
}
