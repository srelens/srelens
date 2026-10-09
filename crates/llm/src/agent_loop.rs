//! The provider-agnostic agentic loop: stream a turn, run any tool calls the
//! model made through the MCP boundary, feed the results back, and repeat until
//! the model replies without calling tools. Written against the `Provider` and
//! `ToolInvoker` traits so it is unit-tested with stubs and run with real
//! network clients unchanged.

use srelens_agent::event::{AgentEvent, ToolStatus};

use crate::error::LlmError;
use crate::provider::{Provider, ToolInvoker};
use crate::types::{StopReason, StreamItem, ToolCall, ToolOutcome, Turn};

/// Cap on tool-use round-trips per user turn, so a model that keeps calling
/// tools without ever finishing can't run unbounded.
const MAX_ROUNDS: usize = 24;

/// Drive one user turn to completion, emitting `AgentEvent`s as it goes. Returns
/// `Err` only for a setup failure (e.g. tools couldn't be listed); provider and
/// tool errors are surfaced as `AgentEvent::Error` and end the turn cleanly, so
/// the caller always sees a `TurnDone`.
///
/// Returns the conversation to continue from on the NEXT user turn. On a normal
/// finish that's `history` + this turn's user message, any tool exchanges, and
/// the assistant's final reply — always ending on an assistant turn, so it's a
/// valid base for the next message. On a provider error or the round-cap
/// backstop the failed turn is discarded and the untouched `history` is returned
/// (never a dangling user/tool-result turn that would make the next request
/// invalid).
pub async fn run(
    provider: &dyn Provider,
    invoker: &dyn ToolInvoker,
    history: Vec<Turn>,
    prompt: String,
    on_event: &mut (dyn FnMut(AgentEvent) + Send),
) -> Result<Vec<Turn>, LlmError> {
    let tools = invoker.list_tools().await?;
    let mut turns = history.clone();
    turns.push(Turn::User(prompt));
    let mut total_usage = crate::types::TokenUsage::default();

    for _ in 0..MAX_ROUNDS {
        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut stream_error: Option<String> = None;
        let mut truncated = false;

        let mut round_usage: Option<crate::types::TokenUsage> = None;

        {
            let mut on_item = |item: StreamItem| match item {
                StreamItem::Text(t) => {
                    text.push_str(&t);
                    on_event(AgentEvent::TextDelta { text: t });
                }
                StreamItem::Thinking(t) => on_event(AgentEvent::Thinking { text: t }),
                StreamItem::ToolCall(c) => calls.push(c),
                StreamItem::Done(reason) => truncated |= reason == StopReason::MaxTokens,
                StreamItem::Error(e) => stream_error = Some(e),
                StreamItem::Usage(u) => {
                    round_usage = Some(u);
                    on_event(AgentEvent::Usage {
                        prompt_tokens: total_usage.prompt_tokens + u.prompt_tokens,
                        completion_tokens: total_usage.completion_tokens + u.completion_tokens,
                        cached_tokens: total_usage.cached_tokens + u.cached_tokens,
                        total_tokens: total_usage.total_tokens + u.total_tokens,
                    });
                }
            };
            let request_turns = compact_request_turns_for_round(&turns);
            provider
                .stream_turn(&request_turns, &tools, &mut on_item)
                .await?;
        }

        if let Some(u) = round_usage {
            total_usage.prompt_tokens += u.prompt_tokens;
            total_usage.completion_tokens += u.completion_tokens;
            total_usage.cached_tokens += u.cached_tokens;
            total_usage.total_tokens += u.total_tokens;
        }

        if let Some(message) = stream_error {
            on_event(AgentEvent::Error { message });
            on_event(AgentEvent::TurnDone);
            // Discard the failed turn: continue next time from the prior history.
            return Ok(history);
        }

        // A truncated round must never run its tool calls: the cutoff can land
        // mid-arguments, and the parsers coerce partial JSON to `{}` — so the
        // call (or the consent dialog shown for it) could carry arguments the
        // model never finished writing. Discard the round instead.
        if truncated && !calls.is_empty() {
            on_event(AgentEvent::Error {
                message:
                    "the reply was cut off at the provider's output-token limit mid-tool-call; \
                          stopping without running the incomplete call"
                        .into(),
            });
            on_event(AgentEvent::TurnDone);
            return Ok(history);
        }

        // No tool calls → the model gave its final reply; the turn is done.
        if calls.is_empty() {
            // Record the reply so a follow-up message sees it in context.
            turns.push(Turn::Assistant {
                text,
                tool_calls: Vec::new(),
            });
            // A token-limit cutoff means the reply above is a fragment — say
            // so instead of presenting it as a finished answer. It's still
            // recorded, so a follow-up "continue" has the fragment in context.
            if truncated {
                on_event(AgentEvent::Error {
                    message: "the reply was cut off at the provider's output-token limit and may be incomplete".into(),
                });
            }
            on_event(AgentEvent::TurnDone);
            return Ok(turns);
        }

        // Record what the model said and requested, then run each call.
        turns.push(Turn::Assistant {
            text: text.clone(),
            tool_calls: calls.clone(),
        });
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
    // The runaway turn ended mid-exchange; drop it so the next request is valid.
    Ok(history)
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
            // A denied call is fed back as an error so the model can adapt.
            let raw_content = if res.denied && res.content.is_empty() {
                "the user declined this tool call".to_string()
            } else {
                res.content
            };
            let content = cap_tool_result_content(&call.name, &raw_content);
            let is_error = res.is_error || res.denied;
            // The row's summary reads the same text the model is given (#385),
            // so the two never tell different stories about one call.
            let summary = srelens_agent::event::summarize_result(&content, is_error);
            on_event(AgentEvent::ToolResult {
                id: call.id.clone(),
                status,
                summary,
            });
            ToolOutcome {
                id: call.id.clone(),
                name: call.name.clone(),
                content,
                is_error,
            }
        }
        Err(e) => {
            let summary = srelens_agent::event::summarize_result(&e.to_string(), true);
            on_event(AgentEvent::ToolResult {
                id: call.id.clone(),
                status: ToolStatus::Error,
                summary,
            });
            ToolOutcome {
                id: call.id.clone(),
                name: call.name.clone(),
                content: e.to_string(),
                is_error: true,
            }
        }
    }
}

