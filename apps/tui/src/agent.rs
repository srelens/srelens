//! In-process autonomous SRE agent for srectl.
//!
//! Drives srelens's MCP tools directly in-process via `srelens_mcp::stdio::handle_request`
//! and runs the multi-turn agentic loop (`srelens_llm::agent_loop::run`), emitting streaming
//! events back into Ratatui's event loop.

use async_trait::async_trait;
use serde_json::{json, Value};
use srelens_agent::adapter::{AgentCommand, AgentKind};
use srelens_agent::event::AgentEvent;
use srelens_agent::turn::CursorChatsInUse;
use srelens_kube::client_cache::ClientCache;
use srelens_llm::types::{ToolDef, Turn};
use srelens_llm::{LlmError, ToolCallResult, ToolInvoker};
use srelens_mcp::McpServer;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::event::AppEvent;

/// A `ToolInvoker` backed by an in-process `McpServer`.
pub struct McpToolInvoker {
    server: Arc<McpServer>,
    aliases: Mutex<HashMap<String, String>>,
}

impl McpToolInvoker {
    pub fn new(server: Arc<McpServer>) -> Self {
        Self {
            server,
            aliases: Default::default(),
        }
    }
}

fn provider_safe_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn assign_alias(aliases: &mut HashMap<String, String>, id: &str) -> String {
    let mut alias = provider_safe_name(id);
    while aliases.get(&alias).is_some_and(|existing| existing != id) {
        alias.push('_');
    }
    aliases.insert(alias.clone(), id.to_string());
    alias
}

