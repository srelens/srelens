//! OpenAI Chat Completions adapter: build the streaming request body and parse
//! the SSE stream into `StreamItem`s. Reused verbatim for the OpenAI-compatible
//! custom-base-URL provider — only the base URL and key differ, which are the
//! client's concern, not this pure wire mapping.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::types::{StopReason, StreamItem, ToolCall, ToolDef, Turn};

/// Build the JSON body for `POST /v1/chat/completions` with `stream: true`.
pub fn build_request(model: &str, system: &str, turns: &[Turn], tools: &[ToolDef]) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": system })];
    for turn in turns {
        append_turn(&mut messages, turn);
    }
    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools
            .iter()
            .map(|t| json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                },
            }))
            .collect::<Vec<_>>());
    }
    body
}

fn append_turn(messages: &mut Vec<Value>, turn: &Turn) {
    match turn {
        Turn::User(text) => messages.push(json!({ "role": "user", "content": text })),
        Turn::Assistant { text, tool_calls } => {
            let mut msg = json!({ "role": "assistant" });
            // OpenAI wants `content: null` (not "") when the turn is only tool calls.
            msg["content"] = if text.is_empty() { Value::Null } else { json!(text) };
            if !tool_calls.is_empty() {
                msg["tool_calls"] = json!(tool_calls
                    .iter()
                    .map(|c| json!({
                        "id": c.id,
                        "type": "function",
                        "function": {
                            "name": c.name,
                            // OpenAI carries arguments as a JSON *string*, not an object.
                            "arguments": c.arguments.to_string(),
                        },
                    }))
                    .collect::<Vec<_>>());
            }
            messages.push(msg);
        }
        Turn::ToolResults(outcomes) => {
            for o in outcomes {
                // OpenAI tool messages have no error flag; mark failures inline
                // so the model still sees that the call failed.
                let content = if o.is_error { format!("Error: {}", o.content) } else { o.content.clone() };
                messages.push(json!({ "role": "tool", "tool_call_id": o.id, "content": content }));
            }
        }
    }
}

/// One tool call being assembled across streamed fragments (id and name arrive
/// on the first fragment for an index; `arguments` accumulate as a JSON string).
#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

