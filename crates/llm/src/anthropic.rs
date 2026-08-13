//! Anthropic Messages API adapter: build the streaming request body and parse
//! the SSE event stream back into `StreamItem`s. Both halves are pure — the
//! network client lives in `client.rs` — so the wire mapping is unit-tested
//! against recorded event sequences without a live API.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::types::{StopReason, StreamItem, ToolCall, ToolDef, Turn};

/// Build the JSON body for `POST /v1/messages` with `stream: true`. `system` is
/// the base system prompt; `turns` is the running conversation; `tools` are the
/// srelens MCP tools the model may call.
pub fn build_request(model: &str, max_tokens: u32, system: &str, turns: &[Turn], tools: &[ToolDef]) -> Value {
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "stream": true,
        "system": system,
        "messages": turns.iter().map(message_for_turn).collect::<Vec<_>>(),
        "tools": tools.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.input_schema,
        })).collect::<Vec<_>>(),
    })
}

fn message_for_turn(turn: &Turn) -> Value {
    match turn {
        Turn::User(text) => json!({
            "role": "user",
            "content": [{ "type": "text", "text": text }],
        }),
        Turn::Assistant { text, tool_calls } => {
            let mut content: Vec<Value> = Vec::new();
            if !text.is_empty() {
                content.push(json!({ "type": "text", "text": text }));
            }
            for call in tool_calls {
                content.push(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": call.arguments,
                }));
            }
            json!({ "role": "assistant", "content": content })
        }
        Turn::ToolResults(outcomes) => json!({
            "role": "user",
            "content": outcomes.iter().map(|o| json!({
                "type": "tool_result",
                "tool_use_id": o.id,
                "content": o.content,
                "is_error": o.is_error,
            })).collect::<Vec<_>>(),
        }),
    }
}

/// Accumulated state for one streamed content block (text, thinking, or a
/// tool_use whose JSON arguments arrive as `input_json_delta` fragments).
#[derive(Default)]
struct Block {
    kind: BlockKind,
    tool_id: String,
    tool_name: String,
    json_buf: String,
}

#[derive(Default, PartialEq)]
enum BlockKind {
    #[default]
    Other,
    ToolUse,
}

/// Streaming parser for the Anthropic SSE event stream. Feed it each `data:`
/// payload (the JSON after `data: `) via [`push`]; it dispatches on the event's
/// own `type` field, so the `event:` lines can be ignored entirely. Never
/// errors: an unrecognized or non-JSON payload yields nothing, so a future
/// event type is skipped rather than aborting the turn.
#[derive(Default)]
pub struct Stream {
    blocks: HashMap<u64, Block>,
    stop_reason: Option<PendingStop>,
}

/// A `stop_reason` from `message_delta`, held until `message_stop` decides
/// whether the turn ended cleanly or abnormally (e.g. `refusal`).
enum PendingStop {
    Clean(StopReason),
    Abnormal(String),
}

