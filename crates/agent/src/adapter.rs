//! Per-agent identity and command construction. Pure: given a resolved binary
//! path, a server URL and a token, produce the argv and MCP-config file the CLI
//! needs. Finding the binary and spawning it are each app's (the desktop
//! and srectl).

use serde::{Deserialize, Serialize};

/// Which agent backs a turn. The first three shell out to their vendor CLIs;
/// `Srelens` is srelens's own in-process agent, which talks directly to a
/// provider API with the user's key and needs no external binary at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    Claude,
    Codex,
    Cursor,
    #[default]
    Srelens,
}

impl AgentKind {
    /// The binary name to look for on PATH, or `None` for the native agent
    /// (`Srelens`), which runs in-process and resolves no executable.
    pub fn binary(self) -> Option<&'static str> {
        match self {
            AgentKind::Claude => Some("claude"),
            AgentKind::Codex => Some("codex"),
            AgentKind::Cursor => Some("cursor-agent"),
            AgentKind::Srelens => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Cursor => "Cursor",
            // Lowercase brand, deliberately.
            AgentKind::Srelens => "srelens",
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

/// A resolved command line, ready for an app to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
    /// Environment variables the app must set on the child process.
    /// Used for secrets (e.g. Codex's MCP bearer token) that must never
    /// appear in argv, where they'd be visible to anyone who can list
    /// processes on the machine.
    pub env: Vec<(String, String)>,
    /// What to write to the child's stdin before closing it; `None` spawns
    /// it with stdin closed.
    pub stdin: Option<String>,
}

/// Whether `id` may follow a CLI's model flag. A model id comes from a
/// setting or a chat, so it must not be able to pass for a flag or smuggle
/// whitespace into argv: non-empty, at most 128 characters, not starting with
/// `-`, and only `[A-Za-z0-9._:/@\[\]-]` (`claude-fable-5[1m]`,
/// `models/gemini-2.5-pro`). Callers drop an invalid id (the CLI's own
/// default model) rather than failing the turn.
pub fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '[' | ']' | '-'))
}

/// One model a CLI offers: the `id` goes after its model flag, the `label` is
/// what a picker shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
}

/// Claude Code has no model-listing command, so the picker offers the aliases
/// its `--model` flag accepts.
pub const CLAUDE_MODEL_ALIASES: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

/// The argv (after the binary) that prints `kind`'s models, or `None` when
/// the CLI has no such command (Claude Code, and the native agent, which has
/// no CLI at all).
pub fn model_list_args(kind: AgentKind) -> Option<&'static [&'static str]> {
    match kind {
        AgentKind::Codex => Some(&["debug", "models"]),
        AgentKind::Cursor => Some(&["models"]),
        AgentKind::Claude | AgentKind::Srelens => None,
    }
}