/// Streaming parser for the OpenAI Chat Completions SSE stream. Feed it each
/// `data:` payload; `[DONE]` terminates. Never errors on unrecognized input.
#[derive(Default)]
pub struct Stream {
    /// Keyed by the `index` OpenAI assigns each streamed tool call, so fragments
    /// are reassembled in call order regardless of interleaving.
    calls: BTreeMap<u64, PartialCall>,
    done: bool,
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
        if data == "[DONE]" {
            // A stream that ended without a `finish_reason` still closes cleanly.
            return if self.done { Vec::new() } else { self.finish(StopReason::EndTurn) };
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        if let Some(msg) = v.get("error").and_then(|e| e.get("message")).and_then(Value::as_str) {
            return vec![StreamItem::Error(msg.to_string())];
        }
        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    out.push(StreamItem::Text(text.to_string()));
                }
            }
            // Some OpenAI-compatible servers stream reasoning separately.
            if let Some(t) = delta.get("reasoning_content").and_then(Value::as_str) {
                if !t.is_empty() {
                    out.push(StreamItem::Thinking(t.to_string()));
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    self.accumulate_call(call);
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            // Only genuinely successful finishes map to clean stop reasons.
            // `content_filter` and anything else abnormal must surface as an
            // error, or a filtered/partial reply would persist as a normal
            // completed turn with no explanation.
            match reason {
                "stop" => out.extend(self.finish(StopReason::EndTurn)),
                "tool_calls" => out.extend(self.finish(StopReason::ToolUse)),
                "length" => out.extend(self.finish(StopReason::MaxTokens)),
                other => {
                    if !self.done {
                        self.done = true;
                        out.push(StreamItem::Error(format!("the provider stopped generating: {other}")));
                    }
                }
            }
        }
        out
    }

    fn accumulate_call(&mut self, call: &Value) {
        let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
        let entry = self.calls.entry(index).or_default();
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                entry.id = id.to_string();
            }
        }
        if let Some(func) = call.get("function") {
            if let Some(name) = func.get("name").and_then(Value::as_str) {
                if !name.is_empty() {
                    entry.name = name.to_string();
                }
            }
            if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                entry.args.push_str(args);
            }
        }
    }

    fn finish(&mut self, reason: StopReason) -> Vec<StreamItem> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut out = Vec::new();
        for (_, call) in std::mem::take(&mut self.calls) {
            // Empty argument stream is a legitimate no-arg call. Non-empty but
            // unparseable is NOT: coercing it to `{}` would run the tool with
            // arguments materially different from what the model was writing,
            // so surface it as an error (which discards the turn) instead.
            let arguments = if call.args.trim().is_empty() {
                json!({})
            } else {
                match serde_json::from_str(&call.args) {
                    Ok(v) => v,
                    Err(_) => {
                        out.push(StreamItem::Error(format!(
                            "the model produced malformed arguments for tool `{}`; not running it",
                            call.name
                        )));
                        continue;
                    }
                }
            };
            out.push(StreamItem::ToolCall(ToolCall {
                id: call.id,
                name: call.name,
                arguments,
                thought_signature: None,
            }));
        }
        out.push(StreamItem::Done(reason));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolOutcome;

    fn tool(name: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: format!("{name} desc"),
            input_schema: json!({ "type": "object" }),
            read_only: true,
        }
    }

    #[test]
    fn request_puts_the_system_prompt_first_and_maps_tools_to_functions() {
        let req = build_request("gpt-5", "sys", &[Turn::User("hi".into())], &[tool("k8s_listPods")]);
        assert_eq!(req["model"], "gpt-5");
        assert_eq!(req["stream"], true);
        assert_eq!(req["messages"][0]["role"], "system");
        assert_eq!(req["messages"][0]["content"], "sys");
        assert_eq!(req["messages"][1]["role"], "user");
        assert_eq!(req["messages"][1]["content"], "hi");
        assert_eq!(req["tools"][0]["type"], "function");
        assert_eq!(req["tools"][0]["function"]["name"], "k8s_listPods");
        assert_eq!(req["tools"][0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn tools_key_is_omitted_when_there_are_none() {
        let req = build_request("m", "s", &[Turn::User("hi".into())], &[]);
        assert!(req.get("tools").is_none());
    }

    #[test]
    fn assistant_tool_calls_serialize_arguments_as_a_json_string_and_tool_results_are_tool_messages() {
        let turns = vec![
            Turn::Assistant {
                text: String::new(),
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
        let req = build_request("m", "s", &turns, &[]);
        let assistant = &req["messages"][1];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["content"], Value::Null);
        assert_eq!(assistant["tool_calls"][0]["id"], "call_1");
        assert_eq!(assistant["tool_calls"][0]["type"], "function");
        // arguments is a JSON string, not an object.
        assert_eq!(assistant["tool_calls"][0]["function"]["arguments"], "{\"replicas\":3}");
        let tool_msg = &req["messages"][2];
        assert_eq!(tool_msg["role"], "tool");
        assert_eq!(tool_msg["tool_call_id"], "call_1");
        assert_eq!(tool_msg["content"], "scaled");
    }

    #[test]
    fn a_failed_tool_result_is_marked_inline() {
        let turns = vec![Turn::ToolResults(vec![ToolOutcome {
            id: "c".into(),
            name: "t".into(),
            content: "boom".into(),
            is_error: true,
        }])];
        let req = build_request("m", "s", &turns, &[]);
        assert_eq!(req["messages"][1]["content"], "Error: boom");
    }

    #[test]
    fn content_deltas_stream_as_text_items() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"choices":[{"delta":{"content":"Hel"}}]}"#),
            vec![StreamItem::Text("Hel".into())]
        );
        assert_eq!(
            s.push(r#"{"choices":[{"delta":{"content":"lo"}}]}"#),
            vec![StreamItem::Text("lo".into())]
        );
    }

    #[test]
    fn reasoning_content_streams_as_thinking() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"choices":[{"delta":{"reasoning_content":"let me think"}}]}"#),
            vec![StreamItem::Thinking("let me think".into())]
        );
    }

    #[test]
    fn malformed_streamed_arguments_become_an_error_not_an_empty_call() {
        let mut s = Stream::new();
        // The stream dies mid-arguments in a way that still finishes with
        // tool_calls — the truncated JSON must not coerce to `{}` and run.
        assert!(s
            .push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"k8s_scale","arguments":"{\"replicas\": oops"}}]}}]}"#)
            .is_empty());
        let items = s.push(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#);
        assert_eq!(
            items,
            vec![
                StreamItem::Error(
                    "the model produced malformed arguments for tool `k8s_scale`; not running it".into()
                ),
                StreamItem::Done(StopReason::ToolUse),
            ]
        );
    }

    #[test]
    fn streamed_tool_call_fragments_assemble_and_flush_on_finish() {
        let mut s = Stream::new();
        assert!(s
            .push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"k8s_scale","arguments":"{\"repl"}}]}}]}"#)
            .is_empty());
        assert!(s
            .push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"icas\": 2}"}}]}}]}"#)
            .is_empty());
        let items = s.push(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#);
        assert_eq!(
            items,
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "call_1".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({ "replicas": 2 }), thought_signature: None }),
                StreamItem::Done(StopReason::ToolUse),
            ]
        );
    }

    #[test]
    fn finish_reason_stop_ends_the_turn() {
        let mut s = Stream::new();
        s.push(r#"{"choices":[{"delta":{"content":"done"}}]}"#);
        assert_eq!(
            s.push(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
            vec![StreamItem::Done(StopReason::EndTurn)]
        );
        // A trailing [DONE] after finish is a no-op, not a second Done.
        assert!(s.push("[DONE]").is_empty());

        // `length` (the token-limit cutoff) is truncation, not a normal finish.
        let mut s2 = Stream::new();
        assert_eq!(
            s2.push(r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#),
            vec![StreamItem::Done(StopReason::MaxTokens)]
        );

        // `content_filter` (or anything else abnormal) is an error, not a
        // clean end — and the trailing [DONE] must not add a Done after it.
        let mut s3 = Stream::new();
        assert_eq!(
            s3.push(r#"{"choices":[{"delta":{},"finish_reason":"content_filter"}]}"#),
            vec![StreamItem::Error("the provider stopped generating: content_filter".into())]
        );
        assert!(s3.push("[DONE]").is_empty());
    }

    #[test]
    fn a_done_sentinel_without_a_finish_reason_still_closes_the_turn() {
        let mut s = Stream::new();
        s.push(r#"{"choices":[{"delta":{"content":"hi"}}]}"#);
        assert_eq!(s.push("[DONE]"), vec![StreamItem::Done(StopReason::EndTurn)]);
    }

    #[test]
    fn an_error_payload_surfaces_its_message() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"error":{"message":"invalid_api_key","type":"auth"}}"#),
            vec![StreamItem::Error("invalid_api_key".into())]
        );
    }

    #[test]
    fn blank_and_unknown_payloads_yield_nothing() {
        let mut s = Stream::new();
        assert!(s.push("").is_empty());
        assert!(s.push("not json").is_empty());
        assert!(s.push(r#"{"choices":[]}"#).is_empty());
    }
}