impl Stream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &str) -> Vec<StreamItem> {
        let data = data.trim();
        if data.is_empty() {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        match v.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                self.on_block_start(&v);
                Vec::new()
            }
            Some("content_block_delta") => self.on_block_delta(&v),
            Some("content_block_stop") => self.on_block_stop(&v),
            Some("message_delta") => {
                if let Some(reason) = v.get("delta").and_then(|d| d.get("stop_reason")).and_then(Value::as_str) {
                    self.stop_reason = Some(map_stop_reason(reason));
                }
                Vec::new()
            }
            Some("message_stop") => match self.stop_reason.take() {
                // A refusal or unknown stop reason is not a normal finish —
                // surface it rather than persisting a partial/empty reply.
                Some(PendingStop::Abnormal(reason)) => {
                    vec![StreamItem::Error(format!("the provider stopped generating: {reason}"))]
                }
                Some(PendingStop::Clean(reason)) => vec![StreamItem::Done(reason)],
                None => vec![StreamItem::Done(StopReason::EndTurn)],
            },
            Some("error") => {
                let msg = v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("the provider reported an error")
                    .to_string();
                vec![StreamItem::Error(msg)]
            }
            _ => Vec::new(),
        }
    }

    fn on_block_start(&mut self, v: &Value) {
        let Some(index) = v.get("index").and_then(Value::as_u64) else { return };
        let cb = v.get("content_block");
        let mut block = Block::default();
        if cb.and_then(|c| c.get("type")).and_then(Value::as_str) == Some("tool_use") {
            block.kind = BlockKind::ToolUse;
            block.tool_id = cb.and_then(|c| c.get("id")).and_then(Value::as_str).unwrap_or("").to_string();
            block.tool_name = cb.and_then(|c| c.get("name")).and_then(Value::as_str).unwrap_or("").to_string();
        }
        self.blocks.insert(index, block);
    }

    fn on_block_delta(&mut self, v: &Value) -> Vec<StreamItem> {
        let Some(index) = v.get("index").and_then(Value::as_u64) else { return Vec::new() };
        let Some(delta) = v.get("delta") else { return Vec::new() };
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                vec![StreamItem::Text(text.to_string())]
            }
            Some("thinking_delta") => {
                let text = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                vec![StreamItem::Thinking(text.to_string())]
            }
            Some("input_json_delta") => {
                if let Some(block) = self.blocks.get_mut(&index) {
                    block.json_buf.push_str(delta.get("partial_json").and_then(Value::as_str).unwrap_or(""));
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_block_stop(&mut self, v: &Value) -> Vec<StreamItem> {
        let Some(index) = v.get("index").and_then(Value::as_u64) else { return Vec::new() };
        let Some(block) = self.blocks.remove(&index) else { return Vec::new() };
        if block.kind != BlockKind::ToolUse {
            return Vec::new();
        }
        // Empty argument stream means a no-arg tool call — default to `{}`.
        // Non-empty but unparseable is different: coercing it to `{}` would
        // run the tool with arguments the model never wrote — error instead.
        let arguments = if block.json_buf.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(&block.json_buf) {
                Ok(v) => v,
                Err(_) => {
                    return vec![StreamItem::Error(format!(
                        "the model produced malformed arguments for tool `{}`; not running it",
                        block.tool_name
                    ))]
                }
            }
        };
        vec![StreamItem::ToolCall(ToolCall {
            id: block.tool_id,
            name: block.tool_name,
            arguments,
            thought_signature: None,
        })]
    }
}