impl AgentCommand {
    /// Run on `model`: Codex takes `-m <id>`, Claude Code and Cursor
    /// `--model <id>`, put right after the first arg (`exec` / `-p`) so the
    /// rest of the argv, and the trailing prompt, stay as built. `None`, an
    /// id `valid_model_id` refuses, and the native agent (it has no CLI)
    /// leave the command unchanged.
    pub fn with_model(mut self, kind: AgentKind, model: Option<&str>) -> Self {
        let flag = match kind {
            AgentKind::Codex => "-m",
            AgentKind::Claude | AgentKind::Cursor => "--model",
            AgentKind::Srelens => return self,
        };
        if let Some(id) = model.filter(|id| valid_model_id(id)) {
            let at = self.args.len().min(1);
            self.args.splice(at..at, [flag.to_string(), id.to_string()]);
        }
        self
    }
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
const DISALLOWED_TOOLS: &str =
    "Bash Read Edit Write NotebookEdit Glob Grep WebFetch WebSearch Task";

/// Establishes the assistant's identity and scope. Deliberately names no
/// local path and makes no mention of srelens's own source, repo, or
/// branches — it must not leak anything about the machine srelens runs on,
/// only the cluster-operating role the agent is boxed into.
pub const BASE_SYSTEM_PROMPT: &str = concat!(
    "You are srelens's Kubernetes assistant. Investigate and operate the selected cluster(s) ",
    "ONLY through the srelens MCP tools (the mcp__srelens__* tools). You have no access to the ",
    "local filesystem, shell, git, or network beyond those tools; do not attempt to read files ",
    "or run commands. Be concise. Anything that changes cluster state will prompt the user ",
    "for confirmation.\n\n",
    "Core Investigation Rules:\n",
    "1. Always query live cluster state: Cluster state is dynamic. Never answer questions about ",
    "resource existence, status, capacity, labels, taints, or scheduling feasibility from memory, ",
    "assumptions, or prior conversation turns. Always execute the appropriate MCP tool to inspect ",
    "the live cluster freshly before answering.\n",
    "2. Say what you know, not what you guess: Summary listing tools (such as listNodes or listPods) ",
    "return abridged data and omit full metadata.labels, annotations, extended resources, and complete ",
    "container specs. If a field or label is not displayed in a summary list, NEVER assert that it does ",
    "not exist on the resource. To read it across many resources, call listResource once with `fields` ",
    "(kubectl-style JSONPath such as \".status.allocatable\" or \".spec.taints\"), narrowing the set with ",
    "labelSelector or fieldSelector; do not call getObject once per item. For one specific resource, ",
    "call getObject (with `fields` when only part of it is needed) or getManifest.\n",
    "3. Zero-hallucination component verification: NEVER name, diagnose, or blame a specific infrastructure ",
    "component (such as Cilium, Calico, Istio, MetalLB, CoreDNS, Vault) without confirming its presence ",
    "in the cluster first via listPods or getObject. On managed Kubernetes (GKE, EKS, AKS), verify the native ",
    "cloud provider components (e.g. GKE Datapath V2 with anetd/netd) rather than assuming open-source defaults.\n",
    "4. Tool failure transparency: If an MCP tool call fails or errors, state clearly that the tool call ",
    "failed and try an alternative (e.g. getObject, getManifest, listEndpoints, listEndpointSlices). NEVER ",
    "fabricate a cluster issue or speculate on cluster health to explain away a failed tool call.\n",
    "5. Cross-layer incident correlation: When investigating why pods restarted, disappeared, or moved:\n",
    "- Young pods (<30m) with 0 restart count do not establish why they were created. Check events and owner history before attributing their creation to eviction, replacement, scaling, or a rollout.\n",
    "- Trace the timeline: Check listEvents for TaintManagerEviction, Killing, SuccessfulCreate, and node NodeNotReady events.\n",
    "- Inspect candidate nodes with listEvents or getObject for kernel OOMKilling, MemoryPressure, DiskPressure, and ReadOnlyFileSystemDetected.\n",
    "- Distinguish between application rollouts (new ReplicaSet revision or image) and infrastructure-level pod rescheduling.\n",
    "6. Compute and scheduling headroom: A node's allocatable capacity is the ceiling for all pods, ",
    "NOT free or available headroom. Nodes always run system daemonsets and existing workloads. ",
    "Never calculate whether a pod will fit by subtracting its requests from node allocatable ",
    "capacity alone. Always inspect the running pods on candidate nodes (via podsOnNode or ",
    "listPods) and sum their requested CPU and memory:\n",
    "Schedulable Headroom = Allocatable - Sum(Running Pod Requests).\n",
    "Only state a pod can schedule if Schedulable Headroom >= Pod Request, and all node affinities, ",
    "tolerations, and taints match.\n",
    "7. Service endpoints and network discovery: To inspect service endpoints, call listEndpoints ",
    "or listEndpointSlices. Check whether endpoints exist, are ready, and match selector-backed pods.\n",
    "8. GitOps and ArgoCD in Multi-Cluster (Hub-and-Spoke): If an ArgoCD Hub Context is designated, ",
    "or if the active cluster has no argoproj.io CRDs (such as Application), ArgoCD applications ",
    "and GitOps controllers reside on the Hub cluster while workloads run on the spoke cluster. ",
    "Query the ArgoCD Hub context with \"context\": \"<hub_context>\" to inspect Application definitions, ",
    "sync status, and deployment sources for workloads on the active cluster.",
);

/// Whether `program` is a Windows batch file (`.cmd`/`.bat`, any case), as
/// npm installs `claude` and `codex`. Rust refuses to spawn one with a newline
/// in an argument, and cmd.exe expands `%VAR%` in the rest, so such a binary
/// gets its prompt on stdin (`claude_command_via_stdin`,
/// `codex_command_via_stdin`). Pure path logic, the same on every OS.
pub fn is_batch_shim(program: &str) -> bool {
    std::path::Path::new(program)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
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
    let mut args = claude_args(mcp_config_path, ["--append-system-prompt", BASE_SYSTEM_PROMPT], resume);
    // `--` ends option parsing so a prompt that itself looks like a flag
    // (e.g. a pasted `--disallowedTools none`) can never be reinterpreted by
    // clap as overriding the flags above — it is always treated as the
    // trailing positional message. Verified live against the real CLI.
    args.push("--".to_string());
    args.push(prompt.to_string());
    // Claude passes its MCP bearer token via the config file at
    // `mcp_config_path`, never via env.
    AgentCommand {
        program: binary.to_string(),
        args,
        env: Vec::new(),
        stdin: None,
    }
}

/// `claude_command` for a batch shim (`is_batch_shim`): the same box, with the
/// system prompt read from `system_prompt_file` and no prompt in argv — the
/// caller sets `stdin` to it, which `claude -p` reads when given no positional.
pub fn claude_command_via_stdin(
    binary: &str,
    mcp_config_path: &str,
    system_prompt_file: &str,
    resume: Option<&str>,
) -> AgentCommand {
    AgentCommand {
        program: binary.to_string(),
        args: claude_args(mcp_config_path, ["--append-system-prompt-file", system_prompt_file], resume),
        env: Vec::new(),
        stdin: None,
    }
}

/// Every Claude flag but the prompt: the box both builders above share, so
/// they cannot drift apart. `system_prompt` is the flag and its value.
fn claude_args(mcp_config_path: &str, system_prompt: [&str; 2], resume: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--mcp-config".to_string(),
        mcp_config_path.to_string(),
        // Use ONLY the srelens server supplied above — without this, Claude also
        // loads the user's global/project `.mcp.json` servers, whose tools the
        // model could invoke (we run with --dangerously-skip-permissions and the
        // deny-list below covers only built-in tools), breaking the srelens-only
        // box. Verified against the CLI reference.
        "--strict-mcp-config".to_string(),
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
        system_prompt[0].to_string(),
        system_prompt[1].to_string(),
    ];
    if let Some(id) = resume {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }
    args
}

