//! Agent-CLI bridge: normalize each supported agent CLI's streaming output
//! into a common `AgentEvent` stream. Pure — no Tauri, no cluster I/O.

pub mod adapter;
pub mod claude;
pub mod codex;
pub mod cursor;
pub mod event;

/// The agent CLI's own session id, if `line` (one stream-json line) carries
/// one. Claude Code and Cursor both stamp a top-level `session_id` string on
/// every stream-json line (verified live against both CLIs), so the first
/// line of a turn is enough to learn the id that `--resume` needs on the
/// next turn. Codex names its thread once, as `thread_id` on the turn's
/// `thread.started` line — the id `codex_command` resumes (#215). Never
/// errors: a non-JSON or id-less line yields `None`, matching the parsers'
/// tolerance above.
pub fn stream_session_id(line: &str) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
    let id = match v.get("type").and_then(|t| t.as_str()) {
        Some("thread.started") => v.get("thread_id"),
        _ => v.get("session_id"),
    };
    id.and_then(|s| s.as_str()).map(str::to_string)
}

/// What the caller should store as the conversation's CLI session id after a
/// turn: the id captured from the stream when there is one, else the id we
/// resumed FROM (a clean-but-drifty stream, or a cancel) — EXCEPT after a crash
/// that yielded no id. That is the signature of the `--resume` itself failing
/// (the CLI GC'd or never had the session), and echoing "old" back would retry
/// the same failing resume with no way out short of a new chat. `None` there
/// tells the next turn to start clean.
pub fn next_resume_id(
    agent_session: Option<String>,
    resume: Option<String>,
    crashed: bool,
) -> Option<String> {
    match agent_session {
        Some(session) => Some(session),
        None if crashed => None,
        None => resume,
    }
}

/// The directory under `root` that one Cursor chat is kept in. Its workspace
/// is emptied before each turn and the whole of it removed with the chat, so
/// `chat` must be one plain component — never `.` or `..`.
pub fn cursor_chat_dir(root: &std::path::Path, chat: &str) -> Result<std::path::PathBuf, String> {
    let one_name = matches!(
        std::path::Path::new(chat).components().collect::<Vec<_>>()[..],
        [std::path::Component::Normal(_)]
    );
    if !one_name {
        return Err(format!("invalid session id {chat:?}"));
    }
    Ok(root.join(chat))
}

/// Remove one Cursor chat's directory tree under `dir`. Tolerates `NotFound`.
pub fn remove_cursor_chats(dir: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod session_id_tests {
    use super::stream_session_id;

    #[test]
    fn claude_shaped_lines_yield_the_session_id() {
        // Abridged from a real `claude -p --output-format stream-json` init line.
        let line = r#"{"type":"system","subtype":"init","cwd":"/tmp","session_id":"ee637368-ca14-4420-b431-e346fc3877b5","model":"m"}"#;
        assert_eq!(
            stream_session_id(line).as_deref(),
            Some("ee637368-ca14-4420-b431-e346fc3877b5")
        );
    }

    #[test]
    fn cursor_shaped_lines_yield_the_session_id() {
        // Abridged from a real `cursor-agent -p --output-format stream-json` line.
        let line = r#"{"type":"user","message":{"role":"user","content":[]},"session_id":"ea50278e-075d-4b80-849e-f7c952ed74b1"}"#;
        assert_eq!(
            stream_session_id(line).as_deref(),
            Some("ea50278e-075d-4b80-849e-f7c952ed74b1")
        );
    }

    #[test]
    fn a_codex_thread_started_line_yields_the_thread_id() {
        // Verbatim first line of a real `codex exec --json` turn (0.149.1).
        let line =
            r#"{"type":"thread.started","thread_id":"01a15e55-92e4-7cb3-81c4-9688adf89849"}"#;
        assert_eq!(
            stream_session_id(line).as_deref(),
            Some("01a15e55-92e4-7cb3-81c4-9688adf89849")
        );
    }

    #[test]
    fn a_thread_id_on_any_other_codex_line_is_not_the_conversation() {
        assert_eq!(
            stream_session_id(r#"{"type":"item.completed","thread_id":"x"}"#),
            None
        );
    }

    #[test]
    fn idless_nonjson_and_nonstring_lines_yield_none() {
        assert_eq!(stream_session_id(r#"{"type":"turn.started"}"#), None);
        assert_eq!(stream_session_id("not json"), None);
        assert_eq!(stream_session_id(""), None);
        // A non-string session_id (future shape drift) is ignored, not coerced.
        assert_eq!(stream_session_id(r#"{"session_id":42}"#), None);
    }

    #[test]
    fn next_resume_id_clean_stream_and_crashes() {
        use super::next_resume_id;

        // When stream emitted a session id, use it
        assert_eq!(
            next_resume_id(Some("new".into()), Some("old".into()), false),
            Some("new".into())
        );
        assert_eq!(
            next_resume_id(Some("new".into()), None, false),
            Some("new".into())
        );
        // Even on crash, if an id was seen before the crash, keep it
        assert_eq!(
            next_resume_id(Some("new".into()), Some("old".into()), true),
            Some("new".into())
        );

        // A clean turn without an id echoes resume back (or None if none)
        assert_eq!(
            next_resume_id(None, Some("old".into()), false),
            Some("old".into())
        );
        assert_eq!(next_resume_id(None, None, false), None);

        // A crash without an id clears the resume (stale/broken session)
        assert_eq!(next_resume_id(None, Some("old".into()), true), None);
        assert_eq!(next_resume_id(None, None, true), None);
    }

    #[test]
    fn cursor_chat_dir_rejects_path_traversal() {
        use super::cursor_chat_dir;
        let root = std::path::Path::new("/tmp/test-chats");

        assert_eq!(
            cursor_chat_dir(root, "valid-uuid-123").unwrap(),
            root.join("valid-uuid-123")
        );
        assert!(cursor_chat_dir(root, ".").is_err());
        assert!(cursor_chat_dir(root, "..").is_err());
        assert!(cursor_chat_dir(root, "foo/bar").is_err());
        assert!(cursor_chat_dir(root, "/absolute").is_err());
    }

    #[test]
    fn remove_cursor_chats_tolerates_not_found() {
        use super::remove_cursor_chats;
        let non_existent = std::path::Path::new("/tmp/definitely-not-existent-123456789");
        assert!(remove_cursor_chats(non_existent).is_ok());
    }
}