/// Maximum tool result bytes passed to the native model in one tool execution.
pub const MAX_AGENT_TOOL_RESULT_BYTES: usize = 16 * 1024; // 16 KB

/// Cap individual tool result content to prevent single massive outputs (e.g. huge pod logs)
/// from blowing out the model context window.
pub fn cap_tool_result_content(tool_name: &str, content: &str) -> String {
    if content.len() > MAX_AGENT_TOOL_RESULT_BYTES {
        let boundary = content
            .char_indices()
            .take_while(|(idx, _)| *idx <= MAX_AGENT_TOOL_RESULT_BYTES)
            .last()
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        let mut truncated = content[..boundary].to_string();
        truncated.push_str(&format!(
            "\n\n[Output truncated at 16KB for `{tool_name}`. Output exceeded limit; specify more focused selectors, namespace, or tail_lines.]"
        ));
        truncated
    } else {
        content.to_string()
    }
}

/// Compact earlier intermediate tool results within the conversation when sending
/// requests to the provider in multi-round execution. The latest tool result is kept
/// in full so the model can inspect current outputs, while earlier round outputs are
/// condensed to conserve context window tokens.
pub fn compact_request_turns_for_round(turns: &[Turn]) -> Vec<Turn> {
    let last_tool_results_idx = turns
        .iter()
        .rposition(|t| matches!(t, Turn::ToolResults(_)));

    turns
        .iter()
        .enumerate()
        .map(|(idx, turn)| match turn {
            Turn::ToolResults(outcomes) => {
                if Some(idx) == last_tool_results_idx {
                    turn.clone()
                } else {
                    let compacted = outcomes
                        .iter()
                        .map(|o| {
                            let mut copy = o.clone();
                            if copy.content.len() > 600 {
                                let snippet: String = copy.content.chars().take(250).collect();
                                copy.content = format!(
                                    "{}... [Output condensed for subsequent round]",
                                    snippet.trim_end()
                                );
                            }
                            copy
                        })
                        .collect();
                    Turn::ToolResults(compacted)
                }
            }
            _ => turn.clone(),
        })
        .collect()
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
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), args.clone()));
            Ok(self.result.clone())
        }
    }

    fn drive(provider: &dyn Provider, invoker: &dyn ToolInvoker, prompt: &str) -> Vec<AgentEvent> {
        drive_from(provider, invoker, Vec::new(), prompt).0
    }

    /// Like `drive`, but seeds prior `history` and also returns the conversation
    /// `run` hands back for the next turn.
    fn drive_from(
        provider: &dyn Provider,
        invoker: &dyn ToolInvoker,
        history: Vec<Turn>,
        prompt: &str,
    ) -> (Vec<AgentEvent>, Vec<Turn>) {
        let events = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let mut on_event = move |e: AgentEvent| sink.lock().unwrap().push(e);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt
            .block_on(run(
                provider,
                invoker,
                history,
                prompt.to_string(),
                &mut on_event,
            ))
            .unwrap();
        drop(on_event);
        let collected = events.lock().unwrap().clone();
        (collected, out)
    }

    #[test]
    fn the_returned_conversation_records_the_user_prompt_and_final_reply() {
        let provider = ScriptedProvider::new(vec![vec![
            StreamItem::Text("all healthy".into()),
            StreamItem::Done(StopReason::EndTurn),
        ]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: String::new(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let (_events, history) = drive_from(&provider, &invoker, Vec::new(), "status?");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], Turn::User("status?".into()));
        assert_eq!(
            history[1],
            Turn::Assistant {
                text: "all healthy".into(),
                tool_calls: Vec::new()
            }
        );
    }

    #[test]
    fn a_max_tokens_cutoff_surfaces_a_truncation_error_but_keeps_the_fragment() {
        let provider = ScriptedProvider::new(vec![vec![
            StreamItem::Text("the pods are".into()),
            StreamItem::Done(StopReason::MaxTokens),
        ]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: String::new(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let (events, history) = drive_from(&provider, &invoker, Vec::new(), "status?");
        // The fragment stays in history so a follow-up "continue" has it…
        assert_eq!(
            history[1],
            Turn::Assistant {
                text: "the pods are".into(),
                tool_calls: Vec::new()
            }
        );
        // …but the user is told it was cut off, not shown a "complete" reply.
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Error { message } if message.contains("cut off"))),
            "expected a truncation error event, got {events:?}"
        );
        assert!(matches!(events.last(), Some(AgentEvent::TurnDone)));
    }

    #[test]
    fn a_truncated_round_with_tool_calls_runs_nothing_and_discards_the_turn() {
        // The cutoff can land mid-arguments (parsers coerce partial JSON to
        // `{}`), so executing the call could act on arguments the model never
        // finished. The whole round is discarded instead.
        let provider = ScriptedProvider::new(vec![vec![
            StreamItem::ToolCall(ToolCall {
                id: "c1".into(),
                name: "k8s_scale".into(),
                arguments: json!({}),
                thought_signature: None,
            }),
            StreamItem::Done(StopReason::MaxTokens),
        ]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: "ok".into(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let prior = vec![Turn::User("earlier".into())];
        let (events, history) = drive_from(&provider, &invoker, prior.clone(), "scale it");
        assert!(
            invoker.calls.lock().unwrap().is_empty(),
            "no tool may run from a truncated round"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Error { message } if message.contains("cut off"))),
            "expected a truncation error event, got {events:?}"
        );
        assert!(matches!(events.last(), Some(AgentEvent::TurnDone)));
        // The failed turn is discarded — the next request starts from the prior history.
        assert_eq!(history, prior);
    }

    #[test]
    fn a_follow_up_turn_carries_prior_history_into_the_provider_request() {
        let provider = ScriptedProvider::new(vec![vec![
            StreamItem::Text("still healthy".into()),
            StreamItem::Done(StopReason::EndTurn),
        ]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: String::new(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let prior = vec![
            Turn::User("what pods are down?".into()),
            Turn::Assistant {
                text: "web-0".into(),
                tool_calls: Vec::new(),
            },
        ];
        let (_events, history) = drive_from(&provider, &invoker, prior.clone(), "and now?");
        // The provider saw the full prior conversation plus the new prompt.
        let seen = provider.seen_turns.lock().unwrap();
        assert_eq!(seen[0].len(), 3);
        assert_eq!(seen[0][0], prior[0]);
        assert_eq!(seen[0][2], Turn::User("and now?".into()));
        // And the returned history grows to include the new exchange.
        assert_eq!(history.len(), 4);
    }

    #[test]
    fn a_failed_turn_is_discarded_from_the_continued_history() {
        let provider = ScriptedProvider::new(vec![vec![StreamItem::Error("Overloaded".into())]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: String::new(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let prior = vec![
            Turn::User("hi".into()),
            Turn::Assistant {
                text: "hello".into(),
                tool_calls: Vec::new(),
            },
        ];
        let (_events, history) = drive_from(&provider, &invoker, prior.clone(), "do a thing");
        // The failed turn (its user message and any partial reply) is dropped,
        // so the next request continues cleanly from the prior history.
        assert_eq!(history, prior);
    }

    #[test]
    fn a_reply_with_no_tool_calls_streams_text_then_turn_done() {
        let provider = ScriptedProvider::new(vec![vec![
            StreamItem::Text("all healthy".into()),
            StreamItem::Done(StopReason::EndTurn),
        ]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: String::new(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "status?");
        assert_eq!(
            events,
            vec![
                AgentEvent::TextDelta {
                    text: "all healthy".into()
                },
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
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            vec![
                StreamItem::Text("scaled to 3".into()),
                StreamItem::Done(StopReason::EndTurn),
            ],
        ]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: "ok".into(),
                is_error: false,
                denied: false,
            },
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
                AgentEvent::ToolResult {
                    id: "c1".into(),
                    status: ToolStatus::Ok,
                    summary: Some("ok".into())
                },
                AgentEvent::TextDelta {
                    text: "scaled to 3".into()
                },
                AgentEvent::TurnDone,
            ]
        );
        // The tool was actually invoked with the model's args.
        assert_eq!(
            invoker.calls.lock().unwrap().as_slice(),
            &[("k8s_scale".into(), json!({ "replicas": 3 }))]
        );
        // The second provider request carried the tool result back.
        let seen = provider.seen_turns.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(matches!(seen[1].last(), Some(Turn::ToolResults(o)) if o[0].content == "ok"));
    }

    #[test]
    fn a_denied_tool_call_reports_denied_and_feeds_that_back() {
        let provider = ScriptedProvider::new(vec![
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            vec![
                StreamItem::Text("ok, leaving it".into()),
                StreamItem::Done(StopReason::EndTurn),
            ],
        ]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: String::new(),
                is_error: false,
                denied: true,
            },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "scale it");
        assert!(events.contains(&AgentEvent::ToolResult {
            id: "c1".into(),
            status: ToolStatus::Denied,
            summary: Some("the user declined this tool call".into())
        }));
        let seen = provider.seen_turns.lock().unwrap();
        assert!(matches!(seen[1].last(), Some(Turn::ToolResults(o)) if o[0].is_error));
    }

    /// An invoker that cannot reach the MCP server at all.
    struct UnreachableInvoker;

    #[async_trait]
    impl ToolInvoker for UnreachableInvoker {
        async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
            Ok(vec![ToolDef {
                name: "k8s_scale".into(),
                description: "scale".into(),
                input_schema: json!({ "type": "object" }),
                read_only: false,
            }])
        }

        async fn call_tool(&self, _name: &str, _args: &Value) -> Result<ToolCallResult, LlmError> {
            Err(LlmError::Http("connection refused".into()))
        }
    }

    /// PR #806 review: a call that never reached the server is an error whose
    /// summary is the transport's own message.
    #[test]
    fn an_unreachable_tool_is_an_error_summarised_by_the_transport_failure() {
        let provider = ScriptedProvider::new(vec![
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            vec![
                StreamItem::Text("could not reach it".into()),
                StreamItem::Done(StopReason::EndTurn),
            ],
        ]);
        let events = drive(&provider, &UnreachableInvoker, "scale it");
        assert!(
            events.contains(&AgentEvent::ToolResult {
                id: "c1".into(),
                status: ToolStatus::Error,
                summary: Some("network error: connection refused".into()),
            }),
            "events: {events:?}"
        );
    }

    #[test]
    fn a_provider_error_surfaces_before_turn_done() {
        let provider = ScriptedProvider::new(vec![vec![StreamItem::Error("Overloaded".into())]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: String::new(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "hi");
        assert_eq!(
            events,
            vec![
                AgentEvent::Error {
                    message: "Overloaded".into()
                },
                AgentEvent::TurnDone
            ]
        );
    }

    #[test]
    fn token_usage_is_accumulated_and_emitted_across_rounds() {
        let provider = ScriptedProvider::new(vec![
            vec![
                StreamItem::Usage(crate::types::TokenUsage {
                    prompt_tokens: 1000,
                    completion_tokens: 50,
                    cached_tokens: 200,
                    total_tokens: 1050,
                }),
                StreamItem::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({ "replicas": 3 }),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            vec![
                StreamItem::Text("done".into()),
                StreamItem::Usage(crate::types::TokenUsage {
                    prompt_tokens: 1200,
                    completion_tokens: 80,
                    cached_tokens: 300,
                    total_tokens: 1280,
                }),
                StreamItem::Done(StopReason::EndTurn),
            ],
        ]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: "ok".into(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "scale it");
        assert!(events.contains(&AgentEvent::Usage {
            prompt_tokens: 1000,
            completion_tokens: 50,
            cached_tokens: 200,
            total_tokens: 1050,
        }));
        assert!(events.contains(&AgentEvent::Usage {
            prompt_tokens: 2200,
            completion_tokens: 130,
            cached_tokens: 500,
            total_tokens: 2330,
        }));
    }

    #[test]
    fn a_round_with_multiple_usage_snapshots_does_not_double_count() {
        let provider = ScriptedProvider::new(vec![vec![
            StreamItem::Text("working...".into()),
            StreamItem::Usage(crate::types::TokenUsage {
                prompt_tokens: 1000,
                completion_tokens: 20,
                cached_tokens: 0,
                total_tokens: 1020,
            }),
            StreamItem::Text("done".into()),
            StreamItem::Usage(crate::types::TokenUsage {
                prompt_tokens: 1000,
                completion_tokens: 50,
                cached_tokens: 0,
                total_tokens: 1050,
            }),
            StreamItem::Done(StopReason::EndTurn),
        ]]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: "ok".into(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let events = drive(&provider, &invoker, "do work");
        let final_usage = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Usage {
                    prompt_tokens,
                    completion_tokens,
                    cached_tokens,
                    total_tokens,
                } => Some((
                    *prompt_tokens,
                    *completion_tokens,
                    *cached_tokens,
                    *total_tokens,
                )),
                _ => None,
            })
            .last()
            .expect("should have emitted usage");

        assert_eq!(
            final_usage,
            (1000, 50, 0, 1050),
            "multiple snapshots in a single round must record the latest snapshot, not sum them"
        );
    }

    #[test]
    fn test_cap_tool_result_content_truncates_large_output_with_hint() {
        let large = "a".repeat(20 * 1024);
        let capped = cap_tool_result_content("k8s_podLogs", &large);
        assert!(capped.len() < large.len());
        assert!(capped.contains("[Output truncated at 16KB for `k8s_podLogs`"));
        assert!(capped.contains("specify more focused selectors"));
    }

    #[test]
    fn test_cap_tool_result_content_preserves_short_output() {
        let short = "{\"status\":\"ok\"}";
        let capped = cap_tool_result_content("k8s_scale", short);
        assert_eq!(capped, short);
    }

    #[test]
    fn test_compact_request_turns_condenses_older_intermediate_results_keeps_latest() {
        let turns = vec![
            Turn::User("diagnose".into()),
            Turn::Assistant {
                text: "".into(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "k8s_listPods".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }],
            },
            Turn::ToolResults(vec![ToolOutcome {
                id: "c1".into(),
                name: "k8s_listPods".into(),
                content: "a".repeat(1500),
                is_error: false,
            }]),
            Turn::Assistant {
                text: "".into(),
                tool_calls: vec![ToolCall {
                    id: "c2".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }],
            },
            Turn::ToolResults(vec![ToolOutcome {
                id: "c2".into(),
                name: "k8s_scale".into(),
                content: "b".repeat(1500),
                is_error: false,
            }]),
        ];

        let compacted = compact_request_turns_for_round(&turns);
        assert_eq!(compacted.len(), 5);

        // Turn at index 2 (older ToolResults) must be condensed
        if let Turn::ToolResults(outcomes) = &compacted[2] {
            assert!(outcomes[0]
                .content
                .contains("... [Output condensed for subsequent round]"));
            assert!(outcomes[0].content.len() < 400);
        } else {
            panic!("Expected Turn::ToolResults at index 2");
        }

        // Turn at index 4 (latest ToolResults) must remain full
        if let Turn::ToolResults(outcomes) = &compacted[4] {
            assert_eq!(outcomes[0].content.len(), 1500);
        } else {
            panic!("Expected Turn::ToolResults at index 4");
        }
    }

    #[test]
    fn test_run_compacts_prior_tool_results_on_request_copy_without_mutating_canonical_turns() {
        let heavy_output_1 = "X".repeat(1000);
        let provider = ScriptedProvider::new(vec![
            // Round 1: calls tool 1
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            // Round 2: calls tool 2
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c2".into(),
                    name: "k8s_scale".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            // Round 3: final answer
            vec![
                StreamItem::Text("done".into()),
                StreamItem::Done(StopReason::EndTurn),
            ],
        ]);

        let invoker = StubInvoker {
            result: ToolCallResult {
                content: heavy_output_1.clone(),
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };

        let (_events, returned_history) = drive_from(&provider, &invoker, Vec::new(), "multi step");

        // Verify provider saw condensed output in round 3
        let seen = provider.seen_turns.lock().unwrap().clone();
        assert_eq!(seen.len(), 3);
        // In round 3 (index 2), the first tool result (index 2) was condensed:
        if let Turn::ToolResults(outcomes) = &seen[2][2] {
            assert!(outcomes[0]
                .content
                .contains("... [Output condensed for subsequent round]"));
        } else {
            panic!("Expected Turn::ToolResults at index 2 of round 3");
        }

        // Canonical returned_history preserves the full content
        if let Turn::ToolResults(outcomes) = &returned_history[2] {
            assert_eq!(outcomes[0].content, heavy_output_1);
        } else {
            panic!("Expected Turn::ToolResults at index 2 of returned history");
        }
    }
}
