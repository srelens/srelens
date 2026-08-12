//! Parse Codex CLI `exec --json` lines into `AgentEvent`s.

use crate::event::{AgentEvent, ToolStatus};

/// Parse one Codex JSONL line. Never errors: an unrecognized, malformed, or
/// non-JSON line yields an empty vec, so a future line type the CLI adds is
/// ignored rather than aborting the turn.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };

    match v.get("type").and_then(|t| t.as_str()) {
        Some("item.started") => item(&v).map(item_started).unwrap_or_default(),
        Some("item.completed") => item(&v).map(item_completed).unwrap_or_default(),
        Some("turn.completed") => vec![AgentEvent::TurnDone],
        _ => Vec::new(),
    }
}

fn item(v: &serde_json::Value) -> Option<&serde_json::Value> {
    v.get("item").filter(|i| i.is_object())
}

fn str_field<'a>(item: &'a serde_json::Value, key: &str) -> &'a str {
    item.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn item_started(item: &serde_json::Value) -> Vec<AgentEvent> {
    match item.get("type").and_then(|t| t.as_str()) {
        Some("mcp_tool_call") => vec![AgentEvent::ToolCallStart {
            id: str_field(item, "id").to_string(),
            tool: format!("{}.{}", str_field(item, "server"), str_field(item, "tool")),
            args: item.get("arguments").cloned().unwrap_or(serde_json::Value::Null),
        }],
        Some("command_execution") => vec![AgentEvent::ToolCallStart {
            id: str_field(item, "id").to_string(),
            tool: "shell".to_string(),
            args: serde_json::json!({ "command": str_field(item, "command") }),
        }],
        _ => Vec::new(),
    }
}