/// Escape a string for embedding inside a double-quoted TOML value (used for
/// the `-c mcp_servers.srelens.*` fragments below). Defensive: `mcp_url` is a
/// trusted localhost URL today, but this prevents a malformed or attacker-
/// controlled URL from breaking out of the quoted literal and injecting
/// extra TOML keys.
fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The environment variable name the app must set on the Codex child
/// process to carry the MCP bearer token. Never passed via argv (see
/// `codex_command`).
pub const CODEX_TOKEN_ENV: &str = "SRELENS_MCP_TOKEN";

/// Build the Codex CLI argv. This is the security-critical sandbox: these
/// exact flags were verified live against the real `codex` CLI to box it to
/// srelens's MCP tools with no other escape.
///
/// `--disable shell_tool --disable unified_exec` removes Codex's shell, and
/// the feature switches below remove every other default tool surface, so no
/// tool the model gets can read a file: `-s read-only` itself still grants
/// reads of the whole disk (its recorded profile is `root: read`), so the
/// absence of a reading tool, not the sandbox, is what keeps reads in. The
/// sandbox refuses every write, and `-C` points the turn at an empty
/// directory. MCP tools (srelens's own) still work normally, configured via
/// the two `-c mcp_servers.srelens.*` TOML overrides below. Known gap:
/// `apply_patch` reads a target file to verify a patch before the sandbox
/// refuses the write, so a model can test guesses about a file's existence
/// and lines (never see its content); dropping the tool takes a whole
/// replacement model catalog. Do not change these flags without
/// re-verifying against the real CLI.
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
    resume: Option<&str>,
) -> AgentCommand {
    codex_boxed(binary, prompt, mcp_url, token, empty_cwd, image_paths, resume)
}

/// `codex_command` for a batch shim (`is_batch_shim`): the same argv with `-`
/// as the prompt, which tells `codex exec` to read it from stdin — the caller
/// sets `stdin` to it.
pub fn codex_command_via_stdin(
    binary: &str,
    mcp_url: &str,
    token: &str,
    empty_cwd: &str,
    image_paths: &[String],
    resume: Option<&str>,
) -> AgentCommand {
    codex_boxed(binary, "-", mcp_url, token, empty_cwd, image_paths, resume)
}

/// The box both Codex builders above share, so they cannot drift apart;
/// `prompt_arg` is the trailing positional.
fn codex_boxed(
    binary: &str,
    prompt_arg: &str,
    mcp_url: &str,
    token: &str,
    empty_cwd: &str,
    image_paths: &[String],
    resume: Option<&str>,
) -> AgentCommand {
    let mut args = vec![
        "exec".to_string(),
        "--json".to_string(),
        "--skip-git-repo-check".to_string(),
        // Do NOT load `$CODEX_HOME/config.toml` — otherwise the user's own MCP
        // servers there stay active alongside srelens's, exposing filesystem/
        // shell/network tools and breaking the srelens-only box. The `-c`
        // overrides below still add srelens, and auth still uses CODEX_HOME
        // (per `codex exec --help`).
        "--ignore-user-config".to_string(),
        "--disable".to_string(),
        "shell_tool".to_string(),
        "--disable".to_string(),
        "unified_exec".to_string(),
    ];
    // Tool surfaces Codex turns on by default that `--ignore-user-config`
    // does not reach: ChatGPT connectors (`apps`: GitHub commits and merges,
    // site deploys, …), goals, image viewing and generation, plugins and
    // their skills, sub-agents, web search, and the user's own skills catalog
    // (with its local paths). Without these a codex-cli 0.149.1 turn offered
    // the model 221 tools; with them, srelens's MCP tools plus `apply_patch`
    // (writes refused by `-s read-only`), `update_plan` and the MCP-resource
    // tools (srelens's server only). An unknown feature name makes Codex
    // refuse to start, so a renamed flag fails closed (#215, verified live).
    for feature in ["apps", "goals", "view_image", "image_generation", "plugins"] {
        args.push("--disable".to_string());
        args.push(feature.to_string());
    }
    for value in [
        "agents.enabled=false",
        "web_search=\"disabled\"",
        "skills.include_instructions=false",
    ] {
        args.push("-c".to_string());
        args.push(value.to_string());
    }
    args.extend([
        "-s".to_string(),
        "read-only".to_string(),
        "-C".to_string(),
        empty_cwd.to_string(),
        "-c".to_string(),
        format!(
            "mcp_servers.srelens.url=\"{}\"",
            escape_toml_string(mcp_url)
        ),
        "-c".to_string(),
        format!("mcp_servers.srelens.bearer_token_env_var=\"{CODEX_TOKEN_ENV}\""),
        // Surface the model's reasoning as `reasoning` items in the JSON
        // stream (mapped to Thinking events by `codex::parse_line`) — with
        // `--ignore-user-config` above, the user's own config.toml can't
        // enable this, so srelens must. "detailed" only changes what the CLI
        // REPORTS about reasoning the model already does; it does not raise
        // the reasoning effort. Verified live: without it exec --json emits
        // no reasoning items at all.
        "-c".to_string(),
        "model_reasoning_summary=\"detailed\"".to_string(),
    ]);
    // A follow-up continues the turn's thread. Everything above is `exec`'s,
    // so it must stay ahead of the subcommand: `codex exec resume` keeps
    // nothing of the box from the thread it resumes (a bare resume gets the
    // shell back) and its own options take neither `-s` nor `-C`. Images go
    // after it, to `resume`'s own `-i`: `exec`'s takes several values and
    // would swallow `resume <thread>`, silently starting a new thread
    // (#215, verified live).
    if let Some(thread) = resume {
        args.push("resume".to_string());
        args.push(thread.to_string());
    }
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
    args.push(prompt_arg.to_string());
    AgentCommand {
        program: binary.to_string(),
        args,
        env: vec![(CODEX_TOKEN_ENV.to_string(), token.to_string())],
        stdin: None,
    }
}

