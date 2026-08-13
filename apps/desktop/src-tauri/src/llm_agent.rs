//! The native (in-process) agent turn. Builds a provider client from the stored
//! config and drives srelens's MCP tools through the SAME consent/audit server
//! the CLI agents reach over HTTP — but called directly in-process via
//! `handle_request`, so read-only tools auto-run and destructive ones raise the
//! identical confirm dialog and land in the same audit log.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use srelens_agent::event::AgentEvent;
use srelens_llm::types::ToolDef;
use srelens_llm::{LlmError, ToolCallResult, ToolInvoker};
use srelens_mcp::McpServer;
use srelens_streams::sink::EventSink;
use tauri::Manager;

use crate::assistant::ChatManager;

/// How many conversation turns to keep per session before trimming the oldest.
/// Bounds both memory and the tokens re-sent on every follow-up.
const MAX_HISTORY_TURNS: usize = 40;

/// How many sessions' histories to retain at once. Nothing ever deletes an
/// in-memory history (New chat just mints a fresh session id, and deleting a
/// saved session only removes the disk record), so without this cap every
/// abandoned transcript — including large MCP results — would accumulate for
/// the process lifetime.
const MAX_HISTORY_SESSIONS: usize = 32;

/// In-memory native-agent conversation history, keyed by chat session id, so
/// follow-up messages in the same session carry the earlier exchange as context
/// (the CLIs are stateless per turn; the native agent runs in-process, so it
/// can hold this). Not persisted: reopening a saved session starts a fresh
/// native context. A "New chat" mints a new session id, so its history is empty.
///
/// Stored as a write-ordered list (every turn rewrites its session's entry, so
/// order == recency): beyond `MAX_HISTORY_SESSIONS` the least-recently-written
/// session is evicted. Linear scans are fine at this size.
#[derive(Default)]
pub struct NativeHistory(std::sync::Mutex<Vec<(String, Vec<srelens_llm::types::Turn>)>>);

impl NativeHistory {
    fn get(&self, session: &str) -> Vec<srelens_llm::types::Turn> {
        let entries = self.0.lock().unwrap();
        entries.iter().find(|(id, _)| id == session).map(|(_, turns)| turns.clone()).unwrap_or_default()
    }

    fn set(&self, session: String, turns: Vec<srelens_llm::types::Turn>) {
        let mut entries = self.0.lock().unwrap();
        entries.retain(|(id, _)| *id != session);
        entries.push((session, turns));
        if entries.len() > MAX_HISTORY_SESSIONS {
            let excess = entries.len() - MAX_HISTORY_SESSIONS;
            entries.drain(0..excess);
        }
    }
}

/// Trim a conversation to at most `max` turns, dropping the oldest. A
/// conversation must begin on a user turn (an assistant or tool-result turn with
/// no preceding user message is invalid for every provider), so after removing
/// the oldest turn we keep dropping any leading non-user turns.
fn trim_history(
    mut turns: Vec<srelens_llm::types::Turn>,
    max: usize,
) -> Vec<srelens_llm::types::Turn> {
    use srelens_llm::types::Turn;
    while turns.len() > max {
        turns.remove(0);
        while turns.first().is_some_and(|t| !matches!(t, Turn::User(_))) {
            turns.remove(0);
        }
    }
    turns
}

