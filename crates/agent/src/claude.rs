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
        Some("result") => vec![AgentEvent::TurnDone],
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
    Some(AgentEvent::ToolResult {
        id: block.get("tool_use_id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
        status: if is_error { ToolStatus::Error } else { ToolStatus::Ok },
    })
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
    fn a_result_line_ends_the_turn() {
        assert_eq!(parse_line(r#"{"type":"result","subtype":"success"}"#), vec![AgentEvent::TurnDone]);
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