/// Environment variable set on the Cursor CLI to point it at a per-chat
/// config directory instead of its real `~/.cursor` config.
/// `prepare_cursor_turn` rewrites `cursor_cli_config_json()` there before
/// every turn
/// (`cursor_mcp_json()` goes in the workspace's `.cursor/`), and Cursor keeps
/// its chats there, which `--resume` reads (#952). Never passed via argv.
pub const CURSOR_CONFIG_DIR_ENV: &str = "CURSOR_CONFIG_DIR";

/// Build the Cursor CLI (`cursor-agent`) argv. These exact flags were
/// verified live against the real CLI to box it to srelens's MCP tools:
/// `--approve-mcps` loads/approves the srelens MCP *server*; `--force` is
/// what actually lets the tool *calls* run — without it, non-interactive `-p`
/// mode auto-rejects every MCP call ("User rejected MCP") since Cursor has
/// nowhere to show a per-call approval prompt.
///
/// SECURITY-CRITICAL: `--force` is "allow unless explicitly denied", so with
/// it present the ONLY thing keeping Cursor boxed is the deny-list written to
/// `config_dir` by `cursor_cli_config_json` (read/write/edit/shell/fetch/…).
/// That deny-list is load-bearing, NOT belt-and-suspenders — do not remove it,
/// and do not drop `--force` (MCP calls stop working). NO `--verbose` — Cursor
/// errors on that flag. Do not change these flags without re-verifying live.
///
/// The MCP bearer token travels only in the `mcp.json` file the turn's setup
/// writes into `config_dir` (see `cursor_mcp_json`), never via argv or env —
/// both of which are visible to anyone who can list processes on the
/// machine.
/// Rides at the front of Cursor's prompt (it has no `--append-system-prompt`
/// flag) so the model operates through the srelens MCP tools instead of
/// flailing at its local shell/kubectl/file tools — which the box deny-lists,
/// so they only fail and waste turns. Never shown in srelens's own transcript
/// (it's part of the outgoing prompt, like the context preface).
pub const CURSOR_GUIDANCE: &str = "You are srelens's Kubernetes assistant. Operate the selected cluster(s) ONLY through the srelens MCP tools (from the `srelens` MCP server). You have NO working shell, kubectl, git, or local filesystem access — those tools are blocked and will fail, so never run shell commands or kubectl or read local files; use the srelens MCP tools for everything. Anything that changes cluster state prompts the user for confirmation.";

pub fn cursor_command(
    binary: &str,
    prompt: &str,
    config_dir: &str,
    empty_cwd: &str,
    resume: Option<&str>,
) -> AgentCommand {
    let mut args = vec![
        "-p".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--trust".to_string(),
        // `--approve-mcps` loads/approves the srelens MCP *server*; `--force`
        // auto-runs the tool *calls* — without `--force`, Cursor's headless
        // `-p` mode auto-rejects every MCP call ("User rejected MCP") because
        // there is no interactive approver, even for read-only tools. `--force`
        // is "allow unless explicitly denied", so the local-tool deny-list in
        // `cursor_cli_config_json` still blocks read/write/shell/etc. — verified
        // live: MCP calls succeed while `read`/`shell` are denied.
        "--force".to_string(),
        "--approve-mcps".to_string(),
        "--workspace".to_string(),
        empty_cwd.to_string(),
    ];
    if let Some(id) = resume {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }
    // `--` ends option parsing so a prompt that itself looks like a flag
    // (e.g. a pasted `--force reply`) can never be reinterpreted as
    // overriding the flags above — it is always treated as the trailing
    // positional message. Verified live against the real CLI.
    args.push("--".to_string());
    args.push(format!("{CURSOR_GUIDANCE}\n\n{prompt}"));
    AgentCommand {
        program: binary.to_string(),
        args,
        env: vec![(CURSOR_CONFIG_DIR_ENV.to_string(), config_dir.to_string())],
        stdin: None,
    }
}

/// The Cursor CLI config content (`cli-config.json`, rewritten every turn into
/// the config dir pointed to by `CURSOR_CONFIG_DIR_ENV`) that denies its
/// built-in local tools. Cursor's `-p` mode ships with full local
/// read/write/shell/fetch access; since `cursor_command` passes `--force`
/// (required so MCP calls run), this deny-list is the ONLY thing boxing Cursor
/// to MCP tools — it is load-bearing, not optional, and must stay comprehensive
/// (`--force` allows any tool NOT listed here). Verified live: a boxed Cursor
/// refuses to read local files ("permission denied") while MCP tool calls
/// still succeed.
pub fn cursor_cli_config_json() -> String {
    serde_json::json!({
        "version": 1,
        "permissions": {
            "allow": [],
            "deny": [
                "Read(**)",
                "Write(**)",
                "Edit(**)",
                "Shell(**)",
                "Fetch(**)",
                "Grep(**)",
                "Delete(**)",
                "List(**)",
                "Search(**)",
                "WebFetch(**)",
                "WebSearch(**)"
            ]
        }
    })
    .to_string()
}

