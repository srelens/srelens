//! Google Gemini `generateContent` adapter: build the streaming request body
//! and parse the SSE stream into `StreamItem`s. Gemini differs from the others
//! in three ways handled here: roles are `user`/`model`, tool results correlate
//! by function *name* (not id), and a `functionCall` part arrives whole rather
//! than as streamed JSON fragments. The model id lives in the URL, not the body.

use serde_json::{json, Value};

use crate::types::{StopReason, StreamItem, ToolCall, ToolDef, Turn};

/// Build the JSON body for `POST /v1beta/models/{model}:streamGenerateContent`.
pub fn build_request(system: &str, turns: &[Turn], tools: &[ToolDef]) -> Value {
    let mut body = json!({
        "system_instruction": { "parts": [{ "text": system }] },
        "contents": turns.iter().map(content_for_turn).collect::<Vec<_>>(),
    });
    if !tools.is_empty() {
        body["tools"] = json!([{
            "function_declarations": tools.iter().map(|t| json!({
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema,
            })).collect::<Vec<_>>(),
        }]);
    }
    body
}

fn content_for_turn(turn: &Turn) -> Value {
    match turn {
        Turn::User(text) => json!({ "role": "user", "parts": [{ "text": text }] }),
        Turn::Assistant { text, tool_calls } => {
            let mut parts: Vec<Value> = Vec::new();
            if !text.is_empty() {
                parts.push(json!({ "text": text }));
            }
            for call in tool_calls {
                let mut part = json!({ "functionCall": { "name": call.name, "args": call.arguments } });
                // Replay the thinking signature on the part it arrived on —
                // signature-requiring models reject the request without it.
                if let Some(sig) = &call.thought_signature {
                    part["thoughtSignature"] = json!(sig);
                }
                parts.push(part);
            }
            json!({ "role": "model", "parts": parts })
        }
        Turn::ToolResults(outcomes) => json!({
            "role": "user",
            "parts": outcomes.iter().map(|o| {
                // Gemini's functionResponse.response is an object; wrap the tool
                // text under `result` (or `error` on failure) and correlate by name.
                let key = if o.is_error { "error" } else { "result" };
                json!({ "functionResponse": { "name": o.name, "response": { key: o.content } } })
            }).collect::<Vec<_>>(),
        }),
    }
}

/// Streaming parser for the Gemini SSE stream (`?alt=sse`). Feed it each `data:`
/// payload. Gemini has no per-call id, so this synthesizes stable ids for
/// `AgentEvent` correlation; the next request correlates results by name.
#[derive(Default)]
pub struct Stream {
    round: u64,
    counter: u64,
    done: bool,
}

impl Stream {
    pub fn new() -> Self {
        Self::default()
    }

    /// A parser stamped with the tool round it belongs to. Each round of a
    /// user turn gets a fresh `Stream` (and so a reset `counter`), so the
    /// round must be part of the synthesized id or the first call of every
    /// round would collide as `gemini-call-0`.
    pub fn for_round(round: u64) -> Self {
        Self { round, ..Self::default() }
    }