/// A `ToolInvoker` backed by an in-process `McpServer`. Each call is a JSON-RPC
/// request handed to `handle_request`, which applies the consent policy and
/// audit before touching the registry.
///
/// Registry ids are dotted (`k8s.listPods`), but Anthropic and OpenAI only
/// accept `[A-Za-z0-9_-]` in function names, so `list_tools` advertises a
/// provider-safe alias for each tool and `call_tool` translates the alias back
/// to the registry id before the JSON-RPC call.
pub struct McpToolInvoker {
    server: Arc<McpServer>,
    /// Provider-safe alias → registry id, populated by `list_tools`.
    aliases: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl McpToolInvoker {
    pub fn new(server: Arc<McpServer>) -> Self {
        Self { server, aliases: Default::default() }
    }
}

/// Rewrite a registry id into a name every provider accepts: any character
/// outside `[A-Za-z0-9_-]` becomes `_` (`k8s.listPods` → `k8s_listPods`).
fn provider_safe_name(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') { c } else { '_' }).collect()
}

/// Pick an unused alias for `id` and record it in `aliases`. Two ids that
/// sanitize identically (e.g. `k8s.x` and `k8s_x`) get distinct aliases by
/// suffixing, so a model call can never be routed to the wrong tool.
fn assign_alias(aliases: &mut std::collections::HashMap<String, String>, id: &str) -> String {
    let mut alias = provider_safe_name(id);
    while aliases.get(&alias).is_some_and(|existing| existing != id) {
        alias.push('_');
    }
    aliases.insert(alias.clone(), id.to_string());
    alias
}

#[async_trait]
impl ToolInvoker for McpToolInvoker {
    async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let resp = srelens_mcp::stdio::handle_request(&self.server, &req, srelens_mcp::Transport::Http)
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
        // The model calls the advertised alias; the registry knows only the
        // dotted id. An unknown name passes through unchanged and surfaces as
        // the server's own "unknown tool" error below.
        let name = self.aliases.lock().unwrap().get(name).cloned().unwrap_or_else(|| name.to_string());
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": args },
        });
        let resp = srelens_mcp::stdio::handle_request(&self.server, &req, srelens_mcp::Transport::Http)
            .await
            .ok_or_else(|| LlmError::Api("tools/call returned no response".into()))?;
        // A JSON-RPC error (unknown tool / bad params) is fed back as a failed
        // result so the model can correct itself rather than aborting the turn.
        if let Some(err) = resp.get("error") {
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("tool call failed");
            return Ok(ToolCallResult { content: msg.to_string(), is_error: true, denied: false });
        }
        let result = resp.get("result");
        let is_error = result.and_then(|r| r.get("isError")).and_then(Value::as_bool).unwrap_or(false);
        // A consent refusal is `isError: true` PLUS the `_meta` marker the
        // server sets on that path — mapping it to `denied` lets the loop
        // report "the user declined" instead of a failed execution.
        let denied = result
            .and_then(|r| r.get("_meta"))
            .and_then(|m| m.get("srelens/denied"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // MCP results carry `content: [{type:"text", text}]`; concatenate the
        // text parts (a denied destructive call comes back here as isError:true
        // with the reason text, which we feed straight to the model).
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
        Ok(ToolCallResult { content, is_error, denied })
    }
}

fn tool_def_from_json(v: &Value) -> ToolDef {
    ToolDef {
        name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        description: v.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
        input_schema: v.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
        read_only: v
            .get("annotations")
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// The directory holding the native agent's config (settings + fallback keys).
pub fn llm_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("llm"))
}

