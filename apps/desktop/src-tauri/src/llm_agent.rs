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

/// How many conversation turns to keep per chat before trimming the oldest.
/// Bounds both the history file and the tokens re-sent on every follow-up.
const MAX_HISTORY_TURNS: usize = 40;

/// The chat a native turn's history is kept under: the saved conversation's
/// id, which a reopened conversation keeps while its session is new (#952),
/// else the session (a turn with nothing saved).
fn chat_id<'a>(session: &'a str, conversation: Option<&'a str>) -> &'a str {
    conversation.unwrap_or(session)
}

/// Where the native agent keeps each chat's history, so a follow-up carries
/// the earlier exchange — across a reopened conversation and a restart, as
/// the CLIs keep theirs (#215). The per-user data dir, like Cursor's chats:
/// a saved conversation's history goes only with the chat
/// (`chat_history_delete`), and the launch sweep removes the rest.
pub(crate) fn native_state_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("native-chats"))
}

/// `<dir>/<chat>.json`. A session or conversation id is `[A-Za-z0-9._-]`
/// by the time a turn runs, so with the suffix (even for `.` or `..`) this
/// is always one plain file in `dir`.
pub(crate) fn native_chat_path(dir: &std::path::Path, chat: &str) -> std::path::PathBuf {
    dir.join(format!("{chat}.json"))
}

/// The history kept for `chat`: none for a new chat, and none — logged, not a
/// failed turn — for one that cannot be read back.
fn load_history(dir: &std::path::Path, chat: &str) -> Vec<srelens_llm::types::Turn> {
    let path = native_chat_path(dir, chat);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!("could not read {}: {error}", path.display());
            return Vec::new();
        }
    };
    serde_json::from_str(&raw).unwrap_or_else(|error| {
        log::warn!("starting {chat} afresh: {} is unreadable: {error}", path.display());
        Vec::new()
    })
}

/// What a native turn continues from. Its history id travels as the
/// conversation's resume id, as the CLIs' do: `run_native_agent` returns it,
/// and switching agents drops it. So only a turn handed its own id continues
/// the history; any other starts afresh, because the history kept would skip
/// whatever another agent answered in between.
fn prior_history(dir: &std::path::Path, chat: &str, resume: Option<&str>) -> Vec<srelens_llm::types::Turn> {
    if resume == Some(chat) {
        load_history(dir, chat)
    } else {
        Vec::new()
    }
}

/// At launch, before any turn runs: see `sweep_native_chats`.
pub fn sweep_native_state(app: &tauri::AppHandle) {
    let dirs = native_state_dir(app).and_then(|dir| Ok((dir, crate::assistant_history::resolve_sessions_dir(app)?)));
    match dirs {
        Ok((dir, sessions)) => sweep_native_chats(&dir, |id| crate::assistant_history::is_saved(&sessions, id)),
        Err(error) => log::warn!("could not find the srelens agent's histories to sweep them: {error}"),
    }
}

/// Remove the histories no saved conversation holds — a turn's with nothing
/// saved, or one whose save never landed. They hold tool results, so a
/// failure is logged: it is why they are still on disk.
fn sweep_native_chats(dir: &std::path::Path, saved: impl Fn(&str) -> bool) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            log::warn!("could not list the srelens agent's histories in {}: {error}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let held = path.extension().is_some_and(|ext| ext == "json")
            && path.file_stem().and_then(|stem| stem.to_str()).is_some_and(&saved);
        if !held {
            if let Err(error) = std::fs::remove_file(&path) {
                log::warn!("could not delete {}: {error}", path.display());
            }
        }
    }
}

/// Keep `chat`'s history, owner-only: it holds tool results.
fn save_history(dir: &std::path::Path, chat: &str, turns: &[srelens_llm::types::Turn]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let raw = serde_json::to_string(turns).map_err(std::io::Error::other)?;
    crate::assistant_history::write_private(&native_chat_path(dir, chat), &raw)
}

/// Distill and trim conversation history using the shared srelens_llm engine.
fn trim_history(
    turns: Vec<srelens_llm::types::Turn>,
    max: usize,
) -> Vec<srelens_llm::types::Turn> {
    srelens_llm::history::distill_and_trim_history(turns, max)
}

/// A `ToolInvoker` backed by an in-process `McpServer`. Each call is a JSON-RPC
/// request handed to `handle_request`, which applies the consent policy and
/// audit before touching the registry.
///
/// Registry ids are dotted (`k8s.listPods`), but Anthropic and OpenAI only
/// accept `[A-Za-z0-9_-]` in function names, so `list_tools` advertises a
/// provider-safe alias for each tool and `call_tool` translates the alias back
/// to the registry id before the JSON-RPC call.
///
/// It also carries the chat it answers for (#393). There is no bearer token
/// in-process to authenticate, so the invoker names the caller itself, and a
/// gated call it makes reaches the confirm prompt as that chat's.
pub struct McpToolInvoker {
    server: Arc<McpServer>,
    /// Provider-safe alias → registry id, populated by `list_tools`.
    aliases: std::sync::Mutex<std::collections::HashMap<String, String>>,
    caller: Option<srelens_mcp::policy::Caller>,
}