/// The srelens MCP server entry Cursor reads from `mcp.json`, written into
/// the turn's workspace `.cursor/`. Built with `serde_json` (not hand-formatted)
/// so `url`/`token` are always properly JSON-escaped rather than risking a
/// malformed or attacker-controlled value breaking out of a hand-built
/// string.
pub fn cursor_mcp_json(url: &str, token: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "srelens": {
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_command_points_the_cli_at_our_mcp_server() {
        let cmd = claude_command(
            "/usr/bin/claude",
            "Why is web-0 failing?",
            "/tmp/mcp.json",
            None,
        );
        assert_eq!(cmd.program, "/usr/bin/claude");
        // Non-interactive, streaming, our config, permissions bypassed so tool
        // calls don't block on Claude's own allowlist (srelens gates them).
        assert!(cmd.args.contains(&"-p".to_string()));
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w == ["--output-format", "stream-json"]));
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w == ["--mcp-config", "/tmp/mcp.json"]));
        assert!(cmd.args.contains(&"--verbose".to_string()));
        // The prompt is the trailing positional argument.
        assert_eq!(cmd.args.last().unwrap(), "Why is web-0 failing?");
    }

    #[test]
    fn claude_command_uses_only_the_supplied_mcp_config_not_the_users_servers() {
        // Without --strict-mcp-config, the user's global/project MCP servers
        // would load alongside srelens's and the model could call their tools
        // (we bypass Claude's permission prompt), escaping the srelens-only box.
        let cmd = claude_command("/usr/bin/claude", "hi", "/tmp/mcp.json", None);
        assert!(cmd.args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn a_resume_id_adds_the_resume_flag() {
        let cmd = claude_command(
            "/usr/bin/claude",
            "and now?",
            "/tmp/mcp.json",
            Some("sess-123"),
        );
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
        let cmd = claude_command(
            "/usr/bin/claude",
            "Why is web-0 failing?",
            "/tmp/mcp.json",
            None,
        );
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
        assert!(!BASE_SYSTEM_PROMPT
            .to_lowercase()
            .contains("filesystem-access"));
        // Anti-hallucination and operational guardrails.
        assert!(BASE_SYSTEM_PROMPT.contains("live cluster state"));
        assert!(BASE_SYSTEM_PROMPT.contains("Allocatable"));
        assert!(BASE_SYSTEM_PROMPT.contains("podsOnNode"));
        assert!(BASE_SYSTEM_PROMPT.contains("getObject"));
        assert!(BASE_SYSTEM_PROMPT.contains("Schedulable Headroom"));
        assert!(BASE_SYSTEM_PROMPT.contains("Zero-hallucination"));
        assert!(BASE_SYSTEM_PROMPT.contains("Cross-layer incident correlation"));
        assert!(BASE_SYSTEM_PROMPT.contains("TaintManagerEviction"));
        assert!(BASE_SYSTEM_PROMPT.contains("listEndpoints"));
    }

    /// A field a summary lacks is read across the set in one projected list,
    /// not with one full-object fetch per item — the fan-out the old rule
    /// prescribed cost a whole Node (every cached image included) per node.
    #[test]
    fn base_system_prompt_reads_a_missing_field_with_one_projected_list() {
        assert!(BASE_SYSTEM_PROMPT.contains("listResource"));
        assert!(BASE_SYSTEM_PROMPT.contains("`fields`"));
        assert!(BASE_SYSTEM_PROMPT.contains("labelSelector"));
        assert!(BASE_SYSTEM_PROMPT.contains("not call getObject once per item"));
    }

    #[test]
    fn the_mcp_config_is_an_http_server_entry_with_the_bearer_token() {
        let cfg = McpConfig::http("http://127.0.0.1:8765/mcp", "deadbeef");
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(
            v["mcpServers"]["srelens"]["url"],
            "http://127.0.0.1:8765/mcp"
        );
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
            None,
        )
    }

    #[test]
    fn codex_command_disables_the_shell_and_unified_exec_tools() {
        let cmd = codex_cmd();
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w == ["--disable", "shell_tool"]));
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w == ["--disable", "unified_exec"]));
    }

    /// Codex turns these on by default, and `--ignore-user-config` reaches
    /// none of them: on codex-cli 0.149.1 a turn without them offered the
    /// model 221 tools, 210 of them ChatGPT connectors (GitHub commits and
    /// merges, site deploys…), plus web search, image tools, plugins,
    /// sub-agents and the user's own skills (#215, measured live).
    #[test]
    fn codex_command_switches_off_every_tool_surface_beyond_srelens() {
        let cmd = codex_cmd();
        for feature in ["apps", "goals", "view_image", "image_generation", "plugins"] {
            assert!(
                cmd.args.windows(2).any(|w| w == ["--disable", feature]),
                "missing --disable {feature}"
            );
        }
        for value in [
            "agents.enabled=false",
            "web_search=\"disabled\"",
            "skills.include_instructions=false",
        ] {
            assert!(
                cmd.args.windows(2).any(|w| w == ["-c", value]),
                "missing -c {value}"
            );
        }
    }

    #[test]
    fn codex_command_ignores_the_users_config_so_their_mcp_servers_dont_leak_in() {
        // The user's `$CODEX_HOME/config.toml` MCP servers must not stay active
        // alongside srelens's — that would reopen filesystem/shell/network tools.
        let cmd = codex_cmd();
        assert!(cmd.args.contains(&"--ignore-user-config".to_string()));
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
            w[0] == "-c" && w[1] == "mcp_servers.srelens.bearer_token_env_var=\"SRELENS_MCP_TOKEN\""
        }));
    }

    #[test]
    fn codex_command_carries_the_token_only_in_env_never_in_argv() {
        let cmd = codex_cmd();
        assert_eq!(
            cmd.env,
            vec![(CODEX_TOKEN_ENV.to_string(), "deadbeef".to_string())]
        );
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
            None,
        );
        let i_pairs: Vec<&[String]> = cmd.args.windows(2).filter(|w| w[0] == "-i").collect();
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
            None,
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
            None,
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
            None,
        );
        assert_eq!(cmd.args[cmd.args.len() - 2], "--");
        assert_eq!(cmd.args[cmd.args.len() - 1], prompt);
    }

    /// `codex exec resume` keeps nothing of the box from the turn it resumes
    /// (a bare resume got Codex's shell back, measured live), and its own
    /// options take neither `-s` nor `-C`. `exec`'s do, ahead of the
    /// subcommand, so a follow-up carries the fresh turn's box unchanged and
    /// only then `resume <thread>` (#215).
    #[test]
    fn a_codex_follow_up_resumes_its_thread_inside_the_whole_box() {
        let thread = "01a15e68-1088-75f3-b71b-519eef75cd43";
        let prompt = "-s danger-full-access which index?";
        let fresh = codex_command(
            "/usr/bin/codex",
            prompt,
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &[],
            None,
        );
        let resumed = codex_command(
            "/usr/bin/codex",
            prompt,
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &[],
            Some(thread),
        );
        let the_box = &fresh.args[..fresh.args.len() - 2];
        assert_eq!(&resumed.args[..the_box.len()], the_box);
        assert_eq!(
            resumed.args[the_box.len()..],
            ["resume", thread, "--", prompt]
        );
        assert_eq!(resumed.env, fresh.env);
    }

    /// `exec`'s `-i` takes several values, so placed before `resume` it
    /// swallows `resume <thread>` as image paths and Codex silently starts a
    /// new thread (measured live). A follow-up's images are `resume`'s own.
    #[test]
    fn a_codex_follow_up_attaches_its_images_after_the_thread_it_resumes() {
        let thread = "01a15e68-1088-75f3-b71b-519eef75cd43";
        let cmd = codex_command(
            "/usr/bin/codex",
            "and this one?",
            "http://127.0.0.1:8765/mcp",
            "deadbeef",
            "/tmp/srelens-empty-cwd",
            &["/tmp/img1.png".to_string(), "/tmp/img2.png".to_string()],
            Some(thread),
        );
        let resume = cmd.args.iter().position(|a| a == "resume").unwrap();
        assert_eq!(
            cmd.args[resume..],
            [
                "resume",
                thread,
                "-i",
                "/tmp/img1.png",
                "-i",
                "/tmp/img2.png",
                "--",
                "and this one?"
            ]
        );
        assert!(!cmd.args[..resume].contains(&"-i".to_string()));
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
            None,
        );
        let expected = "mcp_servers.srelens.url=\"http://127.0.0.1:8765/mcp?x=\\\"evil\\\"\\\\\"";
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w[0] == "-c" && w[1] == expected));
    }

    fn cursor_cmd() -> AgentCommand {
        cursor_command(
            "/usr/bin/cursor-agent",
            "Why is web-0 failing?",
            "/tmp/srelens-cursor-config",
            "/tmp/srelens-empty-cwd",
            None,
        )
    }

    #[test]
    fn cursor_command_has_the_exact_boxing_flags_in_order() {
        let cmd = cursor_cmd();
        let expected_flags: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--trust",
            "--force",
            "--approve-mcps",
            "--workspace",
            "/tmp/srelens-empty-cwd",
            "--",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        // Everything up to the trailing positional is exactly the boxing flags.
        assert_eq!(cmd.args[..cmd.args.len() - 1], expected_flags[..]);
        // The trailing positional is the guidance preface followed by the prompt.
        let last = cmd.args.last().unwrap();
        assert!(last.starts_with(CURSOR_GUIDANCE));
        assert!(last.ends_with("Why is web-0 failing?"));
        assert_eq!(cmd.program, "/usr/bin/cursor-agent");
    }

    #[test]
    fn cursor_command_forces_and_approves_mcps_but_never_verbose() {
        let cmd = cursor_cmd();
        // `--force` (auto-run MCP calls) + `--approve-mcps` (load the server)
        // are both required; the local-tool deny-list is what keeps `--force`
        // safe. `--verbose` is a Claude-only flag Cursor errors on.
        assert!(cmd.args.contains(&"--force".to_string()));
        assert!(cmd.args.contains(&"--approve-mcps".to_string()));
        assert!(!cmd.args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn cursor_command_ends_options_before_the_prompt_so_it_cannot_inject_flags() {
        let prompt = "--force reply";
        let cmd = cursor_command(
            "/usr/bin/cursor-agent",
            prompt,
            "/tmp/srelens-cursor-config",
            "/tmp/srelens-empty-cwd",
            None,
        );
        assert_eq!(cmd.args[cmd.args.len() - 2], "--");
        // `--` guards it: the flag-looking prompt is the trailing positional
        // (after the guidance preface), never parsed as a flag.
        assert!(cmd.args[cmd.args.len() - 1].ends_with(prompt));
    }

    #[test]
    fn cursor_command_carries_only_the_config_dir_in_env_never_a_token() {
        let cmd = cursor_cmd();
        assert_eq!(
            cmd.env,
            vec![(
                CURSOR_CONFIG_DIR_ENV.to_string(),
                "/tmp/srelens-cursor-config".to_string()
            )]
        );
        assert!(cmd
            .args
            .iter()
            .all(|a| !a.contains("srelens-cursor-config")));
    }

    #[test]
    fn a_resume_id_adds_the_resume_flag_before_the_end_of_options_guard() {
        let cmd = cursor_command(
            "/usr/bin/cursor-agent",
            "and now?",
            "/tmp/srelens-cursor-config",
            "/tmp/srelens-empty-cwd",
            Some("sess-123"),
        );
        assert!(cmd.args.windows(2).any(|w| w == ["--resume", "sess-123"]));
        let dashdash_idx = cmd.args.iter().position(|a| a == "--").unwrap();
        let resume_idx = cmd.args.iter().position(|a| a == "--resume").unwrap();
        assert!(resume_idx < dashdash_idx);
        assert_eq!(cmd.args[cmd.args.len() - 2], "--");
        assert!(cmd.args[cmd.args.len() - 1].ends_with("and now?"));
    }

    #[test]
    fn resume_is_absent_when_none() {
        let cmd = cursor_cmd();
        assert!(!cmd.args.contains(&"--resume".to_string()));
    }

    #[test]
    fn cursor_cli_config_json_denies_local_tools_and_allows_nothing() {
        let v: serde_json::Value = serde_json::from_str(&cursor_cli_config_json()).unwrap();
        assert_eq!(v["permissions"]["allow"], serde_json::json!([]));
        let deny = v["permissions"]["deny"].as_array().unwrap();
        // Comprehensive because `--force` allows any *un-denied* tool — every
        // local-access tool (file read/write/edit, shell, grep, fetch, delete,
        // list, search, web) must be denied. (Cursor's `Glob` is unpermissioned
        // and can't be denied, but it's confined to the empty workspace.)
        for tool in [
            "Read(**)",
            "Write(**)",
            "Edit(**)",
            "Shell(**)",
            "Fetch(**)",
            "Grep(**)",
            "Delete(**)",
            "List(**)",
            "Search(**)",
            "WebFetch(**)",
            "WebSearch(**)",
        ] {
            assert!(
                deny.contains(&serde_json::Value::String(tool.to_string())),
                "deny-list must include {tool}"
            );
        }
    }

    #[test]
    fn cursor_mcp_json_points_at_our_server_with_the_bearer_token() {
        let s = cursor_mcp_json("http://127.0.0.1:8765/mcp", "deadbeef");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(
            v["mcpServers"]["srelens"]["url"],
            "http://127.0.0.1:8765/mcp"
        );
        assert_eq!(
            v["mcpServers"]["srelens"]["headers"]["Authorization"],
            "Bearer deadbeef"
        );
    }

    #[test]
    fn cursor_mcp_json_escapes_a_token_containing_a_quote_and_round_trips() {
        let token = "dead\"beef";
        let s = cursor_mcp_json("http://127.0.0.1:8765/mcp", token);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(
            v["mcpServers"]["srelens"]["headers"]["Authorization"],
            format!("Bearer {token}")
        );
    }

    #[test]
    fn a_batch_shim_is_recognised_whatever_its_case_and_path() {
        assert!(is_batch_shim(r"C:\Program Files\nodejs\claude.CMD"));
        assert!(is_batch_shim(r"C:\Users\First Last\bin\codex.bat"));
        assert!(!is_batch_shim(r"C:\x\codex.exe"));
        assert!(!is_batch_shim(r"C:\x\claude"));
        assert!(!is_batch_shim("/usr/bin/claude"));
    }

    /// A batch shim cannot take a newline in argv (Rust refuses to spawn it),
    /// so the system prompt goes by file and the user's prompt by stdin; the
    /// box itself must be exactly `claude_command`'s.
    #[test]
    fn claude_via_stdin_keeps_the_box_and_takes_no_prompt_in_argv() {
        for resume in [None, Some("sess-123")] {
            let mut expected = claude_command("b", "p", "/c", resume).args;
            expected.truncate(expected.len() - 2); // `--` and the prompt
            let at = expected.iter().position(|a| a == "--append-system-prompt").unwrap();
            expected.splice(at..at + 2, ["--append-system-prompt-file".to_string(), "/s".to_string()]);
            let cmd = claude_command_via_stdin("b", "/c", "/s", resume);
            assert_eq!(cmd.args, expected);
            assert!(!cmd.args.iter().any(|a| a == "--" || a.contains('\n')));
            assert_eq!(cmd.program, "b");
            assert!(cmd.env.is_empty());
        }
    }

    #[test]
    fn codex_via_stdin_keeps_the_box_and_reads_the_prompt_from_stdin() {
        let images = ["/tmp/img1.png".to_string()];
        for resume in [None, Some("thread-1")] {
            let mut expected =
                codex_command("b", "PROMPT", "http://127.0.0.1:8765/mcp", "deadbeef", "/e", &images, resume);
            *expected.args.last_mut().unwrap() = "-".to_string();
            assert_eq!(
                codex_command_via_stdin("b", "http://127.0.0.1:8765/mcp", "deadbeef", "/e", &images, resume),
                expected
            );
        }
    }

    #[test]
    fn model_ids_that_could_be_flags_or_junk_are_refused() {
        for ok in ["sonnet", "gpt-5.6-sol", "claude-fable-5[1m]", "models/gemini-2.5-pro", "a@b:c"] {
            assert!(valid_model_id(ok), "{ok:?} must be accepted");
        }
        for bad in ["", "-x", "--dangerously-skip-permissions", "a b", "a\nb", &"x".repeat(129)] {
            assert!(!valid_model_id(bad), "{bad:?} must be refused");
        }
        assert!(valid_model_id(&"x".repeat(128)));
    }

    /// The model flag goes in straight after the subcommand or `-p`, so
    /// everything else — the box, the `--` guard, the prompt — is untouched.
    #[test]
    fn each_cli_gets_its_own_model_flag_right_after_its_first_arg() {
        let claude = claude_command("c", "hi", "/mcp.json", None);
        let cursor = cursor_cmd();
        let codex = codex_cmd();
        for (kind, plain, first, flag, model) in [
            (AgentKind::Claude, claude, "-p", "--model", "sonnet"),
            (AgentKind::Cursor, cursor, "-p", "--model", "sonnet"),
            (AgentKind::Codex, codex, "exec", "-m", "gpt-5.6-luna"),
        ] {
            let with = plain.clone().with_model(kind, Some(model));
            assert_eq!(with.args[..3], [first, flag, model], "{kind:?}");
            assert_eq!([&with.args[..1], &with.args[3..]].concat(), plain.args, "{kind:?}");
            assert_eq!(with.args.last(), plain.args.last(), "{kind:?}: the prompt stays last");
            assert_eq!((with.program, with.env, with.stdin), (plain.program, plain.env, plain.stdin), "{kind:?}");
        }
    }

    /// A batch shim's commands start with the same first arg, so they get the
    /// flag in the same place.
    #[test]
    fn the_stdin_variants_take_the_model_flag_too() {
        let claude = claude_command_via_stdin("c", "/mcp.json", "/sys.txt", None);
        assert_eq!(claude.with_model(AgentKind::Claude, Some("sonnet")).args[..3], ["-p", "--model", "sonnet"]);
        let codex = codex_command_via_stdin("c", "http://127.0.0.1:1/mcp", "t", "/e", &[], None);
        assert_eq!(codex.with_model(AgentKind::Codex, Some("gpt-5.6-luna")).args[..3], ["exec", "-m", "gpt-5.6-luna"]);
    }

    /// `codex exec resume` has no `-m` of its own, so the model rides in
    /// `exec`'s options, ahead of the subcommand.
    #[test]
    fn codex_puts_the_model_ahead_of_resume() {
        let resumed = codex_command("c", "hi", "http://127.0.0.1:1/mcp", "t", "/e", &[], Some("thread-1"))
            .with_model(AgentKind::Codex, Some("gpt-5.6-luna"));
        let model = resumed.args.iter().position(|a| a == "-m").unwrap();
        let resume = resumed.args.iter().position(|a| a == "resume").unwrap();
        assert!(model < resume, "{:?}", resumed.args);
    }

    #[test]
    fn no_model_a_bad_one_or_the_native_agent_leaves_the_command_alone() {
        for (kind, plain) in [
            (AgentKind::Claude, claude_command("c", "hi", "/mcp.json", None)),
            (AgentKind::Cursor, cursor_cmd()),
            (AgentKind::Codex, codex_cmd()),
        ] {
            assert_eq!(plain.clone().with_model(kind, None), plain, "{kind:?}");
            assert_eq!(plain.clone().with_model(kind, Some("-x")), plain, "{kind:?}");
            assert_eq!(plain.clone().with_model(kind, Some("")), plain, "{kind:?}");
        }
        let plain = codex_cmd();
        assert_eq!(plain.clone().with_model(AgentKind::Srelens, Some("sonnet")), plain);
    }

    #[test]
    fn only_codex_and_cursor_have_a_model_listing_command() {
        assert_eq!(model_list_args(AgentKind::Codex), Some(&["debug", "models"][..]));
        assert_eq!(model_list_args(AgentKind::Cursor), Some(&["models"][..]));
        assert_eq!(model_list_args(AgentKind::Claude), None);
        assert_eq!(model_list_args(AgentKind::Srelens), None);
    }

    #[test]
    fn claude_aliases_are_all_valid_model_ids() {
        assert_eq!(CLAUDE_MODEL_ALIASES, ["fable", "opus", "sonnet", "haiku"]);
        assert!(CLAUDE_MODEL_ALIASES.iter().all(|a| valid_model_id(a)));
    }

    #[test]
    fn prompt_does_not_categorically_attribute_young_pods_to_eviction() {
        assert!(
            !BASE_SYSTEM_PROMPT.contains("they were evicted/rescheduled by Kubernetes"),
            "prompt must not speculate or categorically attribute young pods to eviction"
        );
        assert!(
            BASE_SYSTEM_PROMPT.contains(
                "Young pods (<30m) with 0 restart count do not establish why they were created"
            ),
            "prompt must guide assistant to verify before concluding eviction"
        );
    }
}
