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
/// What a tool's own result says, in a few words, for the transcript's row
/// (#385). Only what the text supports: a count where the result is one list,
/// an object's kind and name, or the first line of plain text or of an error.
/// Any other structured result gets no summary rather than a JSON fragment.
pub fn summarize_result(text: &str, is_error: bool) -> Option<String> {
    let first_line = || text.lines().map(str::trim).find(|l| !l.is_empty()).map(bound);
    if is_error {
        return first_line();
    }
    match serde_json::from_str::<serde_json::Value>(text.trim()) {
        Ok(serde_json::Value::Array(items)) => Some(bound(&format!("{} items", items.len()))),
        Ok(serde_json::Value::Object(map)) => {
            let arrays: Vec<(&String, usize)> =
                map.iter().filter_map(|(k, v)| v.as_array().map(|a| (k, a.len()))).collect();
            if let [(field, n)] = arrays.as_slice() {
                let noun = if *n == 1 { field.strip_suffix('s').unwrap_or(field) } else { field.as_str() };
                return Some(bound(&format!("{n} {noun}")));
            }
            let kind = map.get("kind").and_then(|k| k.as_str());
            let name = map.get("metadata").and_then(|m| m.get("name")).and_then(|n| n.as_str());
            match (kind, name) {
                (Some(kind), Some(name)) => Some(bound(&format!("{kind} {name}"))),
                _ => None,
            }
        }
        Ok(_) => None,
        Err(_) => first_line(),
    }
}

/// Control characters stripped, then cut to 80 characters with `…`.
fn bound(s: &str) -> String {
    let clean: String = s.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= 80 {
        clean
    } else {
        let mut cut: String = clean.chars().take(79).collect();
        cut.push('…');
        cut
    }
}

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
    /// A tool call finished with this status — and, when its result says
    /// something short and honest, what (#385, see [`summarize_result`]).
    ToolResult {
        id: String,
        status: ToolStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
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
        let e = AgentEvent::ToolResult { id: "t1".into(), status: ToolStatus::Ok, summary: None };
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

    /// #385: a tool call's row says what its result says — only as much as the
    /// text supports, and nothing for a structure it cannot read honestly.
    #[test]
    fn a_result_is_summarised_by_what_it_actually_says() {
        let cases: &[(&str, bool, Option<&str>)] = &[
            (r#"{"pods":[{"name":"a"},{"name":"b"},{"name":"c"}]}"#, false, Some("3 pods")),
            (r#"{"replicasets":[{"name":"a"}]}"#, false, Some("1 replicaset")),
            (r#"[1,2]"#, false, Some("2 items")),
            (r#"{"kind":"Pod","metadata":{"name":"api-0"}}"#, false, Some("Pod api-0")),
            (r#"{"a":1,"b":[1],"c":[2]}"#, false, None),
            ("first line\nsecond", false, Some("first line")),
            ("\n\n  spaced  \n", false, Some("spaced")),
            ("consent denied: user declined `k8s.scale`\nmore", true, Some("consent denied: user declined `k8s.scale`")),
            ("", false, None),
        ];
        for (text, is_error, want) in cases {
            assert_eq!(summarize_result(text, *is_error).as_deref(), *want, "for {text:?}");
        }
    }

    #[test]
    fn a_summary_is_bounded_and_clean() {
        let long = "x".repeat(200);
        let got = summarize_result(&long, false).unwrap();
        assert_eq!(got.chars().count(), 80);
        assert!(got.ends_with('…'));
        assert_eq!(summarize_result("a\u{1b}[31mred", false).as_deref(), Some("a[31mred"));
    }

    #[test]
    fn a_result_without_a_summary_serialises_as_before() {
        let v = serde_json::to_value(AgentEvent::ToolResult { id: "t".into(), status: ToolStatus::Ok, summary: None }).unwrap();
        assert!(v.get("summary").is_none(), "got {v}");
    }
}