impl McpToolInvoker {
    pub fn new(server: Arc<McpServer>) -> Self {
        Self { server, aliases: Default::default(), caller: None }
    }

    pub fn with_caller(mut self, caller: srelens_mcp::policy::Caller) -> Self {
        self.caller = Some(caller);
        self
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
        let resp = srelens_mcp::stdio::handle_request_as(
            &self.server,
            &req,
            srelens_mcp::Transport::Http,
            self.caller.clone(),
        )
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
/// `conversation` is the saved conversation's id, which names its history
/// (see `chat_id`), and `resume` what the last turn returned (see
/// `prior_history`). Returns the id to resume from next time.
#[allow(clippy::too_many_arguments)]
pub async fn run_native_agent(
    app: tauri::AppHandle,
    chats: &ChatManager,
    session: String,
    conversation: Option<String>,
    resume: Option<String>,
    prompt: String,
    has_images: bool,
    turn: u64,
) -> Result<Option<String>, String> {
    let channel = format!("chat://{session}");
    let sink: Arc<dyn EventSink> = Arc::new(crate::sink::TauriSink(app.clone()));
    let emit = |ev: AgentEvent| sink.emit(&channel, serde_json::to_value(&ev).unwrap());

    // A Stop aimed at this turn can arrive before we do (the frontend awaits
    // channel subscription before invoking `chat_send`); honor it instead of
    // launching — but still close the turn with a `TurnDone`, since the
    // frontend only settles and persists on a terminal event. A stale
    // pending-cancel from a previous turn has a different generation and is
    // dropped by the same take.
    // Nothing runs on any early return below, so the stored id is handed back
    // as it came, like `chat_send` does for a cancel.
    if chats.take_pending_cancel(&session, turn) {
        emit(AgentEvent::TurnDone);
        return Ok(resume);
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
            return Ok(resume);
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
            return Ok(resume);
        }
        None => {
            emit(AgentEvent::Error {
                message: "Add an API key for the srelens agent in Settings → Assistant.".into(),
            });
            emit(AgentEvent::TurnDone);
            return Ok(resume);
        }
    };

    // Build the in-process MCP server (identical consent/audit to the CLIs).
    let pending = app.state::<Arc<crate::mcp_confirm::Pending>>();
    let audit = app.state::<crate::mcp::McpAuditPath>();
    let prompts = app.state::<crate::mcp::McpPromptsDir>();
    let mcp = app.state::<crate::mcp::McpHttpManager>();
    let server = Arc::new(mcp.build_server(&app, pending.inner(), &audit.0, &prompts.0));
    let invoker = McpToolInvoker::new(server).with_caller(srelens_mcp::policy::Caller::Chat(session.clone()));
    let filtered_invoker = srelens_llm::invoker::FilteredToolInvoker::new(invoker, srelens_llm::invoker::CORE_SRE_TOOLS);
    let provider = srelens_llm::HttpProvider::new(cfg);

    // Seed the turn with this chat's prior conversation so follow-ups have
    // context. `run` returns the conversation to continue from next time.
    let history_dir = match native_state_dir(&app) {
        Ok(d) => d,
        Err(e) => {
            emit(AgentEvent::Error { message: e });
            emit(AgentEvent::TurnDone);
            return Ok(resume);
        }
    };
    let chat = chat_id(&session, conversation.as_deref()).to_string();
    let prior = prior_history(&history_dir, &chat, resume.as_deref());

    // Run the loop as a task so `chat_cancel` can abort it. `run` emits a
    // `TurnDone` on every non-cancelled path; on cancellation the task is dropped
    // mid-flight, so we emit the closing `TurnDone` ourselves below.
    let task_sink = sink.clone();
    let task_channel = channel.clone();
    let handle = tokio::spawn(async move {
        let mut on_event =
            |ev: AgentEvent| task_sink.emit(&task_channel, serde_json::to_value(&ev).unwrap());
        srelens_llm::agent_loop::run(&provider, &filtered_invoker, prior, prompt, &mut on_event).await
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
        // The answer is already on screen, so a failure to keep it is logged:
        // it only costs the next turn its context.
        Ok(Ok(updated)) => {
            if let Err(error) = save_history(&history_dir, &chat, &trim_history(updated, MAX_HISTORY_TURNS)) {
                log::warn!("could not keep the srelens agent's history for {chat}: {error}");
            }
            Ok(Some(chat))
        }
        // `run` failed — setup (tools couldn't be listed) or a transport error
        // mid-stream (connection died before the provider's terminal marker).
        // Surface it and keep the prior history untouched; any partial deltas
        // already streamed stay visible but are not treated as a real turn.
        Ok(Err(e)) => {
            emit(AgentEvent::Error { message: e.to_string() });
            emit(AgentEvent::TurnDone);
            Ok(resume)
        }
        // Cancelled (aborted) mid-flight: re-enable the composer; history for the
        // discarded turn is left as it was.
        Err(_) => {
            emit(AgentEvent::TurnDone);
            Ok(resume)
        }
    }
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

    /// A reopened conversation gets a new session, and a restart empties
    /// memory, so the native agent's history is kept under the conversation,
    /// on disk — as Claude, Codex and Cursor keep theirs (#215).
    #[test]
    fn a_reopened_native_conversation_finds_the_history_an_earlier_run_kept() {
        use srelens_llm::types::{ToolCall, ToolOutcome, Turn};
        let dir = tempfile::tempdir().unwrap();
        let turns = vec![
            Turn::User("why is qa mongo slow?".into()),
            Turn::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "k8s_listPods".into(),
                    arguments: json!({"namespace": "qa"}),
                    thought_signature: Some("sig".into()),
                }],
            },
            Turn::ToolResults(vec![ToolOutcome {
                id: "c1".into(),
                name: "k8s_listPods".into(),
                content: "mongo-0 Running".into(),
                is_error: false,
            }]),
            Turn::Assistant { text: "fix the quota lookup's index".into(), tool_calls: Vec::new() },
        ];
        save_history(dir.path(), chat_id("run-1", Some("conv-1")), &turns).unwrap();