fn item_completed(item: &serde_json::Value) -> Vec<AgentEvent> {
    match item.get("type").and_then(|t| t.as_str()) {
        Some("agent_message") => vec![AgentEvent::TextDelta {
            text: str_field(item, "text").to_string(),
        }],
        Some("mcp_tool_call") => {
            let has_error = item.get("error").map(|e| !e.is_null()).unwrap_or(false);
            let completed = item.get("status").and_then(|s| s.as_str()) == Some("completed");
            let status = if !has_error && completed {
                ToolStatus::Ok
            } else if crate::event::is_denial_text(&item.get("error").map(|e| e.to_string()).unwrap_or_default())
            {
                // A consent refusal from the srelens MCP server, not a tool
                // failure (see `DENIED_PREFIX`).
                ToolStatus::Denied
            } else {
                ToolStatus::Error
            };
            vec![AgentEvent::ToolResult { id: str_field(item, "id").to_string(), status }]
        }
        Some("command_execution") => {
            let ok = item.get("exit_code").and_then(|c| c.as_i64()) == Some(0);
            vec![AgentEvent::ToolResult {
                id: str_field(item, "id").to_string(),
                status: if ok { ToolStatus::Ok } else { ToolStatus::Error },
            }]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_or_unknown_line_yields_nothing() {
        assert!(parse_line("").is_empty());
        assert!(parse_line("   ").is_empty());
        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"no_type_field":true}"#).is_empty());
        assert!(parse_line(r#"{"type":"something.unknown"}"#).is_empty());
    }

    #[test]
    fn thread_started_and_turn_started_are_ignored() {
        assert!(parse_line(r#"{"type":"thread.started","thread_id":"019fe8af-25e9-7da2-905a-e07859916dbe"}"#).is_empty());
        assert!(parse_line(r#"{"type":"turn.started"}"#).is_empty());
    }

    #[test]
    fn a_reasoning_item_is_ignored() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_9","type":"reasoning","text":"thinking about it"}}"#,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn an_agent_message_item_becomes_a_text_delta() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hello"}}"#,
        );
        assert_eq!(out, vec![AgentEvent::TextDelta { text: "hello".into() }]);
    }

    #[test]
    fn an_mcp_tool_call_started_becomes_a_tool_call_start() {
        let out = parse_line(
            r#"{"type":"item.started","item":{"id":"item_1","type":"mcp_tool_call","server":"everything","tool":"echo","arguments":{"message":"srelens-mcp-probe"},"result":null,"error":null,"status":"in_progress"}}"#,
        );
        assert_eq!(
            out,
            vec![AgentEvent::ToolCallStart {
                id: "item_1".into(),
                tool: "everything.echo".into(),
                args: serde_json::json!({ "message": "srelens-mcp-probe" }),
            }]
        );
    }

    #[test]
    fn an_mcp_tool_call_completed_with_null_error_and_completed_status_is_ok() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"everything","tool":"echo","arguments":{"message":"srelens-mcp-probe"},"result":{"content":[{"type":"text","text":"Echo: srelens-mcp-probe"}],"structured_content":null},"error":null,"status":"completed"}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Ok }]);
    }

    #[test]
    fn an_mcp_tool_call_completed_with_a_non_null_error_is_an_error() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"everything","tool":"echo","arguments":{},"result":null,"error":{"message":"boom"},"status":"completed"}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Error }]);
    }

    #[test]
    fn an_mcp_tool_call_refused_by_consent_is_denied_not_error() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"srelens","tool":"k8s_deletePod","arguments":{},"result":null,"error":{"message":"consent denied: user declined `k8s.deletePod`"},"status":"completed"}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Denied }]);
    }

    #[test]
    fn an_mcp_tool_call_completed_with_a_non_completed_status_is_an_error() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"everything","tool":"echo","arguments":{},"result":null,"error":null,"status":"failed"}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Error }]);
    }

    #[test]
    fn a_command_execution_started_becomes_a_shell_tool_call_start() {
        let out = parse_line(
            r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"/bin/zsh -lc 'echo srelens-probe'","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#,
        );
        assert_eq!(
            out,
            vec![AgentEvent::ToolCallStart {
                id: "item_1".into(),
                tool: "shell".into(),
                args: serde_json::json!({ "command": "/bin/zsh -lc 'echo srelens-probe'" }),
            }]
        );
    }

    #[test]
    fn a_command_execution_completed_with_zero_exit_is_ok() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"/bin/zsh -lc 'echo srelens-probe'","aggregated_output":"srelens-probe\n","exit_code":0,"status":"completed"}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Ok }]);
    }

    #[test]
    fn a_command_execution_completed_with_nonzero_exit_is_an_error() {
        let out = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"false","aggregated_output":"","exit_code":1,"status":"completed"}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Error }]);
    }

    #[test]
    fn a_turn_completed_line_ends_the_turn() {
        assert_eq!(
            parse_line(r#"{"type":"turn.completed","usage":{"input_tokens":15408,"cached_input_tokens":9984,"output_tokens":5,"reasoning_output_tokens":0}}"#),
            vec![AgentEvent::TurnDone]
        );
    }

    #[test]
    fn the_simple_transcript_fixture_parses_to_the_expected_shape() {
        let raw = include_str!(
            "../tests/fixtures/codex-simple.jsonl"
        );
        let events: Vec<AgentEvent> = raw
            .lines()
            .filter(|l| !l.starts_with("Reading additional input"))
            .flat_map(parse_line)
            .collect();
        assert_eq!(events, vec![AgentEvent::TextDelta { text: "hello".into() }, AgentEvent::TurnDone]);
    }

    #[test]
    fn the_mcp_tool_call_transcript_fixture_parses_to_the_expected_shape() {
        let raw = include_str!(
            "../tests/fixtures/codex-mcp-tool-call.jsonl"
        );
        let events: Vec<AgentEvent> = raw.lines().flat_map(parse_line).collect();
        assert_eq!(
            events.iter().filter(|e| matches!(e, AgentEvent::ToolCallStart { .. })).count(),
            1
        );
        assert!(events.contains(&AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Ok }));
        assert_eq!(
            events.iter().filter(|e| matches!(e, AgentEvent::TextDelta { .. })).count(),
            2
        );
        assert!(events.contains(&AgentEvent::TurnDone));
    }

    #[test]
    fn the_command_execution_transcript_fixture_parses_to_the_expected_shape() {
        let raw = include_str!(
            "../tests/fixtures/codex-command-execution.jsonl"
        );
        let events: Vec<AgentEvent> = raw.lines().flat_map(parse_line).collect();
        assert!(events.contains(&AgentEvent::ToolCallStart {
            id: "item_1".into(),
            tool: "shell".into(),
            args: serde_json::json!({ "command": "/bin/zsh -lc 'echo srelens-probe'" }),
        }));
        assert!(events.contains(&AgentEvent::ToolResult { id: "item_1".into(), status: ToolStatus::Ok }));
        assert!(events.contains(&AgentEvent::TurnDone));
    }
}
