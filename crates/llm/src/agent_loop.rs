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

    for _ in 0..MAX_ROUNDS {
        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut stream_error: Option<String> = None;
        let mut truncated = false;

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
            };
            let stream_turns = compact_turns_for_stream(&turns);
            provider
                .stream_turn(&stream_turns, &tools, &mut on_item)
                .await?;
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
            return Ok(distill_history_for_followup(&turns));
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

/// Universal ceiling on a single tool call's result content (in bytes).
/// Prevents any single rogue tool from blowing the LLM context window.
pub const MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;

/// Maximum characters retained for historical tool results from prior rounds.
pub const MAX_HISTORICAL_TOOL_CHARS: usize = 600;

/// Return a slice of `s` containing at most `max_chars` whole characters from the start.
fn safe_char_prefix(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Return a slice of `s` containing at most `max_chars` whole characters from the end.
fn safe_char_suffix(s: &str, max_chars: usize) -> &str {
    let count = s.chars().count();
    if count <= max_chars {
        return s;
    }
    let skip = count - max_chars;
    match s.char_indices().nth(skip) {
        Some((idx, _)) => &s[idx..],
        None => s,
    }
}

/// Compact prior tool results in a conversation before streaming to the provider.
///
/// Only the most recent tool exchange (`Turn::ToolResults`) retains full fidelity so
/// the model can reason on the immediate results. Older tool results from earlier rounds
/// or previous user turns that exceed [`MAX_HISTORICAL_TOOL_CHARS`] are condensed
/// to prevent quadratic token growth across multi-round investigations.
pub fn compact_turns_for_stream(turns: &[Turn]) -> Vec<Turn> {
    const HEAD_KEEP_CHARS: usize = 250;
    const TAIL_KEEP_CHARS: usize = 150;

    let last_tool_results_idx = turns
        .iter()
        .rposition(|t| matches!(t, Turn::ToolResults(_)));

    turns
        .iter()
        .enumerate()
        .map(|(idx, turn)| match turn {
            Turn::ToolResults(outcomes) if Some(idx) != last_tool_results_idx => {
                let compacted_outcomes = outcomes
                    .iter()
                    .map(|o| {
                        if o.content.chars().count() > MAX_HISTORICAL_TOOL_CHARS {
                            let head = safe_char_prefix(&o.content, HEAD_KEEP_CHARS);
                            let tail = safe_char_suffix(&o.content, TAIL_KEEP_CHARS);
                            let skipped = o
                                .content
                                .chars()
                                .count()
                                .saturating_sub(head.chars().count() + tail.chars().count());
                            let content = format!(
                                "{}\n... [{} characters omitted from earlier round tool result] ...\n{}",
                                head.trim_end(),
                                skipped,
                                tail.trim_start()
                            );
                            ToolOutcome {
                                id: o.id.clone(),
                                name: o.name.clone(),
                                content,
                                is_error: o.is_error,
                            }
                        } else {
                            o.clone()
                        }
                    })
                    .collect();
                Turn::ToolResults(compacted_outcomes)
            }
            other => other.clone(),
        })
        .collect()
}

/// Distill completed conversation history so follow-up user turns do not re-send
/// bloated raw tool outputs from prior completed turns.
///
/// Keeps the exact same sequence of `ToolCall` and `ToolOutcome` correlation IDs
/// (preserving provider API validity for Anthropic, OpenAI, and Gemini), but distills
/// large historical tool outcome payloads to concise summaries.
pub fn distill_history_for_followup(turns: &[Turn]) -> Vec<Turn> {
    const MAX_DISTILLED_CHARS: usize = 300;
    turns
        .iter()
        .map(|turn| match turn {
            Turn::ToolResults(outcomes) => {
                let distilled = outcomes
                    .iter()
                    .map(|o| {
                        if o.content.chars().count() > MAX_DISTILLED_CHARS {
                            let status = if o.is_error { "error" } else { "ok" };
                            let excerpt: String = o.content.chars().take(120).collect();
                            let content = format!(
                                "[Tool {} ({}) executed in prior turn: {}...]",
                                o.name,
                                status,
                                excerpt.trim()
                            );
                            ToolOutcome {
                                id: o.id.clone(),
                                name: o.name.clone(),
                                content,
                                is_error: o.is_error,
                            }
                        } else {
                            o.clone()
                        }
                    })
                    .collect();
                Turn::ToolResults(distilled)
            }
            other => other.clone(),
        })
        .collect()
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
            on_event(AgentEvent::ToolResult {
                id: call.id.clone(),
                status,
            });
            // A denied call is fed back as an error so the model can adapt.
            let content = if res.denied && res.content.is_empty() {
                "the user declined this tool call".to_string()
            } else if res.content.len() > MAX_TOOL_RESULT_BYTES {
                let head = safe_char_prefix(&res.content, 8 * 1024);
                let tail = safe_char_suffix(&res.content, 4 * 1024);
                format!(
                    "{}\n\n[... tool output exceeded 16KB ({} bytes total) and was truncated for token efficiency. Refine your query or inspect specific fields ...]\n\n{}",
                    head,
                    res.content.len(),
                    tail
                )
            } else {
                res.content
            };
            ToolOutcome {
                id: call.id.clone(),
                name: call.name.clone(),
                content,
                is_error: res.is_error || res.denied,
            }
        }
        Err(e) => {
            on_event(AgentEvent::ToolResult {
                id: call.id.clone(),
                status: ToolStatus::Error,
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
                    status: ToolStatus::Ok
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
            status: ToolStatus::Denied
        }));
        let seen = provider.seen_turns.lock().unwrap();
        assert!(matches!(seen[1].last(), Some(Turn::ToolResults(o)) if o[0].is_error));
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
    fn universal_tool_result_ceiling_truncates_giant_outputs() {
        let giant_payload = "X".repeat(30_000);
        let provider = ScriptedProvider::new(vec![
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "k8s_giant".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            vec![
                StreamItem::Text("done".into()),
                StreamItem::Done(StopReason::EndTurn),
            ],
        ]);
        let invoker = StubInvoker {
            result: ToolCallResult {
                content: giant_payload,
                is_error: false,
                denied: false,
            },
            calls: Mutex::new(Vec::new()),
        };
        let _ = drive(&provider, &invoker, "get logs");
        let seen = provider.seen_turns.lock().unwrap();
        assert_eq!(seen.len(), 2);
        if let Some(Turn::ToolResults(outcomes)) = seen[1].last() {
            assert!(outcomes[0].content.contains("exceeded 16KB"));
            assert!(outcomes[0].content.len() < 20_000);
        } else {
            panic!("expected ToolResults in turn 2");
        }
    }

    #[test]
    fn multi_round_compacts_prior_tool_results_while_preserving_latest() {
        let big_tool_1 = "A".repeat(1200);
        let big_tool_2 = "B".repeat(1000);

        let provider = ScriptedProvider::new(vec![
            // Round 0 asks for tool 1
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c1".into(),
                    name: "tool_1".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            // Round 1 asks for tool 2
            vec![
                StreamItem::ToolCall(ToolCall {
                    id: "c2".into(),
                    name: "tool_2".into(),
                    arguments: json!({}),
                    thought_signature: None,
                }),
                StreamItem::Done(StopReason::ToolUse),
            ],
            // Round 2 finishes
            vec![
                StreamItem::Text("diagnosis complete".into()),
                StreamItem::Done(StopReason::EndTurn),
            ],
        ]);

        struct DynamicInvoker {
            count: std::sync::atomic::AtomicUsize,
            out1: String,
            out2: String,
        }
        #[async_trait]
        impl ToolInvoker for DynamicInvoker {
            async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
                Ok(vec![])
            }
            async fn call_tool(
                &self,
                _name: &str,
                _args: &Value,
            ) -> Result<ToolCallResult, LlmError> {
                let n = self
                    .count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if n == 0 {
                    Ok(ToolCallResult {
                        content: self.out1.clone(),
                        is_error: false,
                        denied: false,
                    })
                } else {
                    Ok(ToolCallResult {
                        content: self.out2.clone(),
                        is_error: false,
                        denied: false,
                    })
                }
            }
        }

        let invoker = DynamicInvoker {
            count: std::sync::atomic::AtomicUsize::new(0),
            out1: big_tool_1,
            out2: big_tool_2,
        };

        let (_events, final_history) = drive_from(&provider, &invoker, Vec::new(), "investigate");
        let seen = provider.seen_turns.lock().unwrap();
        // 3 requests to provider
        assert_eq!(seen.len(), 3);

        // In request 1 (Round 1), tool 1 was the latest, so it was sent in full:
        assert!(matches!(&seen[1][2], Turn::ToolResults(o) if o[0].content.len() == 1200));

        // In request 2 (Round 2), tool 1 is older so it is COMPACTED, while tool 2 (latest) is FULL:
        assert!(
            matches!(&seen[2][2], Turn::ToolResults(o) if o[0].content.contains("omitted from earlier round tool result"))
        );
        assert!(matches!(&seen[2][4], Turn::ToolResults(o) if o[0].content.len() == 1000));

        // And in final_history returned for future turns, tool results are distilled:
        assert!(
            matches!(&final_history[2], Turn::ToolResults(o) if o[0].content.contains("executed in prior turn"))
        );
        assert!(
            matches!(&final_history[4], Turn::ToolResults(o) if o[0].content.contains("executed in prior turn"))
        );
    }
}
