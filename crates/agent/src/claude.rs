//! Parse Claude Code `--output-format stream-json` lines into `AgentEvent`s.

use crate::event::{AgentEvent, ToolStatus};

/// Parse one stream-json line. Never errors: an unrecognized or non-JSON line
/// yields an empty vec, so a future line type the CLI adds is ignored rather
/// than aborting the turn.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };

    match v.get("type").and_then(|t| t.as_str()) {
        Some("assistant") => content_blocks(&v)
            .iter()
            .filter_map(block_to_event)
            .collect(),
        Some("user") => content_blocks(&v).iter().filter_map(tool_result).collect(),
        // A terminal `result` ends the turn. When it's a failure (auth/quota/
        // max-turns/etc.) surface its message as an `Error` first — otherwise
        // the turn ends with no visible reply, since the desktop's crash-path
        // reporting stops as soon as it sees a `TurnDone`.
        Some("result") => {
            let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
            if is_error {
                let message = v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("the agent ended the turn with an error")
                    .to_string();
                vec![AgentEvent::Error { message }, AgentEvent::TurnDone]
            } else {
                vec![AgentEvent::TurnDone]
            }
        }
        _ => Vec::new(),
    }
}

fn content_blocks(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default()
}

fn block_to_event(block: &serde_json::Value) -> Option<AgentEvent> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("text") => Some(AgentEvent::TextDelta {
            text: block.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        }),
        // The `thinking` field is the documented key for this block, but we
        // fall back to `text` defensively in case a future CLI version
        // shapes it like a text block instead.
        //
        // KNOWN LIMITATION: in headless (`-p`) mode the CLI redacts the
        // thinking CONTENT — blocks arrive with an empty `thinking` string
        // and only the crypto `signature` (verified live on claude CLI
        // 2.1.228, multiple models, with and without
        // --include-partial-messages, whose thinking_deltas are empty too).
        // So Claude turns currently produce no visible Thoughts; the mapping
        // stays for the day the CLI starts sharing the text.
        Some("thinking") => Some(AgentEvent::Thinking {
            text: block
                .get("thinking")
                .or_else(|| block.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        Some("tool_use") => Some(AgentEvent::ToolCallStart {
            id: block.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
            tool: block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
            args: block
                .get("input")
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        }),
        _ => None,
    }
}

fn tool_result(block: &serde_json::Value) -> Option<AgentEvent> {
    if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
        return None;
    }
    let is_error = block.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
    let status = if !is_error {
        ToolStatus::Ok
    } else if crate::event::is_denial_text(&result_text(block)) {
        // The user declining the consent dialog, not the tool failing —
        // show it as such (see `DENIED_PREFIX`).
        ToolStatus::Denied
    } else {
        ToolStatus::Error
    };
    Some(AgentEvent::ToolResult {
        id: block.get("tool_use_id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
        status,
    })
}

/// A `tool_result`'s `content` is either a bare string or an array of
/// `{type:"text", text}` blocks; flatten to the concatenated text.
fn result_text(block: &serde_json::Value) -> String {
    match block.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_or_unknown_line_yields_nothing() {
        assert!(parse_line("").is_empty());
        assert!(parse_line("   ").is_empty());
        assert!(parse_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
        // Not JSON at all: ignored, not fatal.
        assert!(parse_line("not json").is_empty());
    }

    #[test]
    fn assistant_text_becomes_a_text_delta() {
        let out = parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        );
        assert_eq!(out, vec![AgentEvent::TextDelta { text: "hi".into() }]);
    }

    #[test]
    fn a_tool_use_block_becomes_a_tool_call_start() {
        let out = parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"k8s.scale","input":{"replicas":3}}]}}"#,
        );
        assert_eq!(
            out,
            vec![AgentEvent::ToolCallStart {
                id: "t1".into(),
                tool: "k8s.scale".into(),
                args: serde_json::json!({ "replicas": 3 }),
            }]
        );
    }

    #[test]
    fn a_thinking_block_becomes_a_thinking_event() {
        let out = parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"let me consider the options"}]}}"#,
        );
        assert_eq!(out, vec![AgentEvent::Thinking { text: "let me consider the options".into() }]);
    }

    #[test]
    fn a_non_object_tool_use_input_is_coerced_to_an_empty_object() {
        let out = parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t3","name":"k8s.scale","input":"oops"}]}}"#,
        );
        assert_eq!(
            out,
            vec![AgentEvent::ToolCallStart {
                id: "t3".into(),
                tool: "k8s.scale".into(),
                args: serde_json::json!({}),
            }]
        );
    }

    #[test]
    fn one_message_with_text_and_tool_use_yields_both_in_order() {
        let out = parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"go"},{"type":"tool_use","id":"t2","name":"k8s.listPods","input":{}}]}}"#,
        );
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], AgentEvent::TextDelta { .. }));
        assert!(matches!(out[1], AgentEvent::ToolCallStart { .. }));
    }

    #[test]
    fn a_tool_result_maps_is_error_to_status() {
        let ok = parse_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":false}]}}"#,
        );
        assert_eq!(ok, vec![AgentEvent::ToolResult { id: "t1".into(), status: ToolStatus::Ok }]);
        let err = parse_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true}]}}"#,
        );
        assert_eq!(err, vec![AgentEvent::ToolResult { id: "t1".into(), status: ToolStatus::Error }]);
    }

    #[test]
    fn a_consent_refusal_maps_to_denied_not_error() {
        // Both content shapes Claude Code uses: an array of text blocks…
        let blocks = parse_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"consent denied: user declined `k8s.deletePod`"}]}]}}"#,
        );
        assert_eq!(blocks, vec![AgentEvent::ToolResult { id: "t1".into(), status: ToolStatus::Denied }]);
        // …and a bare string.
        let bare = parse_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":"consent denied: user declined `k8s.scale`"}]}}"#,
        );
        assert_eq!(bare, vec![AgentEvent::ToolResult { id: "t2".into(), status: ToolStatus::Denied }]);
    }

    #[test]
    fn a_result_line_ends_the_turn() {
        assert_eq!(parse_line(r#"{"type":"result","subtype":"success"}"#), vec![AgentEvent::TurnDone]);
    }

    #[test]
    fn a_failed_result_surfaces_its_error_before_ending_the_turn() {
        let out = parse_line(
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"Credit balance too low"}"#,
        );
        assert_eq!(
            out,
            vec![
                AgentEvent::Error { message: "Credit balance too low".into() },
                AgentEvent::TurnDone,
            ]
        );
    }

    #[test]
    fn a_failed_result_with_no_text_still_reports_an_error() {
        let out = parse_line(r#"{"type":"result","is_error":true}"#);
        assert!(matches!(out.first(), Some(AgentEvent::Error { .. })));
        assert_eq!(out.last(), Some(&AgentEvent::TurnDone));
    }

    #[test]
    fn the_recorded_transcript_parses_to_the_expected_shape() {
        let raw = include_str!("../tests/fixtures/claude_basic.jsonl");
        let events: Vec<AgentEvent> = raw.lines().flat_map(parse_line).collect();
        // 2 text deltas, 1 tool call, 1 tool result, 1 turn-done.
        assert_eq!(
            events.iter().filter(|e| matches!(e, AgentEvent::TextDelta { .. })).count(),
            2
        );
        assert_eq!(
            events.iter().filter(|e| matches!(e, AgentEvent::ToolCallStart { .. })).count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(
                    e,
                    AgentEvent::ToolResult { status: ToolStatus::Ok, .. }
                ))
                .count(),
            1
        );
        assert!(events.contains(&AgentEvent::TurnDone));
    }
}
