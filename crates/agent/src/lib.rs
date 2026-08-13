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
/// next turn. Codex is shaped differently (`thread.started`/`thread_id`) and
/// isn't resumable under srelens's sandbox flags — see `chat_send`'s Codex
/// arm. Never errors: a non-JSON or id-less line yields `None`, matching the
/// parsers' tolerance above.
pub fn stream_session_id(line: &str) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
    v.get("session_id").and_then(|s| s.as_str()).map(str::to_string)
}

#[cfg(test)]
mod session_id_tests {
    use super::stream_session_id;

    #[test]
    fn claude_shaped_lines_yield_the_session_id() {
        // Abridged from a real `claude -p --output-format stream-json` init line.
        let line = r#"{"type":"system","subtype":"init","cwd":"/tmp","session_id":"ee637368-ca14-4420-b431-e346fc3877b5","model":"m"}"#;
        assert_eq!(stream_session_id(line).as_deref(), Some("ee637368-ca14-4420-b431-e346fc3877b5"));
    }

    #[test]
    fn cursor_shaped_lines_yield_the_session_id() {
        // Abridged from a real `cursor-agent -p --output-format stream-json` line.
        let line = r#"{"type":"user","message":{"role":"user","content":[]},"session_id":"ea50278e-075d-4b80-849e-f7c952ed74b1"}"#;
        assert_eq!(stream_session_id(line).as_deref(), Some("ea50278e-075d-4b80-849e-f7c952ed74b1"));
    }

    #[test]
    fn idless_nonjson_and_nonstring_lines_yield_none() {
        assert_eq!(stream_session_id(r#"{"type":"turn.started"}"#), None);
        assert_eq!(stream_session_id("not json"), None);
        assert_eq!(stream_session_id(""), None);
        // A non-string session_id (future shape drift) is ignored, not coerced.
        assert_eq!(stream_session_id(r#"{"session_id":42}"#), None);
    }
}