fn tool_def_from_json(v: &Value) -> ToolDef {
    ToolDef {
        name: v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        description: v
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        input_schema: v
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" })),
        read_only: v
            .get("annotations")
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[async_trait]
impl ToolInvoker for McpToolInvoker {
    async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let resp =
            srelens_mcp::stdio::handle_request(&self.server, &req, srelens_mcp::Transport::Http)
                .await
                .ok_or_else(|| LlmError::Api("tools/list returned no response".into()))?;
        let tools = resp
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut aliases = self.aliases.lock().unwrap();
        Ok(tools
            .iter()
            .map(|v| {
                let mut def = tool_def_from_json(v);
                def.name = assign_alias(&mut aliases, &def.name);
                def
            })
            .collect())
    }

    async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolCallResult, LlmError> {
        let real_name = {
            let aliases = self.aliases.lock().unwrap();
            aliases
                .get(name)
                .cloned()
                .or_else(|| {
                    let k8s_name = format!("k8s_{name}");
                    aliases.get(&k8s_name).cloned()
                })
                .or_else(|| {
                    if name.starts_with("k8s_") {
                        Some(name.replacen('_', ".", 1))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| name.to_string())
        };
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": real_name, "arguments": args },
        });
        let resp =
            srelens_mcp::stdio::handle_request(&self.server, &req, srelens_mcp::Transport::Http)
                .await
                .ok_or_else(|| LlmError::Api("tools/call returned no response".into()))?;
        if let Some(err) = resp.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("tool call failed");
            return Ok(ToolCallResult {
                content: msg.to_string(),
                is_error: true,
                denied: false,
            });
        }
        let result = resp.get("result");
        let is_error = result
            .and_then(|r| r.get("isError"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let denied = result
            .and_then(|r| r.get("_meta"))
            .and_then(|m| m.get("srelens/denied"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let content = result
            .and_then(|r| r.get("content"))
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Ok(ToolCallResult {
            content,
            is_error,
            denied,
        })
    }
}

pub fn build_mcp_server(
    cache: Arc<ClientCache>,
    kubeconfig_paths: Vec<PathBuf>,
    policy: Arc<dyn srelens_mcp::policy::ConfirmPolicy>,
) -> Arc<McpServer> {
    let registry = srelens_registry::build_registry_with_paths(cache, kubeconfig_paths);
    let server = McpServer::new(Arc::new(registry))
        .with_policy(policy)
        .with_kind_resolver(srelens_registry::kind_resolver());
    Arc::new(server)
}

pub fn enrich_prompt_with_context(
    prompt: &str,
    active_context: &str,
    active_namespace: &str,
    argo_hub_context: Option<&str>,
) -> String {
    let hub_clause = match argo_hub_context {
        Some(hub) if !hub.is_empty() && hub != active_context => {
            format!(
                ", ArgoCD Hub Context: \"{}\"]\n\
                [NOTE: In a GitOps Hub-and-Spoke topology, ArgoCD Application CRDs and GitOps controllers reside on the Hub cluster (\"{}\"). When diagnosing ArgoCD applications, sync status, or GitOps rollouts for workloads in cluster \"{}\", query context \"{}\" (e.g. `k8s.listCustomResource` or `k8s.getCustomResource` with group `argoproj.io`, kind `Application`, context \"{}\").",
                hub, hub, active_context, hub, hub
            )
        }
        _ => String::new(),
    };
    if hub_clause.is_empty() {
        format!(
            "[Active Kubernetes Context: \"{}\", Namespace: \"{}\"]\n\n{}",
            active_context, active_namespace, prompt
        )
    } else {
        format!(
            "[Active Kubernetes Context: \"{}\", Namespace: \"{}\"{}]\n\n{}",
            active_context, active_namespace, hub_clause, prompt
        )
    }
}

/// Seconds the turn has gone without a sign of life. A turn held up on the
/// user's approval is waiting, not stalled, so every tick while a prompt is
/// open counts as a sign of life: the model then gets its whole limit from the
/// answer, not from the event before the question.
fn quiet_secs(elapsed: u64, last_activity: &std::sync::atomic::AtomicU64, waiting: usize) -> u64 {
    if waiting > 0 {
        last_activity.store(elapsed, std::sync::atomic::Ordering::Relaxed);
    }
    elapsed.saturating_sub(last_activity.load(std::sync::atomic::Ordering::Relaxed))
}

/// The ceiling on any one turn, prompts or not.
const MAX_TURN_SECS: u64 = 900;

/// Resolves once the turn has gone `limit` seconds without a sign of life
/// (see [`quiet_secs`]), or has run for [`MAX_TURN_SECS`].
async fn stalled(
    start: std::time::Instant,
    last_activity: &std::sync::atomic::AtomicU64,
    waiting: &std::sync::atomic::AtomicUsize,
    limit: u64,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let elapsed = start.elapsed().as_secs();
        let quiet = quiet_secs(
            elapsed,
            last_activity,
            waiting.load(std::sync::atomic::Ordering::SeqCst),
        );
        if quiet >= limit || elapsed >= MAX_TURN_SECS {
            return;
        }
    }
}

/// What the Assistant shows for one agent event, as `(ActionResult title,
/// payload)` pairs tagged with the turn's context. Usage shows nothing here:
/// the turn reports it once, with its duration.
pub fn agent_event_actions(ev: &AgentEvent, ctx: &str) -> Vec<(String, String)> {
    let at = |action: &str, payload: String| (format!("{action}:{ctx}"), payload);
    match ev {
        AgentEvent::TextDelta { text } => vec![at("ai_chunk", text.clone())],
        AgentEvent::Thinking { text } => vec![at("ai_status", text.clone())],
        AgentEvent::ToolCallStart { id, tool, args } => {
            let args = match args {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            vec![
                at("ai_status", format!("Executing {tool}...")),
                at("ai_tool_start", format!("{id}|{tool}|{args}")),
            ]
        }
        AgentEvent::ToolResult { id, status, .. } => {
            let status = match status {
                srelens_agent::event::ToolStatus::Ok => "ok",
                srelens_agent::event::ToolStatus::Error => "error",
                srelens_agent::event::ToolStatus::Denied => "denied",
            };
            vec![at("ai_tool_done", format!("{id}|{status}"))]
        }
        AgentEvent::Error { message } => vec![at("ai_chunk", format!("\n[Error: {message}]"))],
        AgentEvent::Usage { .. } | AgentEvent::TurnDone => vec![],
    }
}

/// `waiting` is the count of approval prompts open for this turn
/// ([`crate::consent::TuiConsent::waiting`]); while it is above zero the
/// inactivity timeout stands still. The 15-minute ceiling does not.
pub async fn run_native_agent_turn(
    config: srelens_llm::ProviderConfig,
    invoker: Arc<McpToolInvoker>,
    history: Arc<tokio::sync::Mutex<Vec<Turn>>>,
    prompt: String,
    active_context: String,
    active_namespace: String,
    argo_hub_context: Option<String>,
    event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    timeout_seconds: u32,
    waiting: Arc<std::sync::atomic::AtomicUsize>,
) {
    let provider = srelens_llm::HttpProvider::new(config);
    let start_time = std::time::Instant::now();
    let out_chars = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let out_chars_clone = out_chars.clone();
    let event_tx_clone = event_tx.clone();

    let enriched_prompt = enrich_prompt_with_context(
        &prompt,
        &active_context,
        &active_namespace,
        argo_hub_context.as_deref(),
    );

    let prior_turns = {
        let lock = history.lock().await;
        lock.clone()
    };

    let last_activity = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let last_act_clone = last_activity.clone();
    let start_time_copy = start_time;

    let real_usage = Arc::new(std::sync::Mutex::new(None));
    let real_usage_clone = real_usage.clone();

    let ctx_tag = active_context.clone();
    let mut on_event = move |ev: AgentEvent| {
        last_act_clone.store(
            start_time_copy.elapsed().as_secs(),
            std::sync::atomic::Ordering::Relaxed,
        );
        match &ev {
            AgentEvent::TextDelta { text } => {
                out_chars_clone.fetch_add(text.len(), std::sync::atomic::Ordering::Relaxed);
            }
            AgentEvent::Usage {
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                total_tokens,
            } => {
                *real_usage_clone.lock().unwrap() = Some((
                    *prompt_tokens,
                    *completion_tokens,
                    *cached_tokens,
                    *total_tokens,
                ));
            }
            _ => {}
        }
        for (title, payload) in agent_event_actions(&ev, &ctx_tag) {
            let _ = event_tx_clone.send(AppEvent::ActionResult {
                title,
                result: Ok(payload),
            });
        }
    };

    let filtered_invoker = srelens_llm::invoker::FilteredToolInvoker::new(
        invoker.clone(),
        srelens_llm::invoker::CORE_SRE_TOOLS,
    );
    let run_fut = srelens_llm::agent_loop::run(
        &provider,
        &filtered_invoker,
        prior_turns,
        enriched_prompt,
        &mut on_event,
    );
    let watchdog = stalled(
        start_time,
        &last_activity,
        &waiting,
        timeout_seconds.max(5) as u64,
    );

    let result = tokio::select! {
        res = run_fut => res,
        _ = watchdog => {
            let msg = format!(
                "AI assistant turn timed out after {} seconds. You can increase the timeout in settings (<Ctrl+s>).",
                timeout_seconds
            );
            let _ = event_tx.send(AppEvent::ActionResult {
                title: format!("ai_chunk:{}", active_context),
                result: Ok(format!("\n[Error: {}]", msg)),
            });
            Err(srelens_llm::error::LlmError::Api(msg))
        }
    };

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let (prompt_tokens, comp_tokens, cached_tokens, total_tokens) = resolve_token_usage(
        real_usage.lock().unwrap().take(),
        prompt.len(),
        out_chars.load(std::sync::atomic::Ordering::Relaxed),
    );
    let payload = format!(
        "{}|{}|{}|{}|{}",
        prompt_tokens, comp_tokens, cached_tokens, total_tokens, duration_ms
    );
    let _ = event_tx.send(AppEvent::ActionResult {
        title: format!("ai_usage:{}", active_context),
        result: Ok(payload),
    });

    match result {
        Ok(updated_turns) => {
            {
                let mut lock = history.lock().await;
                *lock = srelens_llm::history::distill_and_trim_history(updated_turns, 40);
            }
            let _ = event_tx.send(AppEvent::ActionResult {
                title: format!("ai_done:{}", active_context),
                result: Ok(String::new()),
            });
        }
        Err(err) => {
            let _ = event_tx.send(AppEvent::ActionResult {
                title: format!("ai_chunk:{}", active_context),
                result: Err(format!("AI Agent Error: {}", err)),
            });
            let _ = event_tx.send(AppEvent::ActionResult {
                title: format!("ai_done:{}", active_context),
                result: Ok(String::new()),
            });
        }
    }
}

/// Global process-lifetime root directory for Cursor CLI agent conversations.
/// Created on first use; removed by [`cleanup_process_cursor_chats`] when
/// srectl exits (a static is never dropped). A killed srectl leaves it
/// behind, owner-only.
static CURSOR_CHATS_ROOT: std::sync::Mutex<Option<tempfile::TempDir>> = std::sync::Mutex::new(None);

pub fn process_cursor_chats_root() -> std::path::PathBuf {
    let mut lock = CURSOR_CHATS_ROOT.lock().unwrap();
    if lock.is_none() {
        let dir = tempfile::Builder::new()
            .prefix("srelens-cursor-chats-")
            .tempdir()
            .unwrap_or_else(|_| tempfile::tempdir().expect("failed to create cursor chats root"));
        *lock = Some(dir);
    }
    lock.as_ref().unwrap().path().to_path_buf()
}

pub fn cleanup_process_cursor_chats() {
    let mut lock = CURSOR_CHATS_ROOT.lock().unwrap();
    if let Some(dir) = lock.take() {
        let _ = dir.close();
    }
}

/// Synchronization lock per conversation ID to serialize workspace creation, execution,
/// and teardown across rapid turns or turn cancellation retries.
pub fn conversation_lock(conv_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
        >,
    > = std::sync::OnceLock::new();
    let map = LOCKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = map.lock().unwrap();
    if guard.len() > 100 {
        guard.retain(|_, v| v.strong_count() > 0);
    }
    if let Some(weak) = guard.get(conv_id) {
        if let Some(arc) = weak.upgrade() {
            return arc;
        }
    }
    let arc = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    guard.insert(conv_id.to_string(), std::sync::Arc::downgrade(&arc));
    arc
}

/// The Cursor chats a turn in this process is using (see `CursorChatsInUse`).
static CURSOR_CHATS: std::sync::OnceLock<CursorChatsInUse> = std::sync::OnceLock::new();

/// One turn of a local agent CLI — Claude Code, Codex or Cursor — boxed to
/// srelens's MCP tools, every write approved in srectl.
pub struct CliTurn {
    pub kind: AgentKind,
    pub binary: PathBuf,
    /// `None` runs the CLI's own default model.
    pub model: Option<String>,
    /// Cursor's API key; it reaches the agent in its environment only.
    pub cursor_key: Option<String>,
    pub query: String,
    pub active_ctx: String,
    pub active_ns: String,
    pub argo_hub_context: Option<String>,
    pub cache: Arc<ClientCache>,
    pub kubeconfig_paths: Vec<PathBuf>,
    pub event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    pub timeout_seconds: u32,
    /// The Assistant's conversation: it names the Cursor chat, and every
    /// `ai_session` event carries it so a stale one can be told apart.
    pub conversation_id: String,
    /// The CLI's session id from the conversation's previous turn.
    pub resume: Option<String>,
    /// Where Cursor chats are kept ([`process_cursor_chats_root`]).
    pub chat_root: PathBuf,
}

/// `cmd` with Cursor's API key in the child's environment — never in argv,
/// where any local account could list it, and never in srectl's own.
pub fn with_cursor_key(mut cmd: AgentCommand, key: Option<&str>) -> AgentCommand {
    if let Some(key) = key {
        cmd.env.push(("CURSOR_API_KEY".into(), key.into()));
    }
    cmd
}

/// The agent's process. Dropped before it exited — the turn's task aborted,
/// or it timed out — it ends the whole tree, so nothing is left running.
struct Agent(tokio::process::Child);

impl Drop for Agent {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            srelens_agent::turn::kill_tree(&mut self.0);
        }
    }
}

/// How much of the agent's stderr a turn keeps, for the error it shows.
const STDERR_TAIL: usize = 8 * 1024;

/// The end of an agent's stderr for the transcript: its last few non-empty
/// lines joined by ` | `, at most 1 KiB. Not just the last line — a Node
/// CLI's crash ends with `Node.js v22…`, its cause a line or two above.
fn stderr_summary(tail: &[u8]) -> String {
    let text = String::from_utf8_lossy(tail);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let joined = lines[lines.len().saturating_sub(5)..].join(" | ");
    let start = joined
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| joined.len() - i <= 1024)
        .unwrap_or(joined.len());
    joined[start..].to_string()
}

/// Run one agent-CLI turn: a loopback MCP server of its own whose gated calls
/// ask the user in srectl ([`crate::consent::TuiConsent`]; an open prompt
/// stops the inactivity clock), the CLI spawned in the turn's private
/// directory, and its stream shown in the Assistant. The session to resume
/// next goes back as `ai_session:<ctx>` `"<conversation_id>|<id>"`; every
/// turn ends with `ai_done:<ctx>`.
pub async fn run_cli_agent_turn(turn: CliTurn) {
    let CliTurn {
        kind,
        binary,
        model,
        cursor_key,
        query,
        active_ctx: ctx,
        active_ns,
        argo_hub_context,
        cache,
        kubeconfig_paths,
        event_tx: tx,
        timeout_seconds,
        conversation_id,
        resume,
        chat_root,
    } = turn;
    let send = |action: &str, result: Result<String, String>| {
        let _ = tx.send(AppEvent::ActionResult {
            title: format!("{action}:{ctx}"),
            result,
        });
    };
    // Before the agent ran: no `ai_session`, so the conversation keeps the
    // session it had.
    let fail = |message: String| {
        send("ai_chunk", Err(message));
        send("ai_done", Ok(String::new()));
    };
    let resume = resume.filter(|id| srelens_agent::turn::valid_resume_id(id));
    let started = std::time::Instant::now();

    // One turn at a time per conversation, so an aborted turn's workspace is
    // gone before the next one lays it out again.
    let conv_lock = conversation_lock(&conversation_id);
    let _conv = conv_lock.lock().await;

    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => return fail(format!("Failed to bind local MCP port: {e}")),
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => return fail(format!("Failed to get local MCP address: {e}")),
    };
    let url = format!("http://127.0.0.1:{port}/mcp");
    let token = srelens_mcp::auth::Token::generate();
    let token_str = token.as_str().to_string();
    // Dropped when the turn ends, however it ends, which stops the server and
    // denies every prompt it still has open.
    let (_turn_alive, turn_over) = tokio::sync::watch::channel(());
    let consent =
        crate::consent::TuiConsent::new(tx.clone(), kind.label(), &ctx).for_turn(turn_over.clone());
    let waiting = consent.waiting();
    let registry = srelens_registry::build_registry_with_paths(cache, kubeconfig_paths);
    let server = McpServer::new(Arc::new(registry))
        .with_policy(Arc::new(consent))
        .with_kind_resolver(srelens_registry::kind_resolver());
    let mut stopped = turn_over;
    tokio::spawn(srelens_mcp::http::serve_http_with_shutdown(
        server,
        listener,
        async move { while stopped.changed().await.is_ok() {} },
        token,
    ));

    let prompt = enrich_prompt_with_context(&query, &ctx, &active_ns, argo_hub_context.as_deref());
    let binary = binary.to_string_lossy();
    let session = uuid::Uuid::new_v4().to_string();
    let temp_dir = std::env::temp_dir();
    // Holds the turn's files (the bearer token among them) until the agent is done.
    let prepared = match srelens_agent::turn::prepare_turn(&srelens_agent::turn::TurnSpec {
        kind,
        binary: &binary,
        prompt: &prompt,
        mcp_url: &url,
        token: &token_str,
        resume: resume.as_deref(),
        model: model.as_deref(),
        session: &session,
        conversation: Some(&conversation_id),
        cursor_root: &chat_root,
        cursor_chats: CURSOR_CHATS.get_or_init(Default::default),
        temp_dir: &temp_dir,
        image_paths: &[],
    }) {
        Ok(p) => p,
        Err(e) => return fail(format!("Could not set up the {} turn: {e}", kind.label())),
    };
    let parse_line: fn(&str) -> Vec<AgentEvent> = match kind {
        AgentKind::Claude => srelens_agent::claude::parse_line,
        AgentKind::Codex => srelens_agent::codex::parse_line,
        // `prepare_turn` refused the native agent above.
        AgentKind::Cursor | AgentKind::Srelens => srelens_agent::cursor::parse_line,
    };

    let cmd = with_cursor_key(prepared.cmd.clone(), cursor_key.as_deref());
    let spawned = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .envs(cmd.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&prepared.cwd)
        .stdin(if cmd.stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut agent = match spawned {
        Ok(child) => Agent(child),
        Err(e) => {
            send(
                "ai_session",
                Ok(format!("{conversation_id}|{}", resume.unwrap_or_default())),
            );
            let program = kind.binary().unwrap_or("the agent");
            return fail(format!("Failed to launch {program}: {e}"));
        }
    };
    if let (Some(input), Some(mut stdin)) = (cmd.stdin, agent.0.stdin.take()) {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            // An agent that stops reading says so in its exit status.
            let _ = stdin.write_all(input.as_bytes()).await;
            // Dropped here: the agent reads the end of its prompt.
        });
    }
    // Drained alongside stdout, or an agent writing past the pipe buffer
    // would block on it and stall its stdout too.
    let stderr = agent.0.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut tail = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = stderr.read(&mut buf).await {
                tail.extend_from_slice(&buf[..n]);
                tail.drain(..tail.len().saturating_sub(STDERR_TAIL));
            }
            tail
        })
    });
    let stdout = agent.0.stdout.take().expect("the agent's stdout is piped");

    let last_activity = std::sync::atomic::AtomicU64::new(0);
    let mut captured: Option<String> = None;
    let mut usage: Option<(usize, usize, usize, usize, Option<u64>)> = None;
    let mut out_chars = 0;
    let mut said_anything = false;
    let read = async {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            last_activity.store(
                started.elapsed().as_secs(),
                std::sync::atomic::Ordering::Relaxed,
            );
            if captured.is_none() {
                captured = srelens_agent::stream_session_id(&line);
                if let Some(id) = &captured {
                    send("ai_session", Ok(format!("{conversation_id}|{id}")));
                }
            }
            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                usage = crate::app::extract_usage_metrics(&v).or(usage);
            }
            for ev in parse_line(&line) {
                match &ev {
                    AgentEvent::TextDelta { text } => {
                        out_chars += text.len();
                        said_anything = true;
                    }
                    AgentEvent::Error { .. } => said_anything = true,
                    AgentEvent::Usage {
                        prompt_tokens,
                        completion_tokens,
                        cached_tokens,
                        total_tokens,
                    } => {
                        usage = Some((
                            *prompt_tokens,
                            *completion_tokens,
                            *cached_tokens,
                            *total_tokens,
                            None,
                        ))
                    }
                    _ => {}
                }
                for (title, payload) in agent_event_actions(&ev, &ctx) {
                    let _ = tx.send(AppEvent::ActionResult {
                        title,
                        result: Ok(payload),
                    });
                }
            }
        }
    };
    let timed_out = tokio::select! {
        _ = read => false,
        _ = stalled(started, &last_activity, &waiting, timeout_seconds.max(5) as u64) => true,
    };

    // An agent that closed its stdout but will not exit is ended, as is one
    // that timed out, so its stderr closes too.
    let status = if timed_out {
        None
    } else {
        tokio::time::timeout(std::time::Duration::from_secs(5), agent.0.wait())
            .await
            .ok()
            .and_then(Result::ok)
    };
    if status.is_none() {
        srelens_agent::turn::kill_tree(&mut agent.0);
    }
    let crashed = !status.is_some_and(|s| s.success());
    let tail = match stderr {
        Some(task) => tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let label = kind.label();
    if timed_out {
        send(
            "ai_chunk",
            Ok(format!(
                "\n[Error: {label} timed out after {timeout_seconds} seconds. You can increase the timeout in settings (<Ctrl+s>) or try another model.]"
            )),
        );
    } else if !said_anything {
        // A clean exit too: a CLI that changed its format, or reports a
        // failure on stderr and exits 0, would otherwise end the turn blank.
        let why = stderr_summary(&tail);
        let why = if why.is_empty() {
            format!("{label} finished without an answer")
        } else {
            why
        };
        send("ai_chunk", Ok(format!("\n[Error: {why}]")));
    }

    let next = srelens_agent::next_resume_id(captured, resume, crashed);
    send(
        "ai_session",
        Ok(format!("{conversation_id}|{}", next.unwrap_or_default())),
    );
    let (p, c, cached, total) = resolve_token_usage(
        usage.map(|(p, c, cached, total, _)| (p, c, cached, total)),
        prompt.len(),
        out_chars,
    );
    let ms = usage
        .and_then(|u| u.4)
        .unwrap_or_else(|| started.elapsed().as_millis() as u64);
    send("ai_usage", Ok(format!("{p}|{c}|{cached}|{total}|{ms}")));
    send("ai_done", Ok(String::new()));
}

