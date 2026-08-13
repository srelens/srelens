//! The transport-agnostic event stream every agent adapter normalizes to.

use serde::{Deserialize, Serialize};

/// Outcome of a tool call, as the drawer shows it on a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Ok,
    Error,
    Denied,
}

/// Stable prefix the srelens MCP server puts on the text of a consent-denied
/// tool result (`srelens_mcp::stdio::DENIED_PREFIX` — same value; neither
/// crate depends on the other, and the desktop crate pins the two equal).
/// CLI transports strip the MCP `_meta` denial marker, so this text is the
/// only signal that survives an agent CLI's transcript.
pub const DENIED_PREFIX: &str = "consent denied: ";

/// Whether a failed tool result's text is a consent refusal rather than an
/// execution error. `contains`, not `starts_with`: a CLI may wrap the tool
/// text (e.g. "Error: …"), and result text only ever comes from our own MCP
/// server, so a false positive would require the server itself to emit the
/// marker mid-output.
pub fn is_denial_text(text: &str) -> bool {
    text.contains(DENIED_PREFIX)
}

/// One normalized event from any agent CLI. `#[serde(tag = "type")]` so the
/// WebView switches on a single discriminant, camelCase to match the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    /// A chunk of streamed assistant text.
    TextDelta { text: String },
    /// The agent has begun a tool call. `id` correlates with the matching
    /// `ToolResult`.
    ToolCallStart { id: String, tool: String, args: serde_json::Value },
    /// A tool call finished with this status.
    ToolResult { id: String, status: ToolStatus },
    /// A chunk of the agent's internal reasoning/thinking, shown separately
    /// from its final response text.
    Thinking { text: String },
    /// The agent finished this turn and is waiting for the next user message.
    TurnDone,
    /// A fatal error for this turn (parse failure, process died, transport).
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta_serializes_with_a_tagged_type() {
        let e = AgentEvent::TextDelta { text: "hi".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "textDelta");
        assert_eq!(v["text"], "hi");
    }

    #[test]
    fn tool_call_start_carries_name_and_args() {
        let e = AgentEvent::ToolCallStart {
            id: "t1".into(),
            tool: "k8s.listPods".into(),
            args: serde_json::json!({ "namespace": "default" }),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "toolCallStart");
        assert_eq!(v["tool"], "k8s.listPods");
        assert_eq!(v["args"]["namespace"], "default");
    }

    #[test]
    fn tool_result_reports_a_status() {
        let e = AgentEvent::ToolResult { id: "t1".into(), status: ToolStatus::Ok };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "toolResult");
        assert_eq!(v["status"], "ok");
    }

    #[test]
    fn thinking_serializes_with_a_tagged_type() {
        let e = AgentEvent::Thinking { text: "pondering...".into() };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "thinking");
        assert_eq!(v["text"], "pondering...");
    }

    #[test]
    fn error_and_turn_done_are_distinct_variants() {
        assert_eq!(serde_json::to_value(AgentEvent::TurnDone).unwrap()["type"], "turnDone");
        let err = AgentEvent::Error { message: "boom".into() };
        assert_eq!(serde_json::to_value(&err).unwrap()["type"], "error");
    }
}