fn map_stop_reason(reason: &str) -> PendingStop {
    match reason {
        "end_turn" | "stop_sequence" => PendingStop::Clean(StopReason::EndTurn),
        "tool_use" => PendingStop::Clean(StopReason::ToolUse),
        "max_tokens" => PendingStop::Clean(StopReason::MaxTokens),
        // `refusal`, `pause_turn`, or anything future: not a normal finish.
        other => PendingStop::Abnormal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolOutcome;

    fn tool(name: &str, read_only: bool) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: format!("{name} desc"),
            input_schema: json!({ "type": "object" }),
            read_only,
        }
    }

    #[test]
    fn request_carries_model_stream_system_and_tools() {
        let req = build_request(
            "claude-opus-4-8",
            2048,
            "you are srelens",
            &[Turn::User("why is web-0 down?".into())],
            &[tool("k8s_listPods", true)],
        );
        assert_eq!(req["model"], "claude-opus-4-8");
        assert_eq!(req["stream"], true);
        assert_eq!(req["max_tokens"], 2048);
        assert_eq!(req["system"], "you are srelens");
        assert_eq!(req["messages"][0]["role"], "user");
        assert_eq!(req["messages"][0]["content"][0]["text"], "why is web-0 down?");
        assert_eq!(req["tools"][0]["name"], "k8s_listPods");
        assert_eq!(req["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn assistant_tool_use_and_tool_results_round_trip_into_content_blocks() {
        let turns = vec![
            Turn::User("scale it".into()),
            Turn::Assistant {
                text: "Scaling now.".into(),
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({ "replicas": 3 }), thought_signature: None }],
            },
            Turn::ToolResults(vec![ToolOutcome {
                id: "call_1".into(),
                name: "k8s_scale".into(),
                content: "scaled".into(),
                is_error: false,
            }]),
        ];
        let req = build_request("m", 1, "sys", &turns, &[]);
        let msgs = &req["messages"];
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[1]["content"][0]["type"], "text");
        assert_eq!(msgs[1]["content"][1]["type"], "tool_use");
        assert_eq!(msgs[1]["content"][1]["id"], "call_1");
        assert_eq!(msgs[1]["content"][1]["input"]["replicas"], 3);
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(msgs[2]["content"][0]["is_error"], false);
    }

    #[test]
    fn an_assistant_turn_with_no_text_omits_the_text_block() {
        let turns = vec![Turn::Assistant {
            text: String::new(),
            tool_calls: vec![ToolCall { id: "c".into(), name: "t".into(), arguments: json!({}), thought_signature: None }],
        }];
        let req = build_request("m", 1, "s", &turns, &[]);
        assert_eq!(req["messages"][0]["content"][0]["type"], "tool_use");
    }

    #[test]
    fn text_deltas_stream_as_text_items() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#),
            vec![]
        );
        assert_eq!(
            s.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#),
            vec![StreamItem::Text("Hello".into())]
        );
        assert_eq!(
            s.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}"#),
            vec![StreamItem::Text(" world".into())]
        );
    }

    #[test]
    fn thinking_deltas_stream_as_thinking_items() {
        let mut s = Stream::new();
        s.push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#);
        assert_eq!(
            s.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#),
            vec![StreamItem::Thinking("hmm".into())]
        );
    }

    #[test]
    fn a_tool_use_block_assembles_streamed_json_into_one_tool_call() {
        let mut s = Stream::new();
        s.push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_9","name":"k8s_scale"}}"#);
        // Arguments arrive as partial-JSON fragments across several deltas.
        assert!(s
            .push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"replic"}}"#)
            .is_empty());
        assert!(s
            .push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"as\": 3}"}}"#)
            .is_empty());
        let items = s.push(r#"{"type":"content_block_stop","index":0}"#);
        assert_eq!(
            items,
            vec![StreamItem::ToolCall(ToolCall {
                id: "call_9".into(),
                name: "k8s_scale".into(),
                arguments: json!({ "replicas": 3 }), thought_signature: None })]
        );
    }

    #[test]
    fn a_no_arg_tool_use_defaults_to_an_empty_object() {
        let mut s = Stream::new();
        s.push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"ping"}}"#);
        let items = s.push(r#"{"type":"content_block_stop","index":0}"#);
        assert_eq!(
            items,
            vec![StreamItem::ToolCall(ToolCall { id: "c".into(), name: "ping".into(), arguments: json!({}), thought_signature: None })]
        );
    }

    #[test]
    fn message_stop_reports_the_stop_reason_from_the_message_delta() {
        let mut s = Stream::new();
        s.push(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#);
        assert_eq!(s.push(r#"{"type":"message_stop"}"#), vec![StreamItem::Done(StopReason::ToolUse)]);

        let mut s2 = Stream::new();
        s2.push(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#);
        assert_eq!(s2.push(r#"{"type":"message_stop"}"#), vec![StreamItem::Done(StopReason::EndTurn)]);

        // A token-limit cutoff is truncation, not a normal finish.
        let mut s3 = Stream::new();
        s3.push(r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#);
        assert_eq!(s3.push(r#"{"type":"message_stop"}"#), vec![StreamItem::Done(StopReason::MaxTokens)]);

        // A refusal is not a normal finish — it surfaces as an error.
        let mut s4 = Stream::new();
        s4.push(r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#);
        assert_eq!(
            s4.push(r#"{"type":"message_stop"}"#),
            vec![StreamItem::Error("the provider stopped generating: refusal".into())]
        );
    }

    #[test]
    fn malformed_tool_arguments_become_an_error_not_an_empty_call() {
        let mut s = Stream::new();
        s.push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"k8s_scale"}}"#);
        s.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"replicas\": oops"}}"#);
        assert_eq!(
            s.push(r#"{"type":"content_block_stop","index":0}"#),
            vec![StreamItem::Error(
                "the model produced malformed arguments for tool `k8s_scale`; not running it".into()
            )]
        );
    }

    #[test]
    fn a_stop_with_no_prior_stop_reason_defaults_to_end_turn() {
        let mut s = Stream::new();
        assert_eq!(s.push(r#"{"type":"message_stop"}"#), vec![StreamItem::Done(StopReason::EndTurn)]);
    }

    #[test]
    fn an_error_event_surfaces_its_message() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#),
            vec![StreamItem::Error("Overloaded".into())]
        );
    }

    #[test]
    fn a_blank_or_unknown_payload_yields_nothing() {
        let mut s = Stream::new();
        assert!(s.push("").is_empty());
        assert!(s.push("   ").is_empty());
        assert!(s.push("not json").is_empty());
        assert!(s.push(r#"{"type":"message_start","message":{}}"#).is_empty());
        assert!(s.push(r#"{"type":"ping"}"#).is_empty());
    }
}