        // Reopened after a restart: a new session, nothing in memory.
        assert_eq!(load_history(dir.path(), chat_id("run-2", Some("conv-1"))), turns);
        assert!(load_history(dir.path(), chat_id("run-2", Some("conv-2"))).is_empty());
    }

    /// Switching agents drops the conversation's resume id (`chooseAgent`;
    /// classic keeps one only for the agent that issued it), so the next
    /// srelens-agent turn arrives without its own id and starts afresh: the
    /// history it kept would skip the turns another agent answered in
    /// between. Its follow-ups, and a reopened conversation, hand it back.
    #[test]
    fn a_native_turn_resumes_its_history_only_when_handed_its_id() {
        use srelens_llm::types::Turn;
        let dir = tempfile::tempdir().unwrap();
        let turns = vec![Turn::User("why is qa mongo slow?".into())];
        save_history(dir.path(), "conv-1", &turns).unwrap();

        assert_eq!(prior_history(dir.path(), "conv-1", Some("conv-1")), turns);
        assert!(prior_history(dir.path(), "conv-1", None).is_empty(), "after an agent switch");
        assert!(prior_history(dir.path(), "conv-1", Some("01a15e95-a7fa-7843-8127-54338b81787c")).is_empty(), "another agent's id");
    }

    #[test]
    fn a_restart_keeps_only_the_histories_a_saved_conversation_holds() {
        let dir = tempfile::tempdir().unwrap();
        for chat in ["saved", "one-shot"] {
            std::fs::write(native_chat_path(dir.path(), chat), "[]").unwrap();
        }

        sweep_native_chats(dir.path(), |id| id == "saved");

        assert!(native_chat_path(dir.path(), "saved").exists());
        assert!(!native_chat_path(dir.path(), "one-shot").exists(), "a history no saved conversation holds goes at launch");
    }

    #[test]
    fn a_native_turn_with_no_saved_conversation_keeps_its_history_under_its_session() {
        assert_eq!(chat_id("run-1", None), "run-1");
    }

    #[test]
    fn a_native_history_that_cannot_be_read_starts_the_conversation_afresh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("conv-1.json"), "{ not json").unwrap();
        assert!(load_history(dir.path(), "conv-1").is_empty());
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

    /// #393: a native turn's gated call reaches the consent policy as its own
    /// chat's — there is no token in-process, so the invoker says it.
    #[tokio::test]
    async fn a_native_turn_s_gated_call_is_its_own_chat_s() {
        use std::sync::Mutex;
        #[allow(clippy::type_complexity)]
        struct Saw(Arc<Mutex<Option<Option<srelens_mcp::policy::Caller>>>>);
        #[async_trait::async_trait]
        impl srelens_mcp::policy::ConfirmPolicy for Saw {
            async fn confirm(&self, request: &srelens_mcp::policy::ConsentRequest) -> srelens_mcp::policy::Decision {
                *self.0.lock().unwrap() = Some(request.caller.clone());
                srelens_mcp::policy::Decision::Approved
            }
        }
        let saw = Arc::new(Mutex::new(None));
        let mut reg = srelens_capability::Registry::new();
        let mut cap = srelens_capability::Capability::read_only("danger", "destructive", |_| async { Ok(json!({})) });
        cap.annotations = srelens_capability::Annotations::MUTATING;
        reg.register(cap);
        let server = McpServer::new(Arc::new(reg)).with_policy(Arc::new(Saw(saw.clone())));
        let invoker = McpToolInvoker::new(Arc::new(server))
            .with_caller(srelens_mcp::policy::Caller::Chat("sess-7".into()));

        invoker.call_tool("danger", &json!({})).await.expect("call");
        assert_eq!(*saw.lock().unwrap(), Some(Some(srelens_mcp::policy::Caller::Chat("sess-7".into()))));
    }
}
