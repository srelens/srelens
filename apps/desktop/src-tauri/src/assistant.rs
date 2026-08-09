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

/// Last non-empty line of a byte buffer, trimmed — used to turn a raw stderr
/// tail into a one-line error message instead of dumping a whole traceback
/// into the transcript.
fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Close out a turn whose stdout stream ended without a `TurnDone`: if the
/// child also exited non-zero, that's a crash, not a clean finish, so an
/// `Error` (carrying the stderr tail) is emitted before the synthetic
/// `TurnDone` that always re-enables the drawer's input. A stream that DID
/// see `TurnDone` needs nothing further — the agent already closed its own
/// turn. Pure over the sink, so the crash-reporting decision is unit-tested
/// without spawning a process.
fn finish_turn(sink: &dyn EventSink, channel: &str, saw_done: bool, crashed: bool, stderr_tail: &[u8]) {
    if saw_done {
        return;
    }
    if crashed {
        let tail = last_line(stderr_tail);
        let message = if tail.is_empty() {
            "the agent process exited unexpectedly".to_string()
        } else {
            tail
        };
        sink.emit(
            channel,
            serde_json::to_value(srelens_agent::event::AgentEvent::Error { message }).unwrap(),
        );
    }
    sink.emit(
        channel,
        serde_json::to_value(srelens_agent::event::AgentEvent::TurnDone).unwrap(),
    );
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

/// Removes the wrapped path when dropped. The MCP config file `chat_send`
/// writes for a turn carries the bearer token in cleartext, so it must be
/// removed on every exit — including an early `?` after a failed spawn — not
/// just the happy path a bare `remove_file` at the end of the function would
/// cover.
struct TempFile(std::path::PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    let token = mcp
        .session_token()
        .ok_or("Start the MCP server in Settings → MCP before using the assistant.")?;
    let url = mcp.status_url().ok_or("MCP server URL unavailable")?;

    let channel = format!("chat://{session}");
    let sink: Arc<dyn EventSink> = Arc::new(crate::sink::TauriSink(app.clone()));

    // Write the MCP config to a temp file the child reads by path (never on
    // argv). `_cfg_guard`'s Drop removes it on every subsequent exit from
    // this function, success or error.
    let cfg = srelens_agent::adapter::McpConfig::http(&url, &token);
    let dir = app.path().temp_dir().map_err(|e| e.to_string())?;
    let cfg_path = dir.join(format!("srelens-mcp-{session}.json"));
    std::fs::write(&cfg_path, serde_json::to_vec(&cfg).unwrap()).map_err(|e| e.to_string())?;
    let _cfg_guard = TempFile(cfg_path.clone());

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

    // `Stdio::piped()` above guarantees this is `Some`; still handled rather
    // than unwrapped so a future refactor that drops the `piped()` call
    // degrades to a clean turn-ending error instead of a silently hung UI —
    // every post-spawn exit re-enables input via a `TurnDone`.
    let Some(stdout) = child.stdout.take() else {
        let _ = child.start_kill();
        let message = "the agent process had no stdout".to_string();
        sink.emit(
            &channel,
            serde_json::to_value(srelens_agent::event::AgentEvent::Error { message: message.clone() })
                .unwrap(),
        );
        sink.emit(
            &channel,
            serde_json::to_value(srelens_agent::event::AgentEvent::TurnDone).unwrap(),
        );
        return Err(message);
    };

    // Drain stderr concurrently with the stdout loop below: it's piped but
    // otherwise unread, so once the agent writes past the OS pipe buffer it
    // would block on write() and wedge stdout along with it. Keep only a
    // bounded tail, surfaced as an `error` event below if the child exits
    // non-zero.
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            const TAIL_CAP: usize = 8 * 1024;
            let mut tail: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        tail.extend_from_slice(&buf[..n]);
                        if tail.len() > TAIL_CAP {
                            let excess = tail.len() - TAIL_CAP;
                            tail.drain(0..excess);
                        }
                    }
                }
            }
            tail
        })
    });

    chats.children.lock().unwrap().insert(session.clone(), child);

    let mut lines = BufReader::new(stdout).lines();
    let mut saw_done = false;
    while let Ok(Some(line)) = lines.next_line().await {
        saw_done |= emit_events(sink.clone(), &channel, std::iter::once(line));
    }

    // Taken out of the map either here, or already by `chat_cancel` (a
    // user-initiated kill, which is not a crash worth reporting below).
    let removed_child = chats.children.lock().unwrap().remove(&session);

    let stderr_tail = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => Vec::new(),
    };

    // Wait for the actual exit and reap the process — a well-behaved agent
    // that closes stdout just before exiting would otherwise leave a zombie —
    // and use the status to tell a crash from a clean finish.
    let crashed = match removed_child {
        Some(mut child) => !child.wait().await.map(|s| s.success()).unwrap_or(false),
        None => false,
    };

    finish_turn(sink.as_ref(), &channel, saw_done, crashed, &stderr_tail);

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

    #[test]
    fn a_clean_turn_that_already_saw_turn_done_emits_nothing_further() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(sink.as_ref(), "chat://s1", true, true, b"ignored, moot: saw_done wins");
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_stream_that_ends_cleanly_without_turn_done_gets_only_a_synthetic_one() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(sink.as_ref(), "chat://s1", false, false, b"");
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1["type"], "turnDone");
    }

    #[test]
    fn a_crash_emits_an_error_with_the_stderr_tail_before_the_synthetic_turn_done() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(
            sink.as_ref(),
            "chat://s1",
            false,
            true,
            b"Traceback (most recent call last)\nRuntimeError: boom\n",
        );
        let got = sink.0.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].1["type"], "error");
        assert_eq!(got[0].1["message"], "RuntimeError: boom");
        assert_eq!(got[1].1["type"], "turnDone");
    }

    #[test]
    fn a_crash_with_no_stderr_still_gets_a_generic_error_message() {
        let sink = std::sync::Arc::new(RecordingSink(Default::default()));
        finish_turn(sink.as_ref(), "chat://s1", false, true, b"");
        let got = sink.0.lock().unwrap();
        assert_eq!(got[0].1["type"], "error");
        assert_eq!(got[0].1["message"], "the agent process exited unexpectedly");
    }

    #[test]
    fn last_line_trims_and_skips_trailing_blank_lines() {
        assert_eq!(last_line(b"first\nsecond\n\n"), "second");
        assert_eq!(last_line(b"  only  \n"), "only");
        assert_eq!(last_line(b""), "");
    }

    #[test]
    fn temp_file_guard_removes_the_file_on_drop() {
        let path = std::env::temp_dir()
            .join(format!("srelens-assistant-tempfile-test-{}", std::process::id()));
        std::fs::write(&path, b"secret").unwrap();
        {
            let _guard = TempFile(path.clone());
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    /// The assistant must not stand up its own MCP server or policy — that would
    /// be a consent bypass. It only borrows the running server's token/url, so a
    /// gated call still hits the GUI's PromptUser. This guards against a refactor
    /// that gives the assistant its own server.
    #[test]
    fn the_assistant_module_wires_no_confirm_policy_of_its_own() {
        let src = include_str!("assistant.rs");
        let needle1 = ["McpServer", "::", "new"].join("");
        let needle2 = ["Flag", "Gated"].join("");
        let needle3 = ["Always", "Allow"].join("");
        assert!(!src.contains(&needle1), "assistant must reuse the running server");
        assert!(!src.contains(&needle2), "assistant must not introduce a headless policy");
        assert!(!src.contains(&needle3), "assistant must never bypass consent");
    }
}