    pub fn push(&mut self, data: &str) -> Vec<StreamItem> {
        let data = data.trim();
        if data.is_empty() {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        if let Some(msg) = v.get("error").and_then(|e| e.get("message")).and_then(Value::as_str) {
            return vec![StreamItem::Error(msg.to_string())];
        }
        let Some(candidate) = v.get("candidates").and_then(|c| c.get(0)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(parts) = candidate.get("content").and_then(|c| c.get("parts")).and_then(Value::as_array) {
            for part in parts {
                if let Some(call) = part.get("functionCall") {
                    let name = call.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    let arguments = call.get("args").cloned().unwrap_or_else(|| json!({}));
                    let id = format!("gemini-call-{}-{}", self.round, self.counter);
                    self.counter += 1;
                    // Thinking models stamp the part with an opaque signature
                    // that must be replayed with this call in the next request.
                    let thought_signature =
                        part.get("thoughtSignature").and_then(Value::as_str).map(str::to_string);
                    out.push(StreamItem::ToolCall(ToolCall { id, name, arguments, thought_signature }));
                } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if part.get("thought").and_then(Value::as_bool) == Some(true) {
                        out.push(StreamItem::Thinking(text.to_string()));
                    } else if !text.is_empty() {
                        out.push(StreamItem::Text(text.to_string()));
                    }
                }
            }
        }
        // Gemini has no distinct tool-use stop reason; the loop keys off whether
        // any tool call was emitted, so EndTurn is correct for a normal STOP.
        // Anything else is not a normal finish: MAX_TOKENS is the truncation it
        // is, and the abnormal reasons (SAFETY, RECITATION, BLOCKLIST,
        // MALFORMED_FUNCTION_CALL, …) surface as errors — otherwise a partial
        // or empty reply would persist as a completed turn with no explanation.
        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
            if !self.done {
                self.done = true;
                out.push(match reason {
                    "STOP" => StreamItem::Done(StopReason::EndTurn),
                    "MAX_TOKENS" => StreamItem::Done(StopReason::MaxTokens),
                    other => StreamItem::Error(format!("Gemini stopped generating: {other}")),
                });
            }
        }
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
    fn request_carries_system_instruction_contents_and_function_declarations() {
        let req = build_request("sys", &[Turn::User("hi".into())], &[tool("k8s_listPods")]);
        assert_eq!(req["system_instruction"]["parts"][0]["text"], "sys");
        assert_eq!(req["contents"][0]["role"], "user");
        assert_eq!(req["contents"][0]["parts"][0]["text"], "hi");
        assert_eq!(req["tools"][0]["function_declarations"][0]["name"], "k8s_listPods");
    }

    #[test]
    fn tools_are_omitted_when_there_are_none() {
        let req = build_request("s", &[Turn::User("hi".into())], &[]);
        assert!(req.get("tools").is_none());
    }

    #[test]
    fn assistant_uses_the_model_role_and_tool_results_correlate_by_name() {
        let turns = vec![
            Turn::Assistant {
                text: "scaling".into(),
                tool_calls: vec![ToolCall {
                    id: "gemini-call-0".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({ "replicas": 3 }), thought_signature: None }],
            },
            Turn::ToolResults(vec![ToolOutcome {
                id: "gemini-call-0".into(),
                name: "k8s_scale".into(),
                content: "scaled".into(),
                is_error: false,
            }]),
        ];
        let req = build_request("s", &turns, &[]);
        assert_eq!(req["contents"][0]["role"], "model");
        assert_eq!(req["contents"][0]["parts"][1]["functionCall"]["name"], "k8s_scale");
        assert_eq!(req["contents"][0]["parts"][1]["functionCall"]["args"]["replicas"], 3);
        assert_eq!(req["contents"][1]["role"], "user");
        assert_eq!(req["contents"][1]["parts"][0]["functionResponse"]["name"], "k8s_scale");
        assert_eq!(req["contents"][1]["parts"][0]["functionResponse"]["response"]["result"], "scaled");
    }

    #[test]
    fn a_failed_tool_result_is_keyed_under_error() {
        let turns = vec![Turn::ToolResults(vec![ToolOutcome {
            id: "x".into(),
            name: "k8s_scale".into(),
            content: "denied".into(),
            is_error: true,
        }])];
        let req = build_request("s", &turns, &[]);
        assert_eq!(req["contents"][0]["parts"][0]["functionResponse"]["response"]["error"], "denied");
    }

    #[test]
    fn text_parts_stream_as_text_and_thought_parts_as_thinking() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]}"#),
            vec![StreamItem::Text("Hello".into())]
        );
        assert_eq!(
            s.push(r#"{"candidates":[{"content":{"parts":[{"text":"reasoning","thought":true}]}}]}"#),
            vec![StreamItem::Thinking("reasoning".into())]
        );
    }

    #[test]
    fn a_function_call_part_becomes_a_tool_call_with_a_synthesized_id() {
        let mut s = Stream::new();
        let items = s.push(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"k8s_scale","args":{"replicas":2}}}]}}]}"#,
        );
        assert_eq!(
            items,
            vec![StreamItem::ToolCall(ToolCall {
                id: "gemini-call-0-0".into(),
                name: "k8s_scale".into(),
                arguments: json!({ "replicas": 2 }), thought_signature: None })]
        );
        // A second call in the same round gets a distinct id.
        let more = s.push(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"k8s_listPods","args":{}}}]}}]}"#,
        );
        assert_eq!(
            more,
            vec![StreamItem::ToolCall(ToolCall {
                id: "gemini-call-0-1".into(),
                name: "k8s_listPods".into(),
                arguments: json!({}), thought_signature: None })]
        );
    }

    #[test]
    fn a_later_round_synthesizes_ids_distinct_from_round_zero() {
        let mut s = Stream::for_round(2);
        let items = s.push(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"k8s_scale","args":{}}}]}}]}"#,
        );
        assert_eq!(
            items,
            vec![StreamItem::ToolCall(ToolCall {
                id: "gemini-call-2-0".into(),
                name: "k8s_scale".into(),
                arguments: json!({}), thought_signature: None })]
        );
    }

    #[test]
    fn a_finish_reason_ends_the_turn_once() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"candidates":[{"content":{"parts":[{"text":"done"}]},"finishReason":"STOP"}]}"#),
            vec![StreamItem::Text("done".into()), StreamItem::Done(StopReason::EndTurn)]
        );
        // A trailing empty candidate with another finishReason doesn't re-emit Done.
        assert!(s.push(r#"{"candidates":[{"content":{"parts":[]},"finishReason":"STOP"}]}"#).is_empty());
    }

    #[test]
    fn a_max_tokens_finish_reports_truncation() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}]}"#),
            vec![StreamItem::Text("partial".into()), StreamItem::Done(StopReason::MaxTokens)]
        );
    }

    #[test]
    fn a_thought_signature_is_captured_and_replayed_on_the_reconstructed_call() {
        // Thinking models stamp `thoughtSignature` on the functionCall part;
        // it must ride along on the next request or the follow-up is rejected.
        let mut s = Stream::new();
        let items = s.push(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"k8s_scale","args":{}},"thoughtSignature":"sig-abc"}]}}]}"#,
        );
        let StreamItem::ToolCall(call) = &items[0] else { panic!("expected a tool call, got {items:?}") };
        assert_eq!(call.thought_signature.as_deref(), Some("sig-abc"));

        let turns = vec![Turn::Assistant { text: String::new(), tool_calls: vec![call.clone()] }];
        let req = build_request("s", &turns, &[]);
        assert_eq!(req["contents"][0]["parts"][0]["thoughtSignature"], "sig-abc");
        // A signature-less call (every non-Gemini adapter) adds no field.
        let bare = ToolCall { name: "k8s_scale".into(), ..Default::default() };
        let req2 = build_request("s", &[Turn::Assistant { text: String::new(), tool_calls: vec![bare] }], &[]);
        assert!(req2["contents"][0]["parts"][0].get("thoughtSignature").is_none());
    }

    #[test]
    fn an_abnormal_finish_reason_surfaces_as_an_error_not_a_clean_end() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"candidates":[{"content":{"parts":[]},"finishReason":"SAFETY"}]}"#),
            vec![StreamItem::Error("Gemini stopped generating: SAFETY".into())]
        );
        let mut s2 = Stream::new();
        assert_eq!(
            s2.push(r#"{"candidates":[{"content":{"parts":[]},"finishReason":"MALFORMED_FUNCTION_CALL"}]}"#),
            vec![StreamItem::Error("Gemini stopped generating: MALFORMED_FUNCTION_CALL".into())]
        );
    }

    #[test]
    fn an_error_payload_surfaces_its_message() {
        let mut s = Stream::new();
        assert_eq!(
            s.push(r#"{"error":{"code":429,"message":"Resource exhausted"}}"#),
            vec![StreamItem::Error("Resource exhausted".into())]
        );
    }

    #[test]
    fn blank_and_unknown_payloads_yield_nothing() {
        let mut s = Stream::new();
        assert!(s.push("").is_empty());
        assert!(s.push("not json").is_empty());
        assert!(s.push(r#"{"candidates":[]}"#).is_empty());
    }
}