/// Run one native-agent turn end to end, streaming `AgentEvent`s on the
/// session's `chat://` channel. The loop runs as a task whose `AbortHandle` is
/// parked in `ChatManager`, so `chat_cancel` stops it just like a CLI child.
pub async fn run_native_agent(
    app: tauri::AppHandle,
    chats: &ChatManager,
    session: String,
    prompt: String,
    has_images: bool,
    turn: u64,
) -> Result<(), String> {
    let channel = format!("chat://{session}");
    let sink: Arc<dyn EventSink> = Arc::new(crate::sink::TauriSink(app.clone()));
    let emit = |ev: AgentEvent| sink.emit(&channel, serde_json::to_value(&ev).unwrap());

    // A Stop aimed at this turn can arrive before we do (the frontend awaits
    // channel subscription before invoking `chat_send`); honor it instead of
    // launching — but still close the turn with a `TurnDone`, since the
    // frontend only settles and persists on a terminal event. A stale
    // pending-cancel from a previous turn has a different generation and is
    // dropped by the same take.
    if chats.take_pending_cancel(&session, turn) {
        emit(AgentEvent::TurnDone);
        return Ok(());
    }

    // Image input isn't wired for the native agent yet; tell the user rather
    // than silently dropping an attachment the composer still displays.
    if has_images {
        emit(AgentEvent::Error {
            message: "image attachments aren't supported by the srelens agent yet".to_string(),
        });
    }

    // Resolve the provider config (default provider + its key/model) up front;
    // a missing key or model ends the turn with a clear, actionable message.
    let dir = match llm_dir(&app) {
        Ok(d) => d,
        Err(e) => {
            emit(AgentEvent::Error { message: e });
            emit(AgentEvent::TurnDone);
            return Ok(());
        }
    };
    let settings = crate::llm_config::load_settings(&dir.join("settings.json"));
    let vault = app.state::<std::sync::Arc<crate::vault::Vault>>();
    let cfg = match crate::llm_config::provider_config(&vault, &settings, settings.default_provider) {
        Some(c) if !c.model.is_empty() => c,
        Some(_) => {
            emit(AgentEvent::Error {
                message: "Choose a model for the srelens agent in Settings → Assistant.".into(),
            });
            emit(AgentEvent::TurnDone);
            return Ok(());
        }
        None => {
            emit(AgentEvent::Error {
                message: "Add an API key for the srelens agent in Settings → Assistant.".into(),
            });
            emit(AgentEvent::TurnDone);
            return Ok(());
        }
    };

    // Build the in-process MCP server (identical consent/audit to the CLIs).
    let pending = app.state::<Arc<crate::mcp_confirm::Pending>>();
    let audit = app.state::<crate::mcp::McpAuditPath>();
    let prompts = app.state::<crate::mcp::McpPromptsDir>();
    let mcp = app.state::<crate::mcp::McpHttpManager>();
    let server = Arc::new(mcp.build_server(&app, pending.inner(), &audit.0, &prompts.0));
    let invoker = McpToolInvoker::new(server);
    let provider = srelens_llm::HttpProvider::new(cfg);

    // Seed the turn with this session's prior conversation so follow-ups have
    // context. `run` returns the conversation to continue from next time.
    let history_state = app.state::<NativeHistory>();
    let prior = history_state.get(&session);

    // Run the loop as a task so `chat_cancel` can abort it. `run` emits a
    // `TurnDone` on every non-cancelled path; on cancellation the task is dropped
    // mid-flight, so we emit the closing `TurnDone` ourselves below.
    let task_sink = sink.clone();
    let task_channel = channel.clone();
    let handle = tokio::spawn(async move {
        let mut on_event =
            |ev: AgentEvent| task_sink.emit(&task_channel, serde_json::to_value(&ev).unwrap());
        srelens_llm::agent_loop::run(&provider, &invoker, prior, prompt, &mut on_event).await
    });

    chats.register_native(session.clone(), handle.abort_handle());
    // Honor a Stop that landed during prep, before the task was registered.
    if chats.take_pending_cancel(&session, turn) {
        handle.abort();
    }
    let joined = handle.await;
    chats.unregister_native(&session);

    match joined {
        // Normal/handled finish: persist the continued conversation (trimmed).
        Ok(Ok(updated)) => history_state.set(session, trim_history(updated, MAX_HISTORY_TURNS)),
        // `run` failed — setup (tools couldn't be listed) or a transport error
        // mid-stream (connection died before the provider's terminal marker).
        // Surface it and keep the prior history untouched; any partial deltas
        // already streamed stay visible but are not treated as a real turn.
        Ok(Err(e)) => {
            emit(AgentEvent::Error { message: e.to_string() });
            emit(AgentEvent::TurnDone);
        }
        // Cancelled (aborted) mid-flight: re-enable the composer; history for the
        // discarded turn is left as it was.
        Err(_) => emit(AgentEvent::TurnDone),
    }
    Ok(())
}

