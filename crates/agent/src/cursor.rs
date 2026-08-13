//! Parse Cursor CLI (`cursor-agent`) `--output-format stream-json` lines into
//! `AgentEvent`s.

use crate::event::{AgentEvent, ToolStatus};

/// Parse one Cursor stream-json line. Never errors: an unrecognized,
/// malformed, or non-JSON line yields an empty vec, so a future line type the
/// CLI adds is ignored rather than aborting the turn.
pub fn parse_line(line: &str) -> Vec<AgentEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };

    match v.get("type").and_then(|t| t.as_str()) {
        Some("thinking") => thinking(&v),
        Some("assistant") => content_blocks(&v).iter().filter_map(text_block).collect(),
        Some("tool_call") => tool_call(&v),
        // The `result` text duplicates the already-streamed assistant text
        // deltas, so a success only emits the turn-boundary signal. A FAILURE
        // (`is_error`) surfaces its message as an `Error` first — otherwise an
        // auth/quota/CLI failure ends the turn with a blank reply (the
        // desktop's crash reporting stops as soon as it sees a `TurnDone`).
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

fn thinking(v: &serde_json::Value) -> Vec<AgentEvent> {
    match v.get("subtype").and_then(|s| s.as_str()) {
        Some("delta") => vec![AgentEvent::Thinking {
            text: v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        }],
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

fn text_block(block: &serde_json::Value) -> Option<AgentEvent> {
    if block.get("type").and_then(|t| t.as_str()) != Some("text") {
        return None;
    }
    Some(AgentEvent::TextDelta {
        text: block.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
    })
}

/// `<name>ToolCall` -> `<name>`, e.g. `readToolCall` -> `read`,
/// `getMcpToolsToolCall` -> `getMcpTools`. Left as-is if a future CLI version
/// ships a key that doesn't follow the `...ToolCall` naming convention.
fn tool_name(key: &str) -> String {
    key.strip_suffix("ToolCall").unwrap_or(key).to_string()
}

fn tool_call(v: &serde_json::Value) -> Vec<AgentEvent> {
    // `call_id` is shared verbatim (including any embedded `\n`) between the
    // `started` and `completed` lines for the same call, so it's what
    // correlates a `ToolResult` back to its `ToolCallStart`.
    let id = v.get("call_id").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let Some(inner_map) = v.get("tool_call").and_then(|t| t.as_object()) else {
        return Vec::new();
    };
    // The tool-specific payload lives under a key named for the tool (e.g.
    // `readToolCall`), alongside bookkeeping siblings the CLI also puts in
    // this object (`toolCallId`, `startedAtMs`, `hookAdditionalContexts`,
    // ...). Find the payload key by its `...ToolCall` suffix rather than
    // assuming it's the map's only (or first) entry — map iteration order
    // isn't guaranteed to put it first.
    let Some((key, inner)) = inner_map.iter().find(|(k, _)| k.ends_with("ToolCall")) else {
        return Vec::new();
    };

    match v.get("subtype").and_then(|s| s.as_str()) {
        Some("started") => vec![AgentEvent::ToolCallStart {
            id,
            tool: tool_name(key),
            args: inner.get("args").cloned().unwrap_or(serde_json::Value::Null),
        }],
        Some("completed") => vec![AgentEvent::ToolResult { id, status: completion_status(inner) }],
        _ => Vec::new(),
    }
}

/// Cursor nests the actual outcome under `result`. A tool blocked by the
/// sandbox deny-list reports the denial under `result.permissionDenied` (e.g.
/// a `shell`/`read` call refused because the srelens box denies local tools) —
/// surface that as `Denied` so the UI shows the box working, not a spurious
/// "ok". A genuine failure reports `result.error`/`isError`/`is_error` and maps
/// to `Error`. Everything else is `Ok`.
fn completion_status(inner: &serde_json::Value) -> ToolStatus {
    let candidate = inner.get("result").unwrap_or(inner);
    if candidate.get("permissionDenied").map(is_truthy).unwrap_or(false) {
        return ToolStatus::Denied;
    }
    let has_error = ["error", "isError", "is_error"]
        .iter()
        .any(|key| candidate.get(*key).map(is_truthy).unwrap_or(false));
    if !has_error {
        return ToolStatus::Ok;
    }
    // An MCP-side consent refusal travels as an errored result whose text
    // carries `DENIED_PREFIX` — the user's "no", not a failed execution.
    if crate::event::is_denial_text(&candidate.to_string()) {
        return ToolStatus::Denied;
    }
    ToolStatus::Error
}

fn is_truthy(v: &serde_json::Value) -> bool {
    !v.is_null() && v != &serde_json::Value::Bool(false)
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
    fn system_and_user_lines_are_ignored() {
        assert!(parse_line(
            r#"{"type":"system","subtype":"init","apiKeySource":"login","cwd":"/tmp","session_id":"s1","model":"Auto","permissionMode":"default"}"#
        )
        .is_empty());
        assert!(parse_line(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]},"session_id":"s1"}"#
        )
        .is_empty());
    }

    #[test]
    fn a_thinking_delta_becomes_a_thinking_event() {
        let out = parse_line(
            r#"{"type":"thinking","subtype":"delta","text":"I will first call the","session_id":"s1","timestamp_ms":1}"#,
        );
        assert_eq!(out, vec![AgentEvent::Thinking { text: "I will first call the".into() }]);
    }

    #[test]
    fn a_thinking_completed_line_yields_nothing() {
        assert!(parse_line(r#"{"type":"thinking","subtype":"completed","session_id":"s1"}"#).is_empty());
    }

    #[test]
    fn an_assistant_text_block_becomes_a_text_delta() {
        let out = parse_line(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]},"session_id":"s1"}"#,
        );
        assert_eq!(out, vec![AgentEvent::TextDelta { text: "hello".into() }]);
    }

    #[test]
    fn multiple_text_blocks_in_one_assistant_message_all_become_text_deltas_in_order() {
        let out = parse_line(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"go"},{"type":"text","text":"then this"}]}}"#,
        );
        assert_eq!(
            out,
            vec![
                AgentEvent::TextDelta { text: "go".into() },
                AgentEvent::TextDelta { text: "then this".into() },
            ]
        );
    }

    #[test]
    fn a_tool_call_started_becomes_a_tool_call_start_with_the_stripped_tool_name() {
        let out = parse_line(
            r#"{"type":"tool_call","subtype":"started","call_id":"call-1\nfc_2","tool_call":{"getMcpToolsToolCall":{"args":{"pattern":"echo","toolCallId":"call-1\nfc_2"}},"toolCallId":"call-1\nfc_2"}}"#,
        );
        assert_eq!(
            out,
            vec![AgentEvent::ToolCallStart {
                id: "call-1\nfc_2".into(),
                tool: "getMcpTools".into(),
                args: serde_json::json!({ "pattern": "echo", "toolCallId": "call-1\nfc_2" }),
            }]
        );
    }

    #[test]
    fn readtoolcall_and_mcptoolcall_strip_to_read_and_mcp() {
        let read = parse_line(
            r#"{"type":"tool_call","subtype":"started","call_id":"c1","tool_call":{"readToolCall":{"args":{"path":"/etc/hosts"}}}}"#,
        );
        assert_eq!(
            read,
            vec![AgentEvent::ToolCallStart {
                id: "c1".into(),
                tool: "read".into(),
                args: serde_json::json!({ "path": "/etc/hosts" }),
            }]
        );

        let mcp = parse_line(
            r#"{"type":"tool_call","subtype":"started","call_id":"c2","tool_call":{"mcpToolCall":{"args":{"tool":"k8s.listPods"}}}}"#,
        );
        assert_eq!(
            mcp,
            vec![AgentEvent::ToolCallStart {
                id: "c2".into(),
                tool: "mcp".into(),
                args: serde_json::json!({ "tool": "k8s.listPods" }),
            }]
        );
    }

    #[test]
    fn a_tool_call_started_with_no_args_field_defaults_to_null() {
        let out = parse_line(
            r#"{"type":"tool_call","subtype":"started","call_id":"c1","tool_call":{"shellToolCall":{"workingDirectory":""}}}"#,
        );
        assert_eq!(
            out,
            vec![AgentEvent::ToolCallStart {
                id: "c1".into(),
                tool: "shell".into(),
                args: serde_json::Value::Null,
            }]
        );
    }

    #[test]
    fn a_tool_call_completed_without_an_error_field_is_ok() {
        let out = parse_line(
            r#"{"type":"tool_call","subtype":"completed","call_id":"call-1\nfc_2","tool_call":{"getMcpToolsToolCall":{"args":{"pattern":"echo"},"result":{"success":{"content":"{}"}}}}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "call-1\nfc_2".into(), status: ToolStatus::Ok }]);
    }

    #[test]
    fn a_tool_call_completed_with_an_error_field_is_an_error() {
        let out = parse_line(
            r#"{"type":"tool_call","subtype":"completed","call_id":"call-1\nfc_2","tool_call":{"readToolCall":{"result":{"error":{"errorMessage":"Permission denied"}}}}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "call-1\nfc_2".into(), status: ToolStatus::Error }]);
    }

    #[test]
    fn an_mcp_consent_refusal_is_denied_not_error() {
        // The srelens MCP server's refusal text (see `DENIED_PREFIX`) rides in
        // the errored result — the user's "no", not a failed execution.
        let out = parse_line(
            r#"{"type":"tool_call","subtype":"completed","call_id":"c2","tool_call":{"mcpToolCall":{"result":{"isError":true,"content":[{"type":"text","text":"consent denied: user declined `k8s.deletePod`"}]}}}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "c2".into(), status: ToolStatus::Denied }]);
    }

    #[test]
    fn a_tool_call_blocked_by_the_sandbox_is_denied_not_ok() {
        // A shell/read the box deny-lists reports its refusal under
        // `result.permissionDenied` (real shape from a boxed Cursor run) — it
        // must surface as Denied so the UI shows the box working, not "ok".
        let out = parse_line(
            r#"{"type":"tool_call","subtype":"completed","call_id":"c1","tool_call":{"shellToolCall":{"result":{"permissionDenied":{"command":"cat /etc/hosts"}}}}}"#,
        );
        assert_eq!(out, vec![AgentEvent::ToolResult { id: "c1".into(), status: ToolStatus::Denied }]);
    }

    #[test]
    fn a_result_line_ends_the_turn_regardless_of_subtype() {
        assert_eq!(
            parse_line(r#"{"type":"result","subtype":"success","is_error":false,"result":"hello"}"#),
            vec![AgentEvent::TurnDone]
        );
    }

    #[test]
    fn a_failed_result_surfaces_its_error_before_ending_the_turn() {
        // A failed result must emit an `Error` first, otherwise the turn ends
        // with no visible reply — the desktop stops crash-reporting at TurnDone.
        assert_eq!(
            parse_line(r#"{"type":"result","subtype":"error","is_error":true,"result":"boom"}"#),
            vec![
                AgentEvent::Error { message: "boom".into() },
                AgentEvent::TurnDone,
            ]
        );
    }

    #[test]
    fn a_failed_result_with_no_text_still_reports_an_error() {
        let out = parse_line(r#"{"type":"result","subtype":"error","is_error":true}"#);
        assert!(matches!(out.first(), Some(AgentEvent::Error { .. })));
        assert_eq!(out.last(), Some(&AgentEvent::TurnDone));
    }

    #[test]
    fn the_result_text_is_not_re_emitted_as_a_text_delta() {
        let out = parse_line(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"this text already streamed as deltas"}"#,
        );
        assert_eq!(out, vec![AgentEvent::TurnDone]);
    }

    #[test]
    fn the_simple_transcript_fixture_parses_to_the_expected_shape() {
        let raw = include_str!(
            "../tests/fixtures/cursor-simple.stream-json"
        );
        let events: Vec<AgentEvent> = raw.lines().flat_map(parse_line).collect();
        assert_eq!(
            events,
            vec![
                AgentEvent::Thinking { text: "Preparing to reply with".into() },
                AgentEvent::Thinking { text: " exactly \"hello\".".into() },
                AgentEvent::TextDelta { text: "hello".into() },
                AgentEvent::TurnDone,
            ]
        );
    }

    #[test]
    fn the_toolcalls_and_thinking_transcript_fixture_parses_to_the_expected_shape() {
        let raw = include_str!(
            "../tests/fixtures/cursor-toolcalls-and-thinking.jsonl"
        );
        let events: Vec<AgentEvent> = raw.lines().flat_map(parse_line).collect();

        assert_eq!(events.iter().filter(|e| matches!(e, AgentEvent::Thinking { .. })).count(), 22);
        assert_eq!(events.iter().filter(|e| matches!(e, AgentEvent::TextDelta { .. })).count(), 3);
        assert_eq!(events.iter().filter(|e| matches!(e, AgentEvent::ToolCallStart { .. })).count(), 3);
        assert_eq!(events.iter().filter(|e| matches!(e, AgentEvent::ToolResult { .. })).count(), 3);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, AgentEvent::ToolResult { status: ToolStatus::Error, .. }))
                .count(),
            1
        );
        assert!(events.contains(&AgentEvent::TurnDone));

        // The MCP tool lookup (getMcpToolsToolCall) started and completed
        // sharing the same call_id, which itself contains an embedded `\n`.
        let mcp_call_id =
            "call-66b7c85a-02bc-43a3-a19f-5d5e18038a09-0\nfc_95a75205-f73a-97a5-9d18-e3821407dabd_0";
        assert!(events.contains(&AgentEvent::ToolCallStart {
            id: mcp_call_id.into(),
            tool: "getMcpTools".into(),
            args: serde_json::json!({
                "pattern": "echo",
                "toolCallId": mcp_call_id,
            }),
        }));
        assert!(events.contains(&AgentEvent::ToolResult { id: mcp_call_id.into(), status: ToolStatus::Ok }));

        // The blocked local read: the completed line's call_id must match its
        // started line's call_id verbatim, embedded `\n` and all.
        let read_call_id =
            "call-7a856b08-042f-4c3a-8625-ba222350614a-2\nfc_fc0a70f1-704d-99b6-b60b-dcb932bc30c4_1";
        assert!(events.contains(&AgentEvent::ToolCallStart {
            id: read_call_id.into(),
            tool: "read".into(),
            args: serde_json::json!({ "path": "/etc/hosts" }),
        }));
        assert!(events
            .contains(&AgentEvent::ToolResult { id: read_call_id.into(), status: ToolStatus::Error }));
    }
}