/// Resolve token counts for an assistant turn: prefers provider-reported real usage,
/// falling back to character-based heuristic estimates if real usage is absent (e.g. timeout).
pub fn resolve_token_usage(
    real_usage: Option<(usize, usize, usize, usize)>,
    prompt_len: usize,
    output_chars: usize,
) -> (usize, usize, usize, usize) {
    if let Some(u) = real_usage {
        u
    } else {
        let prompt_est = (prompt_len + 200) / 4;
        let comp_est = if output_chars == 0 {
            0
        } else {
            (output_chars / 4).max(1)
        };
        (prompt_est, comp_est, 0, prompt_est + comp_est)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_mcp::policy::{AlwaysDeny, ConfirmPolicy, ConsentKind, ConsentRequest, Decision};

    /// A policy that writes down what it was asked and says no.
    #[derive(Default)]
    struct Recording(Mutex<Vec<ConsentRequest>>);

    #[async_trait]
    impl ConfirmPolicy for Recording {
        async fn confirm(&self, request: &ConsentRequest) -> Decision {
            self.0.lock().unwrap().push(request.clone());
            Decision::Denied("recorded".into())
        }
    }

    /// The call as the model makes it: the alias, not the dotted id. A
    /// nonexistent kubeconfig is fine, the policy answers before any cluster call.
    async fn call_with_recording(tool: &str, args: Value) -> (ToolCallResult, Vec<ConsentRequest>) {
        let policy = Arc::new(Recording::default());
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let invoker = McpToolInvoker::new(build_mcp_server(cache, vec![], policy.clone()));
        let result = invoker
            .call_tool(tool, &args)
            .await
            .expect("the call completes");
        let seen = policy.0.lock().unwrap().clone();
        (result, seen)
    }

    #[tokio::test]
    async fn a_gated_tool_call_from_the_native_agent_reaches_the_policy() {
        let (result, seen) = call_with_recording(
            "k8s_deletePod",
            json!({ "context": "c", "namespace": "n", "name": "p" }),
        )
        .await;

        assert!(result.denied, "a refusal is marked denied: {result:?}");
        assert_eq!(seen.len(), 1, "the policy was asked once: {seen:?}");
        assert_eq!(seen[0].tool, "k8s.deletePod");
        assert_eq!(seen[0].kind, ConsentKind::Destructive);
    }

    #[tokio::test]
    async fn a_secret_read_from_the_native_agent_reaches_the_policy() {
        let (result, seen) = call_with_recording(
            "k8s_getSecret",
            json!({ "context": "c", "namespace": "n", "name": "s" }),
        )
        .await;

        assert!(result.denied, "a refusal is marked denied: {result:?}");
        assert_eq!(seen.len(), 1, "the policy was asked once: {seen:?}");
        assert_eq!(seen[0].tool, "k8s.getSecret");
        assert_eq!(seen[0].kind, ConsentKind::SensitiveRead);
    }

    #[test]
    fn a_turn_held_up_on_the_user_is_not_counted_as_quiet() {
        use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
        let last_activity = AtomicU64::new(0);

        // Nobody asked: the clock runs from the last event.
        assert_eq!(quiet_secs(40, &last_activity, 0), 40);
        // A prompt is open: the agent is waiting on the user, not stalled.
        assert_eq!(quiet_secs(100, &last_activity, 1), 0);
        assert_eq!(last_activity.load(Relaxed), 100);
        // The user answers: the model gets the full limit from the answer, not
        // from the event before the prompt.
        assert_eq!(quiet_secs(103, &last_activity, 0), 3);
    }

    #[test]
    fn agent_events_map_to_the_assistants_existing_titles() {
        use srelens_agent::event::ToolStatus;
        let pair = |t: &str, p: &str| (t.to_string(), p.to_string());
        let cases = [
            (
                AgentEvent::TextDelta { text: "hi".into() },
                vec![pair("ai_chunk:ctx", "hi")],
            ),
            (
                AgentEvent::Thinking { text: "hmm".into() },
                vec![pair("ai_status:ctx", "hmm")],
            ),
            (
                AgentEvent::ToolCallStart {
                    id: "t1".into(),
                    tool: "k8s.listPods".into(),
                    args: json!({ "namespace": "n" }),
                },
                vec![
                    pair("ai_status:ctx", "Executing k8s.listPods..."),
                    pair("ai_tool_start:ctx", r#"t1|k8s.listPods|{"namespace":"n"}"#),
                ],
            ),
            (
                AgentEvent::ToolResult {
                    id: "t1".into(),
                    status: ToolStatus::Denied,
                    summary: None,
                },
                vec![pair("ai_tool_done:ctx", "t1|denied")],
            ),
            (
                AgentEvent::ToolResult {
                    id: "t2".into(),
                    status: ToolStatus::Ok,
                    summary: None,
                },
                vec![pair("ai_tool_done:ctx", "t2|ok")],
            ),
            // The runner sends usage once, with the turn's duration.
            (
                AgentEvent::Usage {
                    prompt_tokens: 1,
                    completion_tokens: 2,
                    cached_tokens: 0,
                    total_tokens: 3,
                },
                vec![],
            ),
            (AgentEvent::TurnDone, vec![]),
            (
                AgentEvent::Error {
                    message: "m".into(),
                },
                vec![pair("ai_chunk:ctx", "\n[Error: m]")],
            ),
        ];
        for (ev, want) in cases {
            assert_eq!(agent_event_actions(&ev, "ctx"), want, "{ev:?}");
        }
    }

    #[test]
    fn the_cursor_key_rides_only_in_the_childs_env() {
        let before = std::env::var_os("CURSOR_API_KEY");
        let cmd = srelens_agent::adapter::AgentCommand {
            program: "cursor-agent".into(),
            args: vec!["-p".into(), "--".into(), "why?".into()],
            env: vec![],
            stdin: None,
        };

        let keyed = with_cursor_key(cmd.clone(), Some("cur-1"));

        assert!(keyed
            .env
            .contains(&("CURSOR_API_KEY".to_string(), "cur-1".to_string())));
        assert!(
            !keyed.args.iter().any(|a| a.contains("cur-1")),
            "{:?}",
            keyed.args
        );
        assert_eq!(
            std::env::var_os("CURSOR_API_KEY"),
            before,
            "srectl's own env is left alone"
        );
        assert_eq!(with_cursor_key(cmd.clone(), None), cmd);
    }

    #[tokio::test]
    async fn test_mcp_tool_invoker_lists_and_maps_aliases() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let server = build_mcp_server(cache, vec![], Arc::new(AlwaysDeny));
        let invoker = McpToolInvoker::new(server);

        let tools = invoker.list_tools().await.expect("list_tools succeeds");
        assert!(!tools.is_empty(), "tools list should contain capabilities");

        // Check provider-safe naming
        for t in &tools {
            assert!(
                t.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "tool name '{}' must be provider safe",
                t.name
            );
        }

        // Must include basic core k8s capabilities
        let has_list_pods = tools.iter().any(|t| t.name.contains("listPods"));
        let has_list_ns = tools.iter().any(|t| t.name.contains("listNamespaces"));
        assert!(has_list_pods, "should include listPods capability");
        assert!(has_list_ns, "should include listNamespaces capability");

        // Verify every CORE_SRE_TOOLS entry is served by the registry (matching safe underscore alias)
        let tool_names: std::collections::HashSet<String> =
            tools.into_iter().map(|t| t.name).collect();
        for &expected in srelens_llm::invoker::CORE_SRE_TOOLS {
            let safe_alias = expected.replace('.', "_");
            assert!(
                tool_names.contains(expected) || tool_names.contains(&safe_alias),
                "allowlisted tool '{expected}' (or alias '{safe_alias}') must exist in registry"
            );
        }
    }

    #[test]
    fn test_enrich_prompt_without_argo_hub() {
        let p = enrich_prompt_with_context("Check pods", "prod-cluster", "default", None);
        assert_eq!(
            p,
            "[Active Kubernetes Context: \"prod-cluster\", Namespace: \"default\"]\n\nCheck pods"
        );
    }

    #[test]
    fn test_enrich_prompt_with_same_argo_hub_as_active_context() {
        let p = enrich_prompt_with_context("Check pods", "tools", "default", Some("tools"));
        assert_eq!(
            p,
            "[Active Kubernetes Context: \"tools\", Namespace: \"default\"]\n\nCheck pods"
        );
    }

    #[test]
    fn test_enrich_prompt_with_remote_argo_hub_context() {
        let p = enrich_prompt_with_context(
            "Where is the argo application?",
            "spoke-prod",
            "search",
            Some("tools-mgmt"),
        );
        assert!(p.contains("Active Kubernetes Context: \"spoke-prod\""));
        assert!(p.contains("ArgoCD Hub Context: \"tools-mgmt\""));
        assert!(p.contains("spoke-prod"));
        assert!(p.contains("tools-mgmt"));
        assert!(p.ends_with("Where is the argo application?"));
    }

    #[test]
    fn test_resolve_token_usage_prefers_real_usage() {
        let real = Some((1200, 350, 800, 1550));
        let usage = resolve_token_usage(real, 100, 40);
        assert_eq!(usage, (1200, 350, 800, 1550));
    }

    #[test]
    fn test_resolve_token_usage_falls_back_to_estimate() {
        let usage = resolve_token_usage(None, 200, 40);
        assert_eq!(usage, (100, 10, 0, 110));

        let usage_short = resolve_token_usage(None, 200, 2);
        assert_eq!(usage_short, (100, 1, 0, 101));

        let usage_zero = resolve_token_usage(None, 200, 0);
        assert_eq!(usage_zero, (100, 0, 0, 100));
    }
}