// ---- Tauri commands backing the Settings → Assistant section ----

use srelens_llm::types::{ModelInfo, ProviderKind};
use srelens_llm::Provider;

use crate::llm_config::LlmSettings;

fn settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(llm_dir(app)?.join("settings.json"))
}

/// The native agent's non-secret settings (default provider, chosen models,
/// custom base URLs). Keys are never returned here — see `llm_key_status`.
#[tauri::command]
pub async fn llm_get_settings(app: tauri::AppHandle) -> Result<LlmSettings, String> {
    Ok(crate::llm_config::load_settings(&settings_path(&app)?))
}

#[tauri::command]
pub async fn llm_set_settings(app: tauri::AppHandle, settings: LlmSettings) -> Result<(), String> {
    crate::llm_config::save_settings(&settings_path(&app)?, &settings)
}

/// Store an API key for `provider` in the encrypted secrets vault.
#[tauri::command]
pub async fn llm_set_key(app: tauri::AppHandle, provider: ProviderKind, key: String) -> Result<(), String> {
    let vault = app.state::<std::sync::Arc<crate::vault::Vault>>();
    crate::llm_config::set_key(&vault, &llm_dir(&app)?, provider, key.trim())
}

#[tauri::command]
pub async fn llm_clear_key(app: tauri::AppHandle, provider: ProviderKind) -> Result<(), String> {
    let vault = app.state::<std::sync::Arc<crate::vault::Vault>>();
    crate::llm_config::clear_key(&vault, &llm_dir(&app)?, provider)
}

/// Which providers currently have a key configured — so Settings can show a
/// "key set" state without ever returning the secret itself.
#[tauri::command]
pub async fn llm_key_status(app: tauri::AppHandle) -> Result<Vec<ProviderKind>, String> {
    let vault = app.state::<std::sync::Arc<crate::vault::Vault>>();
    Ok(crate::llm_config::all_providers()
        .into_iter()
        .filter(|k| crate::llm_config::has_key(&vault, *k))
        .collect())
}

