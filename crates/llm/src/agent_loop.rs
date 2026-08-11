//! The provider-agnostic agentic loop: stream a turn, run any tool calls the
//! model made through the MCP boundary, feed the results back, and repeat until
//! the model replies without calling tools. Written against the `Provider` and
//! `ToolInvoker` traits so it is unit-tested with stubs and run with real
//! network clients unchanged.

use srelens_agent::event::{AgentEvent, ToolStatus};

use crate::error::LlmError;
use crate::provider::{Provider, ToolInvoker};
use crate::types::{StreamItem, ToolCall, ToolOutcome, Turn};

/// Cap on tool-use round-trips per user turn, so a model that keeps calling
/// tools without ever finishing can't run unbounded.
const MAX_ROUNDS: usize = 24;

/// Drive one user turn to completion, emitting `AgentEvent`s as it goes. Returns
/// `Err` only for a setup failure (e.g. tools couldn't be listed); provider and
/// tool errors are surfaced as `AgentEvent::Error` and end the turn cleanly, so
/// the caller always sees a `TurnDone`.
pub async fn run(
    provider: &dyn Provider,
    invoker: &dyn ToolInvoker,
    history: Vec<Turn>,
    prompt: String,
    on_event: &mut (dyn FnMut(AgentEvent) + Send),
) -> Result<(), LlmError> {
    let tools = invoker.list_tools().await?;
    let mut turns = history;
    turns.push(Turn::User(prompt));

    for _ in 0..MAX_ROUNDS {
        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut stream_error: Option<String> = None;

        {
            let mut on_item = |item: StreamItem| match item {
                StreamItem::Text(t) => {
                    text.push_str(&t);
                    on_event(AgentEvent::TextDelta { text: t });
                }
                StreamItem::Thinking(t) => on_event(AgentEvent::Thinking { text: t }),
                StreamItem::ToolCall(c) => calls.push(c),
                StreamItem::Done(_) => {}
                StreamItem::Error(e) => stream_error = Some(e),
            };
            provider.stream_turn(&turns, &tools, &mut on_item).await?;
        }

        if let Some(message) = stream_error {
            on_event(AgentEvent::Error { message });
            on_event(AgentEvent::TurnDone);
            return Ok(());
        }

        // No tool calls → the model gave its final reply; the turn is done.
        if calls.is_empty() {
            on_event(AgentEvent::TurnDone);
            return Ok(());
        }

        // Record what the model said and requested, then run each call.
        turns.push(Turn::Assistant { text: text.clone(), tool_calls: calls.clone() });
        let mut outcomes = Vec::with_capacity(calls.len());
        for call in &calls {
            on_event(AgentEvent::ToolCallStart {
                id: call.id.clone(),
                tool: call.name.clone(),
                args: call.arguments.clone(),
            });
            let outcome = invoke_one(invoker, call, on_event).await;
            outcomes.push(outcome);
        }
        turns.push(Turn::ToolResults(outcomes));
    }

    on_event(AgentEvent::Error {
        message: "the assistant kept calling tools without finishing; stopping this turn.".into(),
    });
    on_event(AgentEvent::TurnDone);
    Ok(())
}

