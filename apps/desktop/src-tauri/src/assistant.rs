//! The in-app AI assistant: detect agent CLIs, spawn the chosen one against our
//! own MCP server, and stream normalized events to the WebView.

use std::sync::Arc;

use srelens_agent::adapter::{AgentInfo, AgentKind};
use srelens_agent::claude::parse_line;
use srelens_streams::sink::EventSink;

const CLAUDE_INSTALL: &str = "https://docs.anthropic.com/en/docs/claude-code/setup";
const CODEX_INSTALL: &str = "https://developers.openai.com/codex/cli/";
const CURSOR_INSTALL: &str = "https://docs.cursor.com/en/cli/overview";

fn install_url(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => CLAUDE_INSTALL,
        AgentKind::Codex => CODEX_INSTALL,
        AgentKind::Cursor => CURSOR_INSTALL,
    }
}

/// Build an `AgentInfo`, resolving availability through the injected `resolve`
/// (real code passes a PATH lookup; tests pass a stub).
fn detect(kind: AgentKind, resolve: impl Fn(&str) -> Option<String>) -> AgentInfo {
    let path = resolve(kind.binary());
    AgentInfo {
        kind,
        label: kind.label().to_string(),
        available: path.is_some(),
        path,
        version: None,
        install_url: install_url(kind).to_string(),
    }
}

/// Enumerate every known agent CLI with its availability, for the picker.
#[tauri::command]
pub async fn agent_list() -> Result<Vec<AgentInfo>, String> {
    let paths = srelens_kube::toolbox::SearchPaths::from_env();
    let resolve = |bin: &str| which_on_path(bin, &paths.app_path);
    Ok(vec![
        detect(AgentKind::Claude, resolve),
        detect(AgentKind::Codex, resolve),
        detect(AgentKind::Cursor, resolve),
    ])
}

/// First directory in `path` (a `:`-separated PATH string) that holds an
/// executable `bin`, as an absolute path.
fn which_on_path(bin: &str, path: &str) -> Option<String> {
    path.split(':').filter(|d| !d.is_empty()).find_map(|dir| {
        let candidate = std::path::Path::new(dir).join(bin);
        candidate.is_file().then(|| candidate.to_string_lossy().into_owned())
    })
}

/// Map a sequence of raw stream-json lines to channel emits, returning whether
/// a `TurnDone` event was among them. Pure over the sink, so the parse→emit
/// path is unit-tested without spawning a process; `chat_send` also drives its
/// streaming loop through this so the tested contract is the one actually run.
fn emit_events(
    sink: Arc<dyn EventSink>,
    channel: &str,
    lines: impl IntoIterator<Item = String>,
) -> bool {
    let mut saw_done = false;
    for line in lines {
        for event in parse_line(&line) {
            saw_done |= matches!(event, srelens_agent::event::AgentEvent::TurnDone);
            sink.emit(channel, serde_json::to_value(&event).unwrap());
        }
    }
    saw_done
}

/// Tauri-managed state owning the running chat turns, keyed by session id, so
/// `chat_cancel` can find and kill the right child process.
#[derive(Default)]
pub struct ChatManager {
    children: std::sync::Mutex<std::collections::HashMap<String, tokio::process::Child>>,
}

/// Begin a conversation. Returns a fresh session id; the WebView subscribes to
/// `chat://<id>` before its first `chat_send`.
#[tauri::command]
pub async fn chat_start() -> Result<String, String> {
    Ok(uuid::Uuid::new_v4().to_string())
}

/// Send one user turn. Spawns the agent against our running MCP server and
/// streams `AgentEvent`s on `chat://<session>`.
#[tauri::command]
pub async fn chat_send(
    session: String,
    prompt: String,
    agent_path: String,
    app: tauri::AppHandle,
    mcp: tauri::State<'_, crate::mcp::McpHttpManager>,
    chats: tauri::State<'_, ChatManager>,
) -> Result<(), String> {
    use tauri::Manager;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let token = mcp
        .session_token()
        .ok_or("Start the MCP server in Settings → MCP before using the assistant.")?;
    let url = mcp.status_url().ok_or("MCP server URL unavailable")?;

    // Write the MCP config to a temp file the child reads.
    let cfg = srelens_agent::adapter::McpConfig::http(&url, &token);
    let dir = app.path().temp_dir().map_err(|e| e.to_string())?;
    let cfg_path = dir.join(format!("srelens-mcp-{session}.json"));
    std::fs::write(&cfg_path, serde_json::to_vec(&cfg).unwrap()).map_err(|e| e.to_string())?;

    let cmd = srelens_agent::adapter::claude_command(
        &agent_path,
        &prompt,
        &cfg_path.to_string_lossy(),
        None,
    );
    let mut child = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the agent: {e}"))?;

    let stdout = child.stdout.take().ok_or("no stdout from agent")?;
    let channel = format!("chat://{session}");
    let sink: Arc<dyn EventSink> = Arc::new(crate::sink::TauriSink(app.clone()));
    chats.children.lock().unwrap().insert(session.clone(), child);

    let mut lines = BufReader::new(stdout).lines();
    let mut saw_done = false;
    while let Ok(Some(line)) = lines.next_line().await {
        saw_done |= emit_events(sink.clone(), &channel, std::iter::once(line));
    }
    if !saw_done {
        sink.emit(
            &channel,
            serde_json::to_value(srelens_agent::event::AgentEvent::TurnDone).unwrap(),
        );
    }
    chats.children.lock().unwrap().remove(&session);
    let _ = std::fs::remove_file(&cfg_path);
    Ok(())
}

/// Kill a running turn's agent process, if any.
#[tauri::command]
pub async fn chat_cancel(session: String, chats: tauri::State<'_, ChatManager>) -> Result<(), String> {
    if let Some(mut child) = chats.children.lock().unwrap().remove(&session) {
        let _ = child.start_kill();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_found_binary_is_available_with_its_path() {
        let info = detect(AgentKind::Claude, |_| Some("/usr/bin/claude".into()));
        assert!(info.available);
        assert_eq!(info.path.as_deref(), Some("/usr/bin/claude"));
    }

    #[test]
    fn a_missing_binary_is_unavailable_with_an_install_link() {
        let info = detect(AgentKind::Codex, |_| None);
        assert!(!info.available);
        assert!(info.path.is_none());
        assert!(info.install_url.contains("codex"));
    }

    struct RecordingSink(std::sync::Mutex<Vec<(String, serde_json::Value)>>);
    impl srelens_streams::sink::EventSink for RecordingSink {
        fn emit(&self, channel: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((channel.to_string(), payload));
        }
    }

    #[test]
    fn each_parsed_event_is_emitted_on_the_session_channel() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"result","subtype":"success"}"#,
        ];
        emit_events(sink.clone(), "chat://s1", lines.iter().map(|s| s.to_string()));
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "chat://s1");
        assert_eq!(got[0].1["type"], "textDelta");
        assert_eq!(got[1].1["type"], "turnDone");
    }
}
