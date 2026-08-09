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

/// Codex and Cursor are detected but not yet selectable — their sandbox story
/// (what a spawned agent can touch on the user's machine) isn't solved yet.
/// Claude is the only agent shipped end-to-end in v1.
fn is_gated(kind: AgentKind) -> bool {
    matches!(kind, AgentKind::Codex | AgentKind::Cursor)
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
        gated: is_gated(kind),
    }
}

/// Enumerate every known agent CLI with its availability, for the picker.
#[tauri::command]
pub async fn agent_list() -> Result<Vec<AgentInfo>, String> {
    let paths = srelens_kube::toolbox::SearchPaths::from_env();
    let resolve = |bin: &str| resolve_agent(bin, &paths);
    Ok(vec![
        detect(AgentKind::Claude, resolve),
        detect(AgentKind::Codex, resolve),
        detect(AgentKind::Cursor, resolve),
    ])
}

/// Locate an agent binary across both search paths: the app's own PATH first
/// (post `fix-path-env`, plus srelens's managed dirs), then broader system
/// locations the app doesn't search. This closes the prod gap where a CLI
/// installed under `~/.local/bin` — in `system_path` but not `app_path` — was
/// invisible to a packaged build even though a dev shell's inherited PATH
/// happened to cover it.
fn resolve_agent(bin: &str, paths: &srelens_kube::toolbox::SearchPaths) -> Option<String> {
    which_on_path(bin, &paths.app_path).or_else(|| which_on_path(bin, &paths.system_path))
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

/// Prepend one `Attached image: <path>` line per path, then a blank line,
/// then the original prompt — so the agent sees where each decoded image
/// landed on disk before reading the user's actual question. `image_paths`
/// empty (the common case: no attachments) returns `prompt` unchanged,
/// byte-identical, so a turn without images pays no cost here.
fn prompt_with_images(prompt: &str, image_paths: &[String]) -> String {
    if image_paths.is_empty() {
        return prompt.to_string();
    }
    let mut out = String::new();
    for path in image_paths {
        out.push_str("Attached image: ");
        out.push_str(path);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(prompt);
    out
}

/// Decode one attached image's base64 body (no `data:image/...;base64,`
/// prefix — the WebView strips that before sending) into raw bytes. A thin
/// wrapper so `chat_send` can decode without a fallible call inline, and so
/// both directions (valid → bytes, invalid → error) are unit-tested without
/// touching the filesystem.
fn decode_base64_image(data: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.decode(data)
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

/// Removes the wrapped directory (and everything under it) when dropped. The
/// agent is spawned with this as its working directory so it has no code or
/// files of ours to enumerate even if a tool tried; the directory is created
/// empty per turn and torn down when the turn ends, same lifecycle as
/// `TempFile` above.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Send one user turn. Spawns the agent against our running MCP server and
/// streams `AgentEvent`s on `chat://<session>`.
#[tauri::command]
pub async fn chat_send(
    session: String,
    prompt: String,
    images: Vec<String>,
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

    // A fresh, empty scratch directory for the agent's CWD — never the
    // srelens process's own working directory (the user's repo or home) —
    // so a tool that tried to enumerate its surroundings would find nothing.
    // Removed on every exit from this function, same as `_cfg_guard` above.
    let cwd_path = dir.join(format!("srelens-agent-cwd-{session}"));
    std::fs::create_dir_all(&cwd_path).map_err(|e| e.to_string())?;
    let _cwd_guard = TempDir(cwd_path.clone());

    // Decode each attached image to its own temp file under the same dir,
    // guarded by a `TempFile` per image so every one is cleaned up on any
    // exit from this function, same lifecycle as `_cfg_guard` above. A
    // decode or write failure is reported as an `Error` event and that image
    // is skipped rather than aborting the whole turn.
    let mut image_guards: Vec<TempFile> = Vec::new();
    let mut image_paths: Vec<String> = Vec::new();
    for (i, data) in images.iter().enumerate() {
        let bytes = match decode_base64_image(data) {
            Ok(bytes) => bytes,
            Err(e) => {
                sink.emit(
                    &channel,
                    serde_json::to_value(srelens_agent::event::AgentEvent::Error {
                        message: format!("could not decode attached image {i}: {e}"),
                    })
                    .unwrap(),
                );
                continue;
            }
        };
        let image_path = dir.join(format!("srelens-img-{session}-{i}.png"));
        if let Err(e) = std::fs::write(&image_path, &bytes) {
            sink.emit(
                &channel,
                serde_json::to_value(srelens_agent::event::AgentEvent::Error {
                    message: format!("could not save attached image {i}: {e}"),
                })
                .unwrap(),
            );
            continue;
        }
        image_guards.push(TempFile(image_path.clone()));
        image_paths.push(image_path.to_string_lossy().into_owned());
    }

    let effective_prompt = prompt_with_images(&prompt, &image_paths);

    let cmd = srelens_agent::adapter::claude_command(
        &agent_path,
        &effective_prompt,
        &cfg_path.to_string_lossy(),
        None,
    );
    let mut child = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .current_dir(&cwd_path)
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

    #[test]
    fn codex_and_cursor_are_gated_but_claude_is_not() {
        let info = detect(AgentKind::Codex, |_| Some("/usr/bin/codex".into()));
        assert!(info.available);
        assert!(info.gated);

        let info = detect(AgentKind::Cursor, |_| Some("/usr/bin/cursor-agent".into()));
        assert!(info.available);
        assert!(info.gated);

        let info = detect(AgentKind::Claude, |_| Some("/usr/bin/claude".into()));
        assert!(info.available);
        assert!(!info.gated);
    }

    /// Builds a temp dir under `std::env::temp_dir()` with a single executable
    /// file `bin`, cleaned up on drop — the fixture `resolve_agent` tests use
    /// to prove the search actually walks `system_path`, not just `app_path`.
    struct BinDir {
        dir: std::path::PathBuf,
    }

    impl BinDir {
        fn new(bin: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("srelens-resolve-agent-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join(bin);
            std::fs::write(&file, b"#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&file).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&file, perms).unwrap();
            }
            BinDir { dir }
        }

        fn path(&self) -> String {
            self.dir.to_string_lossy().into_owned()
        }
    }

    impl Drop for BinDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn resolve_agent_finds_a_binary_only_present_on_the_system_path() {
        let bin_dir = BinDir::new("x");
        let paths = srelens_kube::toolbox::SearchPaths {
            app_path: "/nope".to_string(),
            system_path: bin_dir.path(),
        };
        let resolved = resolve_agent("x", &paths).expect("should find x via system_path");
        assert_eq!(resolved, bin_dir.dir.join("x").to_string_lossy());
    }

    #[test]
    fn resolve_agent_returns_none_when_absent_from_both_paths() {
        let paths = srelens_kube::toolbox::SearchPaths {
            app_path: "/nope".to_string(),
            system_path: "/also/nope".to_string(),
        };
        assert!(resolve_agent("x", &paths).is_none());
    }

    #[test]
    fn resolve_agent_prefers_app_path_when_present_in_both() {
        let app_dir = BinDir::new("x");
        let system_dir = BinDir::new("x");
        let paths = srelens_kube::toolbox::SearchPaths {
            app_path: app_dir.path(),
            system_path: system_dir.path(),
        };
        let resolved = resolve_agent("x", &paths).unwrap();
        assert_eq!(resolved, app_dir.dir.join("x").to_string_lossy());
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

    #[test]
    fn temp_dir_guard_removes_the_directory_and_its_contents_on_drop() {
        let path = std::env::temp_dir()
            .join(format!("srelens-assistant-tempdir-test-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("nested.txt"), b"nothing to see here").unwrap();
        {
            let _guard = TempDir(path.clone());
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    #[test]
    fn prompt_with_images_lists_each_path_then_a_blank_line_then_the_prompt() {
        let images = vec!["/tmp/a.png".to_string(), "/tmp/b.png".to_string()];
        let got = prompt_with_images("why?", &images);
        assert_eq!(got, "Attached image: /tmp/a.png\nAttached image: /tmp/b.png\n\nwhy?");
    }

    #[test]
    fn prompt_with_images_returns_the_prompt_unchanged_when_there_are_no_images() {
        let got = prompt_with_images("why?", &[]);
        assert_eq!(got, "why?");
    }

    #[test]
    fn decode_base64_image_recovers_the_original_bytes() {
        // "hello" base64-encoded, hand-written rather than produced by encoding
        // "hello" ourselves in the test.
        let bytes = decode_base64_image("aGVsbG8=").expect("valid base64");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn decode_base64_image_rejects_invalid_base64() {
        assert!(decode_base64_image("not-valid-base64!!!").is_err());
    }

    #[test]
    fn a_decoded_image_written_to_a_temp_file_reads_back_byte_identical() {
        let bytes = decode_base64_image("aGVsbG8=").expect("valid base64");
        let path = std::env::temp_dir().join(format!("srelens-assistant-image-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, &bytes).unwrap();
        let read_back = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read_back, bytes);
        assert_eq!(read_back, b"hello");
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