/// Run one tool call, emit its `ToolResult`, and return the outcome to feed
/// back to the model. A transport error is reported to the model as a failed
/// result rather than aborting the whole turn.
async fn invoke_one(
    invoker: &dyn ToolInvoker,
    call: &ToolCall,
    on_event: &mut (dyn FnMut(AgentEvent) + Send),
) -> ToolOutcome {
    match invoker.call_tool(&call.name, &call.arguments).await {
        Ok(res) => {
            let status = if res.denied {
                ToolStatus::Denied
            } else if res.is_error {
                ToolStatus::Error
            } else {
                ToolStatus::Ok
            };
            on_event(AgentEvent::ToolResult { id: call.id.clone(), status });
            // A denied call is fed back as an error so the model can adapt.
            ToolOutcome {
                id: call.id.clone(),
                name: call.name.clone(),
                content: if res.denied && res.content.is_empty() {
                    "the user declined this tool call".to_string()
                } else {
                    res.content
                },
                is_error: res.is_error || res.denied,
            }
        }
        Err(e) => {
            on_event(AgentEvent::ToolResult { id: call.id.clone(), status: ToolStatus::Error });
            ToolOutcome { id: call.id.clone(), name: call.name.clone(), content: e.to_string(), is_error: true }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ToolCallResult;
    use crate::types::{ModelInfo, StopReason, ToolDef};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    /// A provider scripted with one `Vec<StreamItem>` per turn; each call to
    /// `stream_turn` plays the next script and records the turns it was given.
    struct ScriptedProvider {
        scripts: Mutex<std::collections::VecDeque<Vec<StreamItem>>>,
        seen_turns: Mutex<Vec<Vec<Turn>>>,
    }

    impl ScriptedProvider {
        fn new(scripts: Vec<Vec<StreamItem>>) -> Self {
            Self {
                scripts: Mutex::new(scripts.into_iter().collect()),
                seen_turns: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn stream_turn(
            &self,
            turns: &[Turn],
            _tools: &[ToolDef],
            on_item: &mut (dyn FnMut(StreamItem) + Send),
        ) -> Result<(), LlmError> {
            self.seen_turns.lock().unwrap().push(turns.to_vec());
            let script = self.scripts.lock().unwrap().pop_front().unwrap_or_default();
            for item in script {
                on_item(item);
            }
            Ok(())
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
            Ok(vec![])
        }
    }

    struct StubInvoker {
        result: ToolCallResult,
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl ToolInvoker for StubInvoker {
        async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
            Ok(vec![ToolDef {
                name: "k8s_scale".into(),
                description: "scale".into(),
                input_schema: json!({ "type": "object" }),
                read_only: false,
            }])
        }

        async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolCallResult, LlmError> {
            self.calls.lock().unwrap().push((name.to_string(), args.clone()));
            Ok(self.result.clone())
        }
    }

    fn drive(provider: &dyn Provider, invoker: &dyn ToolInvoker, prompt: &str) -> Vec<AgentEvent> {
        let events = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let mut on_event = move |e: AgentEvent| sink.lock().unwrap().push(e);
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(run(provider, invoker, Vec::new(), prompt.to_string(), &mut on_event)).unwrap();
        drop(on_event);
        let collected = events.lock().unwrap().clone();
        collected
    }

    #[test]
    fn a_reply_with_no_tool_calls_streams_text_then_turn_done() {
        let provider = ScriptedProvider::new(vec![vec![
            StreamItem::Text("all healthy".into()),
            StreamItem::Done(StopReason::EndTurn),
        ]]);
        let invoker = StubInvoker {
            result: ToolCallResult { content: String::new(), is_error: false, denied: false },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "status?");
        assert_eq!(
            events,
            vec![
                AgentEvent::TextDelta { text: "all healthy".into() },
                AgentEvent::TurnDone,
            ]
        );
        assert!(invoker.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_tool_call_runs_then_the_model_gets_the_result_and_finishes() {
        // Turn 1: the model asks to scale. Turn 2 (after the tool runs): it replies.
        let provider = ScriptedProvider::new(vec![
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({ "replicas": 3 }),
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            vec![StreamItem::Text("scaled to 3".into()), StreamItem::Done(StopReason::EndTurn)],
        ]);
        let invoker = StubInvoker {
            result: ToolCallResult { content: "ok".into(), is_error: false, denied: false },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "scale web to 3");

        assert_eq!(
            events,
            vec![
                AgentEvent::ToolCallStart {
                    id: "c1".into(),
                    tool: "k8s_scale".into(),
                    args: json!({ "replicas": 3 }),
                },
                AgentEvent::ToolResult { id: "c1".into(), status: ToolStatus::Ok },
                AgentEvent::TextDelta { text: "scaled to 3".into() },
                AgentEvent::TurnDone,
            ]
        );
        // The tool was actually invoked with the model's args.
        assert_eq!(invoker.calls.lock().unwrap().as_slice(), &[("k8s_scale".into(), json!({ "replicas": 3 }))]);
        // The second provider request carried the tool result back.
        let seen = provider.seen_turns.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(matches!(seen[1].last(), Some(Turn::ToolResults(o)) if o[0].content == "ok"));
    }

    #[test]
    fn a_denied_tool_call_reports_denied_and_feeds_that_back() {
        let provider = ScriptedProvider::new(vec![
            vec![
                StreamItem::ToolCall(ToolCall { id: "c1".into(), name: "k8s_scale".into(), arguments: json!({}) }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            vec![StreamItem::Text("ok, leaving it".into()), StreamItem::Done(StopReason::EndTurn)],
        ]);
        let invoker = StubInvoker {
            result: ToolCallResult { content: String::new(), is_error: false, denied: true },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "scale it");
        assert!(events.contains(&AgentEvent::ToolResult { id: "c1".into(), status: ToolStatus::Denied }));
        let seen = provider.seen_turns.lock().unwrap();
        assert!(matches!(seen[1].last(), Some(Turn::ToolResults(o)) if o[0].is_error));
    }

    #[test]
    fn a_provider_error_surfaces_before_turn_done() {
        let provider = ScriptedProvider::new(vec![vec![StreamItem::Error("Overloaded".into())]]);
        let invoker = StubInvoker {
            result: ToolCallResult { content: String::new(), is_error: false, denied: false },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "hi");
        assert_eq!(
            events,
            vec![AgentEvent::Error { message: "Overloaded".into() }, AgentEvent::TurnDone]
        );
    }
}