/// Fetch the model list for a provider from its API, using the stored key and
/// base URL. `base_url` overrides the stored one so the OpenAI-compatible setup
/// flow can fetch models against a just-typed URL before Settings are saved.
#[tauri::command]
pub async fn llm_list_models(
    app: tauri::AppHandle,
    provider: ProviderKind,
    base_url: Option<String>,
) -> Result<Vec<ModelInfo>, String> {
    let vault = app.state::<std::sync::Arc<crate::vault::Vault>>();
    let settings = crate::llm_config::load_settings(&settings_path(&app)?);
    let mut cfg = crate::llm_config::provider_config(&vault, &settings, provider)
        .ok_or("Add an API key for this provider first.")?;
    if let Some(url) = base_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()) {
        cfg.base_url = url;
    }
    srelens_llm::HttpProvider::new(cfg).list_models().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_defs_come_from_the_mcp_tools_list_shape_with_the_read_only_hint() {
        let v = json!({
            "name": "k8s_listPods",
            "description": "list pods",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true, "destructiveHint": false }
        });
        let t = tool_def_from_json(&v);
        assert_eq!(t.name, "k8s_listPods");
        assert_eq!(t.description, "list pods");
        assert!(t.read_only);
        assert_eq!(t.input_schema["type"], "object");
    }

    #[test]
    fn a_tool_without_a_read_only_hint_defaults_to_destructive_side() {
        let v = json!({ "name": "k8s_deletePod", "description": "delete" });
        let t = tool_def_from_json(&v);
        assert!(!t.read_only);
        // Missing inputSchema is coerced to an empty object schema.
        assert_eq!(t.input_schema["type"], "object");
    }

    /// `srelens_mcp` writes the denial text marker and `srelens_agent` parses
    /// it, but neither crate depends on the other — this is the one place both
    /// are visible, so it pins the duplicated constants to the same value.
    #[test]
    fn the_denial_text_marker_matches_between_the_mcp_server_and_the_cli_parsers() {
        assert_eq!(srelens_mcp::stdio::DENIED_PREFIX, srelens_agent::event::DENIED_PREFIX);
    }

    #[test]
    fn dotted_registry_ids_alias_to_provider_safe_names_and_back() {
        let mut aliases = std::collections::HashMap::new();
        // Anthropic/OpenAI reject `.` in function names; the alias replaces it.
        assert_eq!(assign_alias(&mut aliases, "k8s.listPods"), "k8s_listPods");
        assert_eq!(aliases.get("k8s_listPods").map(String::as_str), Some("k8s.listPods"));
        // Already-safe ids pass through and re-listing is stable.
        assert_eq!(assign_alias(&mut aliases, "ping"), "ping");
        assert_eq!(assign_alias(&mut aliases, "k8s.listPods"), "k8s_listPods");
    }

    #[test]
    fn ids_that_sanitize_identically_get_distinct_aliases() {
        let mut aliases = std::collections::HashMap::new();
        assert_eq!(assign_alias(&mut aliases, "k8s.scale"), "k8s_scale");
        let other = assign_alias(&mut aliases, "k8s_scale");
        assert_ne!(other, "k8s_scale");
        assert_eq!(aliases.get(&other).map(String::as_str), Some("k8s_scale"));
    }

    #[test]
    fn native_history_round_trips_per_session_and_defaults_to_empty() {
        use srelens_llm::types::Turn;
        let h = NativeHistory::default();
        assert!(h.get("s1").is_empty());
        h.set("s1".into(), vec![Turn::User("hi".into())]);
        assert_eq!(h.get("s1"), vec![Turn::User("hi".into())]);
        // Sessions are isolated.
        assert!(h.get("s2").is_empty());
    }

    #[test]
    fn abandoned_session_histories_are_evicted_beyond_the_cap() {
        use srelens_llm::types::Turn;
        let h = NativeHistory::default();
        for i in 0..=MAX_HISTORY_SESSIONS {
            h.set(format!("s{i}"), vec![Turn::User(format!("q{i}"))]);
        }
        // One over the cap: the least-recently-written session is gone, the
        // newest survives.
        assert!(h.get("s0").is_empty(), "oldest session should have been evicted");
        assert!(!h.get(&format!("s{MAX_HISTORY_SESSIONS}")).is_empty());
        // Rewriting an old session refreshes its recency instead of duplicating.
        h.set("s1".into(), vec![Turn::User("again".into())]);
        h.set("extra".into(), vec![Turn::User("x".into())]);
        assert_eq!(h.get("s1"), vec![Turn::User("again".into())]);
    }

    #[test]
    fn trimming_keeps_the_newest_turns_and_starts_on_a_user_turn() {
        use srelens_llm::types::Turn;
        let a = |t: &str| Turn::Assistant { text: t.into(), tool_calls: Vec::new() };
        let turns = vec![
            Turn::User("q1".into()),
            a("a1"),
            Turn::User("q2".into()),
            a("a2"),
            Turn::User("q3".into()),
            a("a3"),
        ];
        // Cap at 3: dropping the oldest would leave [a1, q2, a2, ...]; the
        // leading assistant turn is invalid, so it's dropped too — the result
        // begins on a user turn and is within the cap.
        let trimmed = trim_history(turns, 3);
        assert!(trimmed.len() <= 3);
        assert!(matches!(trimmed.first(), Some(Turn::User(_))));
        assert_eq!(trimmed.last(), Some(&a("a3")));
    }

    #[test]
    fn trimming_a_short_history_is_a_no_op() {
        use srelens_llm::types::Turn;
        let turns = vec![Turn::User("q".into())];
        assert_eq!(trim_history(turns.clone(), 40), turns);
    }
}
