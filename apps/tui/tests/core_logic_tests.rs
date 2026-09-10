//! Behavioural tests for the TUI's pure core: command resolution, deep links,
//! AI settings, skills, theme, the event/sink plumbing, and the native agent
//! bridge driven against an in-process fake OpenAI-compatible endpoint on
//! loopback (no real provider, no agent subprocess).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ratatui::style::{Color, Modifier, Style};
use serde_json::{json, Value};
use srelens_kube::client_cache::ClientCache;
use srelens_llm::types::{ProviderKind, Turn};
use srelens_llm::{ProviderConfig, ToolInvoker};
use srelens_streams::EventSink;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use srelens_tui::agent::{
    build_mcp_server, run_boxed_cursor_turn, run_native_agent_turn, McpToolInvoker,
};
use srelens_tui::ai_config::{
    default_base_url_for_provider, default_model_for_provider, env_var_for_provider,
    provider_display_name, provider_slug, AiProvider, AiSettings, ALL_PROVIDERS,
};
use srelens_tui::ai_skills::{
    expand_slash_command, load_user_skills_dir, match_slash_commands, parse_caveman_command,
    CavemanCommandAction, CavemanLevel, BUILTIN_PLAYBOOKS,
};
use srelens_tui::commands::{
    command_suggestions, command_suggestions_with_crds, resolve_command, resolve_command_with_crds,
    CommandTarget, CrdMeta, DynamicCommandDef, ResourceKind, COMMAND_REGISTRY,
};
use srelens_tui::deep_link::DeepLink;
use srelens_tui::event::{AppEvent, EventHandler};
use srelens_tui::sink::TuiSink;
use srelens_tui::theme::{status_style, Theme};
use srelens_tui::tui_config::{CommandPopupDensity, TuiConfig};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Every `ActionResult` the agent emitted, as `(title, result)`, in order.
fn action_results(rx: &mut UnboundedReceiver<AppEvent>) -> Vec<(String, Result<String, String>)> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let AppEvent::ActionResult { title, result } = ev {
            out.push((title, result));
        }
    }
    out
}

fn invoker() -> Arc<McpToolInvoker> {
    let cache = ClientCache::new(PathBuf::from("/nonexistent"));
    Arc::new(McpToolInvoker::new(build_mcp_server(cache, vec![])))
}

fn crd(plural: &str, singular: &str, kind: &str, group: &str, short_names: &[&str]) -> CrdMeta {
    CrdMeta {
        crd_name: format!("{}.{}", plural, group),
        group: group.to_string(),
        version: "v1".to_string(),
        kind: kind.to_string(),
        plural: plural.to_string(),
        singular: singular.to_string(),
        namespaced: true,
        short_names: short_names.iter().map(|s| s.to_string()).collect(),
        printer_columns: vec![],
    }
}

fn cilium_pool() -> CrdMeta {
    crd(
        "ciliumloadbalancerippools",
        "ciliumloadbalancerippool",
        "CiliumLoadBalancerIPPool",
        "cilium.io",
        &["ippool"],
    )
}

// ---------------------------------------------------------------------------
// A fake OpenAI-compatible endpoint on loopback
// ---------------------------------------------------------------------------

/// One scripted HTTP reply: status code and raw body (SSE `data:` lines).
struct Reply {
    status: u16,
    body: String,
}

fn sse(events: &[&str]) -> String {
    let mut s = String::new();
    for e in events {
        s.push_str("data: ");
        s.push_str(e);
        s.push_str("\n\n");
    }
    s.push_str("data: [DONE]\n\n");
    s
}

fn text_reply(chunks: &[&str]) -> Reply {
    let mut events: Vec<String> = chunks
        .iter()
        .map(|c| json!({"choices":[{"delta":{"content":c}}]}).to_string())
        .collect();
    events.push(json!({"choices":[{"delta":{},"finish_reason":"stop"}]}).to_string());
    let refs: Vec<&str> = events.iter().map(String::as_str).collect();
    Reply {
        status: 200,
        body: sse(&refs),
    }
}

/// A reply that requests the given tool calls: `(id, name, raw arguments string)`.
fn tool_call_reply(calls: &[(&str, &str, &str)]) -> Reply {
    let deltas: Vec<Value> = calls
        .iter()
        .enumerate()
        .map(|(i, (id, name, args))| {
            json!({"index": i, "id": id, "function": {"name": name, "arguments": args}})
        })
        .collect();
    let events = vec![
        json!({"choices":[{"delta":{"tool_calls": deltas}}]}).to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string(),
    ];
    let refs: Vec<&str> = events.iter().map(String::as_str).collect();
    Reply {
        status: 200,
        body: sse(&refs),
    }
}

struct FakeLlm {
    base_url: String,
    /// Every request body the provider sent, parsed, in order.
    requests: Arc<Mutex<Vec<Value>>>,
}

/// Read one HTTP request off the socket and return its parsed JSON body.
async fn read_request(sock: &mut tokio::net::TcpStream) -> Value {
    let mut buf = Vec::new();
    let header_end = loop {
        let mut chunk = [0u8; 4096];
        let n = sock.read(&mut chunk).await.expect("read request");
        if n == 0 {
            break None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(pos + 4);
        }
    };
    let Some(header_end) = header_end else {
        return Value::Null;
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length: usize = headers
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    while buf.len() < header_end + content_length {
        let mut chunk = [0u8; 4096];
        let n = sock.read(&mut chunk).await.expect("read body");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    serde_json::from_slice(&buf[header_end..header_end + content_length]).unwrap_or(Value::Null)
}

/// Serve the scripted replies, one per connection, in order. A request past
/// the end of the script gets a 500 so a runaway loop fails loudly.
async fn fake_llm(replies: Vec<Reply>) -> FakeLlm {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let base_url = format!("http://{}/v1", listener.local_addr().expect("local addr"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    tokio::spawn(async move {
        let mut replies = replies.into_iter();
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let body = read_request(&mut sock).await;
            seen.lock().unwrap().push(body);
            let reply = replies.next().unwrap_or(Reply {
                status: 500,
                body: "script exhausted".into(),
            });
            let response = format!(
                "HTTP/1.1 {} X\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status,
                reply.body.len(),
                reply.body
            );
            let _ = sock.write_all(response.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    FakeLlm { base_url, requests }
}

/// `HttpProvider::new` builds a `reqwest::Client` with `rustls-no-provider`,
/// which panics ("No rustls crypto provider is configured") unless a process
/// default was installed earlier. In the running TUI that happens as a side
/// effect of the first kube `Client` build; a fresh test process has to do the
/// same, so mirror production by building one (no network involved).
fn ensure_tls_provider() {
    let config = srelens_kube::kube::Config::new("https://127.0.0.1:6443".parse().expect("uri"));
    srelens_kube::kube::Client::try_from(config).expect("kube client builds");
}

fn provider_config(base_url: &str) -> ProviderConfig {
    ensure_tls_provider();
    ProviderConfig {
        kind: ProviderKind::OpenAiCompatible,
        api_key: "test-key".into(),
        base_url: base_url.to_string(),
        model: "fake-model".into(),
        max_tokens: 256,
    }
}

/// Run one native turn against `llm` and return the emitted action results
/// plus the history the turn left behind.
async fn native_turn(
    llm: &FakeLlm,
    prior: Vec<Turn>,
    prompt: &str,
) -> (Vec<(String, Result<String, String>)>, Vec<Turn>) {
    let (tx, mut rx) = unbounded_channel();
    let history = Arc::new(tokio::sync::Mutex::new(prior));
    run_native_agent_turn(
        provider_config(&llm.base_url),
        invoker(),
        history.clone(),
        prompt.to_string(),
        "kind-dev".into(),
        "payments".into(),
        tx,
        30,
    )
    .await;
    let events = action_results(&mut rx);
    let turns = history.lock().await.clone();
    (events, turns)
}

fn usage_fields(payload: &str) -> Vec<u64> {
    payload
        .split('|')
        .map(|f| f.parse().expect("numeric usage field"))
        .collect()
}

// ---------------------------------------------------------------------------
// agent.rs: McpToolInvoker
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_aliases_are_provider_safe_and_stable_across_repeated_listings() {
    let inv = invoker();
    let first: Vec<String> = inv
        .list_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    let second: Vec<String> = inv
        .list_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();

    assert!(
        first.iter().all(|n| !n.contains('.')),
        "dots must be rewritten: {:?}",
        first
    );
    assert!(first.contains(&"k8s_listPods".to_string()));
    assert!(first.contains(&"ping".to_string()));
    // Re-listing maps each id to the alias it already owns instead of
    // growing a trailing underscore.
    assert_eq!(first, second);
}

#[tokio::test]
async fn list_tools_carries_description_schema_and_read_only_hint() {
    let inv = invoker();
    let tools = inv.list_tools().await.unwrap();
    let ping = tools.iter().find(|t| t.name == "ping").expect("ping tool");
    assert!(ping.read_only);
    assert!(ping.description.contains("health check"));
    assert!(ping.input_schema.is_object());
    let delete = tools
        .iter()
        .find(|t| t.name == "k8s_deleteContext")
        .expect("deleteContext tool");
    assert!(!delete.read_only);
}

#[tokio::test]
async fn calling_a_read_only_tool_returns_its_output_as_a_clean_result() {
    let inv = invoker();
    let res = inv
        .call_tool("ping", &json!({"hello": "world"}))
        .await
        .unwrap();
    assert!(!res.is_error);
    assert!(!res.denied);
    let body: Value = serde_json::from_str(&res.content).expect("ping echoes JSON");
    assert_eq!(body["pong"]["hello"], "world");
}

#[tokio::test]
async fn a_destructive_tool_called_by_alias_is_denied_by_the_policy() {
    let inv = invoker();
    inv.list_tools().await.unwrap();
    let res = inv
        .call_tool("k8s_deleteContext", &json!({"context": "prod"}))
        .await
        .unwrap();
    assert!(
        res.denied,
        "FlagGated(false, ..) must refuse destructive calls: {:?}",
        res
    );
    assert!(res.is_error);
    assert!(
        res.content.contains("k8s.deleteContext"),
        "reason names the real tool: {}",
        res.content
    );
}

#[tokio::test]
async fn an_unknown_tool_name_is_a_tool_error_not_a_transport_failure() {
    let inv = invoker();
    let res = inv
        .call_tool("definitely_not_a_tool", &json!({}))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(!res.denied);
    assert!(!res.content.is_empty());
}

#[tokio::test]
async fn a_tool_that_needs_a_missing_cluster_reports_an_error_result() {
    let inv = invoker();
    inv.list_tools().await.unwrap();
    let res = inv
        .call_tool(
            "k8s_listPods",
            &json!({"context": "nowhere", "namespace": "default"}),
        )
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(!res.denied);
    assert!(!res.content.is_empty());
}

// ---------------------------------------------------------------------------
// agent.rs: run_native_agent_turn against the fake endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_native_turn_streams_text_thinking_usage_and_done_in_order() {
    let events = vec![
        json!({"choices":[{"delta":{"reasoning_content":"let me think"}}]}).to_string(),
        json!({"choices":[{"delta":{"content":"Hel"}}]}).to_string(),
        json!({"choices":[{"delta":{"content":"lo"}}]}).to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}]}).to_string(),
    ];
    let refs: Vec<&str> = events.iter().map(String::as_str).collect();
    let llm = fake_llm(vec![Reply {
        status: 200,
        body: sse(&refs),
    }])
    .await;

    let (results, turns) = native_turn(&llm, vec![], "how are the pods?").await;

    assert_eq!(
        results[0],
        ("ai_status:kind-dev".into(), Ok("let me think".into()))
    );
    assert_eq!(results[1], ("ai_chunk:kind-dev".into(), Ok("Hel".into())));
    assert_eq!(results[2], ("ai_chunk:kind-dev".into(), Ok("lo".into())));
    assert_eq!(results[3].0, "ai_usage:kind-dev");
    let usage = usage_fields(results[3].1.as_ref().unwrap());
    let prompt_est = ("how are the pods?".len() + 200) / 4;
    assert_eq!(usage[0], prompt_est as u64);
    assert_eq!(usage[1], 1, "5 output chars / 4");
    assert_eq!(usage[2], 0);
    assert_eq!(usage[3], prompt_est as u64 + 1);
    assert_eq!(results[4], ("ai_done:kind-dev".into(), Ok(String::new())));
    assert_eq!(results.len(), 5);

    // The reply is recorded so a follow-up sees it, behind the enriched prompt.
    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns[0],
        Turn::User("[Active Kubernetes Context: \"kind-dev\", Namespace: \"payments\"]\n\nhow are the pods?".into())
    );
    assert_eq!(
        turns[1],
        Turn::Assistant {
            text: "Hello".into(),
            tool_calls: vec![]
        }
    );

    // The provider saw the enriched prompt and the tool catalogue.
    let requests = llm.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let messages = requests[0]["messages"].as_array().unwrap();
    let user = messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("user message");
    assert!(user["content"]
        .as_str()
        .unwrap()
        .starts_with("[Active Kubernetes Context: \"kind-dev\""));
    assert!(requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["function"]["name"] == "ping"));
}

#[tokio::test]
async fn a_native_turn_runs_a_tool_call_and_reports_start_and_completion() {
    let llm = fake_llm(vec![
        tool_call_reply(&[("call_1", "ping", "{\"x\":1}")]),
        text_reply(&["pong ok"]),
    ])
    .await;

    let (results, turns) = native_turn(&llm, vec![], "ping it").await;

    let titles: Vec<&str> = results.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "ai_status:kind-dev",
            "ai_tool_start:kind-dev",
            "ai_tool_done:kind-dev",
            "ai_chunk:kind-dev",
            "ai_usage:kind-dev",
            "ai_done:kind-dev",
        ]
    );
    assert_eq!(results[0].1, Ok("Executing ping...".into()));
    assert_eq!(results[1].1, Ok("call_1|ping|{\"x\":1}".into()));
    assert_eq!(results[2].1, Ok("call_1|ok".into()));
    assert_eq!(results[3].1, Ok("pong ok".into()));

    // User, assistant(tool call), tool results, assistant(final).
    assert_eq!(turns.len(), 4);
    assert!(matches!(&turns[1], Turn::Assistant { tool_calls, .. } if tool_calls.len() == 1));
    assert!(matches!(&turns[2], Turn::ToolResults(r) if r.len() == 1 && !r[0].is_error));
    assert_eq!(
        turns[3],
        Turn::Assistant {
            text: "pong ok".into(),
            tool_calls: vec![]
        }
    );

    // The second request fed the tool output back to the model.
    let requests = llm.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let tool_msg = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("tool result message");
    assert!(tool_msg["content"].as_str().unwrap().contains("pong"));
}

#[tokio::test]
async fn tool_call_previews_cover_object_string_and_null_arguments_and_every_status() {
    let llm = fake_llm(vec![
        tool_call_reply(&[
            ("c_obj", "no_such_tool", "{\"a\":1}"),
            ("c_str", "k8s_deleteContext", "\"just text\""),
            ("c_null", "ping", "null"),
        ]),
        text_reply(&["done"]),
    ])
    .await;

    let (results, _) = native_turn(&llm, vec![], "mixed").await;
    let starts: Vec<&str> = results
        .iter()
        .filter(|(t, _)| t == "ai_tool_start:kind-dev")
        .map(|(_, r)| r.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(
        starts,
        vec![
            "c_obj|no_such_tool|{\"a\":1}",
            "c_str|k8s_deleteContext|just text",
            "c_null|ping|"
        ]
    );

    let dones: Vec<&str> = results
        .iter()
        .filter(|(t, _)| t == "ai_tool_done:kind-dev")
        .map(|(_, r)| r.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(dones, vec!["c_obj|error", "c_str|denied", "c_null|ok"]);

    let statuses: Vec<&str> = results
        .iter()
        .filter(|(t, _)| t == "ai_status:kind-dev")
        .map(|(_, r)| r.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(
        statuses,
        vec![
            "Executing no_such_tool...",
            "Executing k8s_deleteContext...",
            "Executing ping..."
        ]
    );
}

#[tokio::test]
async fn a_provider_error_item_is_shown_as_an_error_chunk_and_history_is_kept() {
    let err = json!({"error": {"message": "quota exceeded"}}).to_string();
    let llm = fake_llm(vec![Reply {
        status: 200,
        body: sse(&[err.as_str()]),
    }])
    .await;
    let prior = vec![
        Turn::User("earlier".into()),
        Turn::Assistant {
            text: "ok".into(),
            tool_calls: vec![],
        },
    ];

    let (results, turns) = native_turn(&llm, prior.clone(), "again").await;

    assert_eq!(
        results[0],
        (
            "ai_chunk:kind-dev".into(),
            Ok("\n[Error: quota exceeded]".into())
        )
    );
    assert_eq!(results[1].0, "ai_usage:kind-dev");
    assert_eq!(results[2], ("ai_done:kind-dev".into(), Ok(String::new())));
    assert_eq!(results.len(), 3);
    // The failed turn is discarded: the next message continues from `prior`.
    assert_eq!(turns, prior);
}

#[tokio::test]
async fn an_http_failure_from_the_provider_is_reported_as_an_agent_error() {
    let llm = fake_llm(vec![Reply {
        status: 401,
        body: json!({"error": {"message": "bad key"}}).to_string(),
    }])
    .await;

    let (results, turns) = native_turn(&llm, vec![], "hi").await;

    assert_eq!(results[0].0, "ai_usage:kind-dev");
    assert_eq!(results[1].0, "ai_chunk:kind-dev");
    let err = results[1].1.as_ref().unwrap_err();
    assert!(err.starts_with("AI Agent Error: "), "{err}");
    assert!(err.contains("bad key"), "{err}");
    assert_eq!(results[2], ("ai_done:kind-dev".into(), Ok(String::new())));
    assert!(turns.is_empty(), "a failed turn leaves history untouched");
}

#[tokio::test]
async fn a_stream_that_ends_without_a_terminal_marker_is_an_agent_error() {
    let body = format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":"partial"}}]})
    );
    let llm = fake_llm(vec![Reply { status: 200, body }]).await;

    let (results, turns) = native_turn(&llm, vec![], "hi").await;

    assert_eq!(
        results[0],
        ("ai_chunk:kind-dev".into(), Ok("partial".into()))
    );
    let err = results[2].1.as_ref().unwrap_err();
    assert!(err.contains("ended before signaling completion"), "{err}");
    assert!(turns.is_empty());
}

#[tokio::test]
async fn history_is_capped_at_forty_turns_after_a_successful_turn() {
    let llm = fake_llm(vec![text_reply(&["fresh reply"])]).await;
    let prior: Vec<Turn> = (0..44)
        .map(|i| {
            if i % 2 == 0 {
                Turn::User(format!("u{i}"))
            } else {
                Turn::Assistant {
                    text: format!("a{i}"),
                    tool_calls: vec![],
                }
            }
        })
        .collect();

    let (_, turns) = native_turn(&llm, prior, "latest").await;

    assert_eq!(turns.len(), 40);
    // 44 + 2 new = 46; the oldest six were dropped, so the window starts at u6.
    assert_eq!(turns[0], Turn::User("u6".into()));
    assert_eq!(
        turns[39],
        Turn::Assistant {
            text: "fresh reply".into(),
            tool_calls: vec![]
        }
    );
}

#[tokio::test]
async fn a_native_turn_that_cannot_reach_the_provider_still_finishes_cleanly() {
    // Bind then drop, so the port is closed and the connection is refused.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    drop(listener);
    let llm = FakeLlm {
        base_url,
        requests: Arc::new(Mutex::new(Vec::new())),
    };

    let (results, _) = native_turn(&llm, vec![], "hi").await;

    let titles: Vec<&str> = results.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        vec!["ai_usage:kind-dev", "ai_chunk:kind-dev", "ai_done:kind-dev"]
    );
    assert!(results[1]
        .1
        .as_ref()
        .unwrap_err()
        .starts_with("AI Agent Error: network error"));
}

// ---------------------------------------------------------------------------
// agent.rs: run_boxed_cursor_turn without a cursor-agent binary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_boxed_cursor_turn_reports_a_missing_binary_and_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir
        .path()
        .join("no-such-cursor-agent")
        .to_string_lossy()
        .to_string();
    let (tx, mut rx) = unbounded_channel();
    let cache = ClientCache::new(PathBuf::from("/nonexistent"));

    run_boxed_cursor_turn(
        missing,
        "gpt-x".into(),
        // Deliberately None. `run_boxed_cursor_turn` calls
        // `std::env::set_var("CURSOR_API_KEY", key)` (apps/tui/src/agent.rs:444)
        // as well as putting the key on the child, so passing a key here would
        // leak it into the environment of every sibling test in this binary.
        // The key is not what this test is about.
        None,
        "what is wrong?".into(),
        "kind-dev".into(),
        "default".into(),
        cache,
        vec![],
        tx,
        30,
    )
    .await;

    let results = action_results(&mut rx);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "ai_chunk:kind-dev");
    let err = results[0].1.as_ref().unwrap_err();
    assert!(err.starts_with("Failed to launch cursor-agent: "), "{err}");
    assert_eq!(results[1], ("ai_done:kind-dev".into(), Ok(String::new())));
}

// ---------------------------------------------------------------------------
// event.rs
// ---------------------------------------------------------------------------

async fn next_non_tick(handler: &mut EventHandler) -> AppEvent {
    loop {
        match handler.recv().await.expect("channel open") {
            AppEvent::Tick => continue,
            other => return other,
        }
    }
}

#[tokio::test]
async fn events_sent_on_the_handler_channel_arrive_in_order() {
    let mut handler = EventHandler::new(Duration::from_secs(3600));
    handler.tx.send(AppEvent::Paste("pasted".into())).unwrap();
    handler.tx.send(AppEvent::Resize(80, 24)).unwrap();

    assert!(matches!(next_non_tick(&mut handler).await, AppEvent::Paste(s) if s == "pasted"));
    assert!(matches!(
        next_non_tick(&mut handler).await,
        AppEvent::Resize(80, 24)
    ));
}

#[tokio::test]
async fn try_recv_hands_back_a_queued_event_and_pausing_does_not_close_the_channel() {
    let mut handler = EventHandler::new(Duration::from_secs(3600));
    handler.pause();
    handler
        .tx
        .send(AppEvent::StreamEvent {
            channel: "pods".into(),
            payload: json!({"n": 1}),
        })
        .unwrap();

    let mut found = None;
    while let Ok(ev) = handler.try_recv() {
        if let AppEvent::StreamEvent { channel, payload } = ev {
            found = Some((channel, payload));
        }
    }
    assert_eq!(found, Some(("pods".to_string(), json!({"n": 1}))));

    handler.resume();
    handler
        .tx
        .send(AppEvent::Paste("after resume".into()))
        .unwrap();
    assert!(matches!(next_non_tick(&mut handler).await, AppEvent::Paste(s) if s == "after resume"));
}

#[test]
fn app_events_debug_render_their_variant_and_payload() {
    let ev = AppEvent::ActionResult {
        title: "ai_done:ctx".into(),
        result: Err("boom".into()),
    };
    let dbg = format!("{ev:?}");
    assert!(dbg.contains("ActionResult"));
    assert!(dbg.contains("ai_done:ctx"));
    assert!(dbg.contains("boom"));
}

// ---------------------------------------------------------------------------
// sink.rs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_tui_sink_forwards_stream_events_onto_the_app_channel() {
    let (tx, mut rx) = unbounded_channel();
    let sink = TuiSink::arc(tx);
    sink.emit("pods", json!({"name": "api-1"}));
    sink.emit("events", json!([1, 2]));

    let first = rx.try_recv().unwrap();
    assert!(
        matches!(first, AppEvent::StreamEvent { ref channel, ref payload } if channel == "pods" && payload["name"] == "api-1")
    );
    let second = rx.try_recv().unwrap();
    assert!(
        matches!(second, AppEvent::StreamEvent { ref channel, ref payload } if channel == "events" && payload == &json!([1, 2]))
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn the_tui_sink_ignores_a_closed_receiver() {
    let (tx, rx) = unbounded_channel::<AppEvent>();
    drop(rx);
    let sink = TuiSink::new(tx);
    // Must not panic: the app may be shutting down while a watch still emits.
    sink.emit("pods", json!({}));
}

// ---------------------------------------------------------------------------
// theme.rs
// ---------------------------------------------------------------------------

fn bold(style: Style) -> bool {
    style.add_modifier.contains(Modifier::BOLD)
}

#[test]
fn every_theme_style_uses_its_palette_colour() {
    assert_eq!(Theme::header().fg, Some(Theme::CYAN));
    assert!(bold(Theme::header()));
    assert_eq!(Theme::header_label().fg, Some(Theme::DIM));
    assert!(!bold(Theme::header_label()));
    assert_eq!(Theme::header_val().fg, Some(Theme::FG));
    assert_eq!(Theme::title().fg, Some(Theme::ACCENT));
    assert_eq!(Theme::table_header().fg, Some(Theme::CYAN));
    assert_eq!(Theme::selected_row().bg, Some(Theme::SEL_BG));
    assert_eq!(Theme::selected_row().fg, Some(Theme::SEL_FG));
    assert_eq!(Theme::marked_row().bg, Some(Color::Rgb(60, 40, 90)));
    assert_eq!(Theme::marked_row().fg, Some(Color::Yellow));
    assert_eq!(Theme::status_ok().fg, Some(Theme::GREEN));
    assert_eq!(Theme::status_warn().fg, Some(Theme::YELLOW));
    assert_eq!(Theme::status_error().fg, Some(Theme::RED));
    assert_eq!(Theme::status_dim().fg, Some(Theme::DIM));
    assert_eq!(Theme::key_hint_key().fg, Some(Theme::CYAN));
    assert_eq!(Theme::key_hint_desc().fg, Some(Theme::DIM));
    assert_eq!(Theme::prompt().fg, Some(Theme::ACCENT));
    assert!(bold(Theme::prompt()));
    let badge = Theme::badge(Theme::RED, Theme::SEL_FG);
    assert_eq!(badge.bg, Some(Theme::RED));
    assert_eq!(badge.fg, Some(Theme::SEL_FG));
    assert!(bold(badge));
    assert_eq!(Theme::BORDER_FOCUS, Theme::ACCENT);
    assert_eq!(Theme::BG, Color::Reset);
}

#[test]
fn context_colour_reflects_the_environment_named_in_the_context() {
    let prod = Color::Rgb(255, 110, 110);
    let staging = Color::Rgb(255, 200, 80);
    let local = Color::Rgb(80, 220, 140);
    let other = Color::Rgb(100, 200, 255);

    assert_eq!(Theme::context_color("eu-PROD-1", false), prod);
    assert_eq!(Theme::context_color("prd-us", false), prod);
    assert_eq!(
        Theme::context_color("live-cluster", true),
        prod,
        "prod outranks the local flag"
    );
    assert_eq!(Theme::context_color("stage-eu", false), staging);
    assert_eq!(Theme::context_color("stg", false), staging);
    assert_eq!(Theme::context_color("UAT", false), staging);
    assert_eq!(Theme::context_color("qa-2", false), staging);
    assert_eq!(Theme::context_color("kind-dev", false), local);
    assert_eq!(Theme::context_color("minikube", false), local);
    assert_eq!(Theme::context_color("k3d-x", false), local);
    assert_eq!(
        Theme::context_color("something", true),
        local,
        "local flag alone is enough"
    );
    assert_eq!(Theme::context_color("harvester-amd", false), other);
}

#[test]
fn status_style_maps_every_status_family_to_its_colour() {
    for s in [
        "CrashLoopBackOff",
        "Error",
        "Failed",
        "NotReady",
        "Unknown",
        "ImagePullBackOff",
        "Degraded",
        "false",
    ] {
        assert_eq!(status_style(s), Theme::status_error(), "{s}");
    }
    for s in ["Scaled down", "Not scheduled", "Suspended"] {
        assert_eq!(status_style(s), Style::default().fg(Theme::DIM), "{s}");
    }
    for s in ["Pending", "ContainerCreating", "Terminating", "Warning"] {
        assert_eq!(status_style(s), Theme::status_warn(), "{s}");
    }
    for s in [
        "Running",
        "Active",
        "Ready",
        "Completed",
        "Succeeded",
        "Scheduled",
        "true",
        "SecretSynced",
    ] {
        assert_eq!(status_style(s), Theme::status_ok(), "{s}");
    }
    assert_eq!(status_style("Bound"), Style::default().fg(Theme::FG));
    assert_eq!(status_style(""), Style::default().fg(Theme::FG));
}

// ---------------------------------------------------------------------------
// ai_skills.rs
// ---------------------------------------------------------------------------

#[test]
fn a_skill_placeholder_is_its_target_kind_or_blank() {
    let crash = BUILTIN_PLAYBOOKS
        .iter()
        .find(|s| s.command == "crashloop")
        .unwrap();
    assert_eq!(crash.target_placeholder(), "Pod");
    let clear = BUILTIN_PLAYBOOKS
        .iter()
        .find(|s| s.command == "clear")
        .unwrap();
    assert_eq!(clear.target_placeholder(), "");
}

#[test]
fn caveman_levels_round_trip_through_their_display_names() {
    for level in [
        CavemanLevel::Lite,
        CavemanLevel::Full,
        CavemanLevel::Ultra,
        CavemanLevel::WenyanLite,
        CavemanLevel::WenyanFull,
        CavemanLevel::WenyanUltra,
    ] {
        assert_eq!(CavemanLevel::parse(level.display_name()), Some(level));
    }
    assert_eq!(CavemanLevel::WenyanLite.display_name(), "wenyan-lite");
    assert_eq!(CavemanLevel::WenyanUltra.display_name(), "wenyan-ultra");
    assert_eq!(
        CavemanLevel::parse("  WENYAN_ULTRA "),
        Some(CavemanLevel::WenyanUltra)
    );
    assert_eq!(
        CavemanLevel::parse("wenyanlite"),
        Some(CavemanLevel::WenyanLite)
    );
    assert_eq!(
        CavemanLevel::parse("wenyanfull"),
        Some(CavemanLevel::WenyanFull)
    );
}

#[test]
fn caveman_command_keywords_set_each_level_and_keep_the_trailing_question() {
    let cases = [
        ("full", CavemanLevel::Full),
        ("wenyan-lite", CavemanLevel::WenyanLite),
        ("wenyan_lite", CavemanLevel::WenyanLite),
        ("wenyanlite", CavemanLevel::WenyanLite),
        ("wenyan", CavemanLevel::WenyanFull),
        ("wenyan_full", CavemanLevel::WenyanFull),
        ("wenyanfull", CavemanLevel::WenyanFull),
        ("wenyan-ultra", CavemanLevel::WenyanUltra),
        ("wenyan_ultra", CavemanLevel::WenyanUltra),
        ("wenyanultra", CavemanLevel::WenyanUltra),
        ("ULTRA", CavemanLevel::Ultra),
    ];
    for (word, level) in cases {
        assert_eq!(
            parse_caveman_command(word),
            CavemanCommandAction::SetLevel {
                level,
                remainder_query: None
            },
            "{word}"
        );
        assert_eq!(
            parse_caveman_command(&format!("{word}   why  is it slow?  ")),
            CavemanCommandAction::SetLevel {
                level,
                remainder_query: Some("why is it slow?".into())
            },
            "{word} with question"
        );
    }
    for word in ["disable", "none", "OFF"] {
        assert_eq!(
            parse_caveman_command(word),
            CavemanCommandAction::Disable,
            "{word}"
        );
    }
}

#[test]
fn the_user_skills_dir_lives_under_an_assistant_skills_folder() {
    let dir = load_user_skills_dir();
    assert!(dir.ends_with("skills"), "{}", dir.display());
    assert!(
        dir.to_string_lossy().contains("srelens"),
        "{}",
        dir.display()
    );
}

#[test]
fn slash_command_matching_uses_only_the_first_word() {
    let hits = match_slash_commands("/crash my-pod-123");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].command, "crashloop");
    assert!(match_slash_commands("/zzz").is_empty());
    assert_eq!(match_slash_commands("").len(), BUILTIN_PLAYBOOKS.len());
    // Alias and name prefixes both match: "node" is an alias of nodepressure.
    assert!(match_slash_commands("/node")
        .iter()
        .any(|s| s.command == "nodepressure"));
}

#[test]
fn slash_commands_expand_discovery_prompts_per_target_kind() {
    let node = expand_slash_command("node-pressure", None, "prod", "kube-system").unwrap();
    assert!(node.contains("Scan the cluster for any nodes experiencing pressure"));

    let deploy = expand_slash_command("rollout", None, "prod", "").unwrap();
    assert!(deploy.contains("Scan all namespaces for any stalled or degraded rollouts"));
    assert!(deploy.contains("Context: Cluster 'prod', all namespaces"));

    let svc = expand_slash_command("svc", None, "prod", "web").unwrap();
    assert!(svc.contains("Scan namespace 'web' for any services with zero endpoints"));

    let summary = expand_slash_command("health", None, "prod", "web").unwrap();
    assert!(summary.contains("Investigate cluster 'prod' in namespace 'web'."));

    let targeted = expand_slash_command("summary", Some("thing"), "prod", "").unwrap();
    assert!(targeted.contains("Focus on resource 'thing' in all namespaces."));

    assert!(expand_slash_command("not-a-skill", None, "prod", "web").is_none());
}

// ---------------------------------------------------------------------------
// ai_config.rs
// ---------------------------------------------------------------------------

#[test]
fn every_provider_maps_to_its_slug_label_defaults_and_env_var() {
    let table = [
        (
            AiProvider::Anthropic,
            "anthropic",
            "ANTHROPIC_API_KEY",
            Some(ProviderKind::Anthropic),
        ),
        (
            AiProvider::OpenAi,
            "openai",
            "OPENAI_API_KEY",
            Some(ProviderKind::OpenAi),
        ),
        (
            AiProvider::Gemini,
            "gemini",
            "GEMINI_API_KEY",
            Some(ProviderKind::Gemini),
        ),
        (
            AiProvider::OpenAiCompatible,
            "openai-compatible",
            "OPENAI_COMPATIBLE_API_KEY",
            Some(ProviderKind::OpenAiCompatible),
        ),
        (AiProvider::Cursor, "cursor", "CURSOR_API_KEY", None),
    ];
    for (provider, slug, env, kind) in table {
        assert_eq!(provider_slug(provider), slug);
        assert_eq!(env_var_for_provider(provider), env);
        assert_eq!(provider.to_llm_kind(), kind);
        assert!(!provider_display_name(provider).is_empty());
        assert!(!default_model_for_provider(provider).is_empty());
    }
    assert_eq!(ALL_PROVIDERS.len(), 5);
    assert_eq!(default_base_url_for_provider(AiProvider::Cursor), "");
    assert_eq!(
        default_base_url_for_provider(AiProvider::OpenAi),
        "https://api.openai.com/v1"
    );
    assert_eq!(
        default_base_url_for_provider(AiProvider::Gemini),
        "https://generativelanguage.googleapis.com"
    );
    assert!(provider_display_name(AiProvider::Cursor).contains("cursor-agent"));
}

#[test]
fn settings_missing_optional_fields_deserialize_with_defaults() {
    let s: AiSettings = serde_json::from_str(r#"{"defaultProvider":"gemini"}"#).unwrap();
    assert_eq!(s.default_provider, AiProvider::Gemini);
    assert_eq!(s.max_tokens, 4096);
    assert_eq!(s.timeout_seconds, 120);
    assert!(s.models.is_empty());
    assert!(s.base_urls.is_empty());
    assert_eq!(s.caveman_level, None);
    // Blank maps fall back to the provider defaults.
    assert_eq!(s.get_model(AiProvider::Gemini), "gemini-2.5-flash");
    assert_eq!(
        s.get_base_url(AiProvider::Anthropic),
        "https://api.anthropic.com"
    );
    assert_eq!(s.get_timeout_seconds(AiProvider::Gemini), 120);
}

#[test]
fn blank_model_url_and_zero_timeout_fall_back_to_defaults() {
    let mut s = AiSettings::default();
    s.models.insert("openai".into(), "   ".into());
    s.base_urls.insert("openai".into(), "".into());
    s.timeouts.clear();
    s.timeout_seconds = 0;
    assert_eq!(s.get_model(AiProvider::OpenAi), "gpt-4o");
    assert_eq!(
        s.get_base_url(AiProvider::OpenAi),
        "https://api.openai.com/v1"
    );
    assert_eq!(s.get_timeout_seconds(AiProvider::OpenAi), 120);

    s.timeout_seconds = 45;
    assert_eq!(
        s.get_timeout_seconds(AiProvider::OpenAi),
        45,
        "global timeout when no per-provider entry"
    );
    s.set_timeout_seconds(AiProvider::OpenAi, 300);
    assert_eq!(s.get_timeout_seconds(AiProvider::OpenAi), 300);
    assert_eq!(s.timeout_seconds, 300);
}

#[test]
fn caveman_level_setting_round_trips_and_clears() {
    let mut s = AiSettings::default();
    s.set_caveman_level(Some(CavemanLevel::WenyanFull));
    assert_eq!(s.caveman_level.as_deref(), Some("wenyan-full"));
    assert_eq!(s.get_caveman_level(), Some(CavemanLevel::WenyanFull));
    s.set_caveman_level(None);
    assert_eq!(s.caveman_level, None);
    s.caveman_level = Some("garbage".into());
    assert_eq!(s.get_caveman_level(), None);
}

#[test]
fn an_explicit_api_key_produces_a_provider_config_and_cursor_never_does() {
    let mut s = AiSettings::default();
    s.api_keys.insert("anthropic".into(), "sk-ant-test".into());
    s.models.insert("anthropic".into(), "claude-x".into());
    s.max_tokens = 999;
    let cfg = s
        .resolve_provider_config(AiProvider::Anthropic)
        .expect("config");
    assert_eq!(cfg.kind, ProviderKind::Anthropic);
    assert_eq!(cfg.api_key, "sk-ant-test");
    assert_eq!(cfg.model, "claude-x");
    assert_eq!(cfg.base_url, "https://api.anthropic.com");
    assert_eq!(cfg.max_tokens, 999);

    s.api_keys.insert("cursor".into(), "cur-key".into());
    assert_eq!(
        s.get_api_key(AiProvider::Cursor).as_deref(),
        Some("cur-key")
    );
    assert!(
        s.resolve_provider_config(AiProvider::Cursor).is_none(),
        "cursor is not a native provider"
    );

    s.api_keys.insert("openai".into(), "   ".into());
    assert!(s.api_keys.contains_key("openai"));
    // A blank stored key is treated as absent (the env fallback is exercised below).
    assert_ne!(s.get_api_key(AiProvider::OpenAi).as_deref(), Some("   "));
}

/// The only test in this binary that touches process environment variables, so
/// it cannot race with a sibling test. Each variable is restored afterwards.
#[test]
fn settings_paths_and_key_lookups_follow_the_environment() {
    struct Restore(Vec<(&'static str, Option<String>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
    let vars = [
        "SRELENS_AI_SETTINGS_PATH",
        "SRELENS_CONFIG_DIR",
        "GEMINI_API_KEY",
        "OPENAI_COMPATIBLE_API_KEY",
    ];
    let _restore = Restore(vars.iter().map(|k| (*k, std::env::var(k).ok())).collect());

    // 1. Explicit settings file: save then load round-trips.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nested").join("ai.json");
    std::env::set_var("SRELENS_AI_SETTINGS_PATH", &file);
    std::env::remove_var("SRELENS_CONFIG_DIR");
    assert_eq!(AiSettings::config_path(), file);

    let mut s = AiSettings::default();
    s.default_provider = AiProvider::OpenAiCompatible;
    s.api_keys.insert("gemini".into(), "g-stored".into());
    s.set_timeout_seconds(AiProvider::Gemini, 33);
    let written = s.save().expect("save creates the parent directory");
    assert_eq!(written, file);
    assert!(file.is_file());
    assert_eq!(AiSettings::load(), s);

    // 2. A config dir puts the file at <dir>/ai_settings.json; the explicit path wins over it.
    std::env::set_var("SRELENS_CONFIG_DIR", dir.path());
    assert_eq!(AiSettings::config_path(), file);
    std::env::set_var("SRELENS_AI_SETTINGS_PATH", "   ");
    assert_eq!(
        AiSettings::config_path(),
        dir.path().join("ai_settings.json")
    );
    std::env::remove_var("SRELENS_AI_SETTINGS_PATH");
    assert_eq!(
        AiSettings::config_path(),
        dir.path().join("ai_settings.json")
    );

    // 3. Key lookup: stored key first, then the provider's env var, else none.
    let mut s = AiSettings::default();
    std::env::remove_var("GEMINI_API_KEY");
    assert_eq!(s.get_api_key(AiProvider::Gemini), None);
    assert!(
        s.resolve_provider_config(AiProvider::Gemini).is_none(),
        "no key, no config"
    );
    std::env::set_var("GEMINI_API_KEY", "g-env");
    assert_eq!(s.get_api_key(AiProvider::Gemini).as_deref(), Some("g-env"));
    assert_eq!(
        s.resolve_provider_config(AiProvider::Gemini)
            .unwrap()
            .api_key,
        "g-env"
    );
    s.api_keys.insert("gemini".into(), "g-stored".into());
    assert_eq!(
        s.get_api_key(AiProvider::Gemini).as_deref(),
        Some("g-stored")
    );
    std::env::set_var("GEMINI_API_KEY", "   ");
    s.api_keys.remove("gemini");
    assert_eq!(
        s.get_api_key(AiProvider::Gemini),
        None,
        "blank env value is ignored"
    );

    // 4. OpenAI-compatible endpoints need no key: "ollama" is substituted.
    std::env::remove_var("OPENAI_COMPATIBLE_API_KEY");
    let cfg = s
        .resolve_provider_config(AiProvider::OpenAiCompatible)
        .expect("keyless config");
    assert_eq!(cfg.api_key, "ollama");
    assert_eq!(cfg.base_url, "http://localhost:11434/v1");
}

// ---------------------------------------------------------------------------
// deep_link.rs
// ---------------------------------------------------------------------------

#[test]
fn view_links_format_every_target_kind() {
    let view = |target: CommandTarget| {
        DeepLink::View {
            context: Some("prod".into()),
            namespace: None,
            target,
        }
        .to_url()
    };
    assert_eq!(
        view(CommandTarget::Resource(ResourceKind::Assistant)),
        "srelens://view/prod/_/ai"
    );
    assert_eq!(
        view(CommandTarget::Resource(ResourceKind::Overview)),
        "srelens://view/prod/_/overview"
    );
    assert_eq!(
        view(CommandTarget::Resource(ResourceKind::Toolbox)),
        "srelens://view/prod/_/toolbox"
    );
    assert_eq!(
        view(CommandTarget::Resource(ResourceKind::Settings)),
        "srelens://view/prod/_/settings"
    );
    assert_eq!(
        view(CommandTarget::Resource(ResourceKind::TuiConfig)),
        "srelens://view/prod/_/config"
    );
    assert_eq!(
        view(CommandTarget::Resource(ResourceKind::Deployments)),
        "srelens://view/prod/_/deployments"
    );
    assert_eq!(
        view(CommandTarget::CustomResource(cilium_pool())),
        "srelens://view/prod/_/ciliumloadbalancerippools"
    );
    assert_eq!(view(CommandTarget::Help), "srelens://view/prod/_/help");
    assert_eq!(
        view(CommandTarget::Contexts),
        "srelens://view/prod/_/contexts"
    );
    assert_eq!(
        view(CommandTarget::Namespaces),
        "srelens://view/prod/_/namespaces"
    );
    assert_eq!(view(CommandTarget::Quit), "srelens://view/prod/_/quit");
    assert_eq!(
        view(CommandTarget::OpenUrl("x".into())),
        "srelens://view/prod/_/open/x"
    );

    let no_ctx = DeepLink::View {
        context: None,
        namespace: Some("web".into()),
        target: CommandTarget::Help,
    };
    assert_eq!(no_ctx.to_url(), "srelens://view/_/web/help");
}

#[test]
fn resource_links_with_blank_context_or_namespace_format_placeholders() {
    let link = DeepLink::Resource {
        context: String::new(),
        namespace: Some(String::new()),
        kind: "Pod".into(),
        name: "p".into(),
    };
    assert_eq!(link.to_url(), "srelens://resource/_/_/Pod/p");
}

#[test]
fn view_links_parse_with_placeholder_context_and_namespace() {
    let link = DeepLink::parse("srelens://view/_/_all/pods").unwrap();
    assert_eq!(
        link,
        DeepLink::View {
            context: None,
            namespace: None,
            target: CommandTarget::Resource(ResourceKind::Pods)
        }
    );
    assert_eq!(link.to_url(), "srelens://view/_/_/pods");

    let link = DeepLink::parse("srelens://view/prod/web/deploy").unwrap();
    assert_eq!(
        link,
        DeepLink::View {
            context: Some("prod".into()),
            namespace: Some("web".into()),
            target: CommandTarget::Resource(ResourceKind::Deployments)
        }
    );
    assert_eq!(link.to_url(), "srelens://view/prod/web/deployments");

    let dash = DeepLink::parse("srelens://view/prod/-/ai").unwrap();
    assert!(matches!(
        dash,
        DeepLink::View {
            namespace: None,
            target: CommandTarget::Resource(ResourceKind::Assistant),
            ..
        }
    ));
}

#[test]
fn malformed_srelens_urls_explain_what_is_missing() {
    let err = |s: &str| DeepLink::parse(s).unwrap_err();
    assert_eq!(err(""), "Empty deep link URL");
    assert_eq!(err("srelens://"), "Missing target in srelens:// URL");
    assert!(err("srelens://resource/prod/ns/Pod").contains("Expected format: srelens://resource/"));
    assert!(err("srelens://cluster").contains("Expected format: srelens://cluster/<context>"));
    assert!(err("srelens://view/prod/ns").contains("Expected format: srelens://view/"));
    assert_eq!(
        err("srelens://view/prod/ns/nosuchview"),
        "Unknown view target: 'nosuchview'"
    );
    assert!(err("srelens://bogus/x").starts_with("Unknown srelens URL route 'bogus'"));
}

#[test]
fn the_context_route_is_an_alias_for_cluster() {
    let link = DeepLink::parse("srelens://context/prod-eu").unwrap();
    assert_eq!(
        link,
        DeepLink::Cluster {
            context: "prod-eu".into()
        }
    );
    assert_eq!(link.to_url(), "srelens://cluster/prod-eu");
}

#[test]
fn shorthand_targets_parse_by_segment_count() {
    assert_eq!(
        DeepLink::parse("web/Deployment/api").unwrap(),
        DeepLink::Resource {
            context: String::new(),
            namespace: Some("web".into()),
            kind: "Deployment".into(),
            name: "api".into()
        }
    );
    for placeholder in ["_", "_all", "-"] {
        let link = DeepLink::parse(&format!("{placeholder}/Node/worker-1")).unwrap();
        assert!(
            matches!(
                link,
                DeepLink::Resource {
                    namespace: None,
                    ..
                }
            ),
            "{placeholder}"
        );
        let link = DeepLink::parse(&format!("prod/{placeholder}/Node/worker-1")).unwrap();
        assert!(
            matches!(&link, DeepLink::Resource { context, namespace: None, .. } if context == "prod"),
            "{placeholder}"
        );
    }
    assert_eq!(
        DeepLink::parse("prod/web/Pod/api-1").unwrap(),
        DeepLink::Resource {
            context: "prod".into(),
            namespace: Some("web".into()),
            kind: "Pod".into(),
            name: "api-1".into()
        }
    );
    assert_eq!(
        DeepLink::parse("prod/web/Pod/api-1").unwrap().to_url(),
        "srelens://resource/prod/web/Pod/api-1"
    );
    let err = DeepLink::parse("a/b/c/d/e").unwrap_err();
    assert!(
        err.starts_with("Unrecognized shorthand format 'a/b/c/d/e'"),
        "{err}"
    );
    let err = DeepLink::parse("solo/").unwrap_err();
    assert!(err.contains("Unrecognized shorthand format"), "{err}");
}

#[test]
fn a_bare_command_word_becomes_a_view_link_and_nonsense_is_rejected() {
    let link = DeepLink::parse("  nodes ").unwrap();
    assert_eq!(
        link,
        DeepLink::View {
            context: None,
            namespace: None,
            target: CommandTarget::Resource(ResourceKind::Nodes)
        }
    );
    assert_eq!(
        DeepLink::parse("nosuchthing").unwrap_err(),
        "Unrecognized target or URL: 'nosuchthing'"
    );
}

#[test]
fn a_percent_encoded_namespace_is_kept_verbatim_and_round_trips() {
    let url = "srelens://resource/prod/team%2Fweb/Pod/api-1";
    let link = DeepLink::parse(url).unwrap();
    assert!(matches!(&link, DeepLink::Resource { namespace: Some(ns), .. } if ns == "team%2Fweb"));
    assert_eq!(link.to_url(), url);
}

// ---------------------------------------------------------------------------
// commands.rs: ResourceKind tables
// ---------------------------------------------------------------------------

fn all_static_kinds() -> Vec<ResourceKind> {
    vec![
        ResourceKind::Pods,
        ResourceKind::Deployments,
        ResourceKind::StatefulSets,
        ResourceKind::DaemonSets,
        ResourceKind::Jobs,
        ResourceKind::CronJobs,
        ResourceKind::ConfigMaps,
        ResourceKind::Secrets,
        ResourceKind::ResourceQuotas,
        ResourceKind::LimitRanges,
        ResourceKind::Services,
        ResourceKind::Endpoints,
        ResourceKind::EndpointSlices,
        ResourceKind::Ingresses,
        ResourceKind::NetworkPolicies,
        ResourceKind::PersistentVolumeClaims,
        ResourceKind::PersistentVolumes,
        ResourceKind::StorageClasses,
        ResourceKind::ServiceAccounts,
        ResourceKind::Roles,
        ResourceKind::ClusterRoles,
        ResourceKind::RoleBindings,
        ResourceKind::ClusterRoleBindings,
        ResourceKind::Nodes,
        ResourceKind::Namespaces,
        ResourceKind::Events,
        ResourceKind::CustomResourceDefinitions,
        ResourceKind::HelmReleases,
        ResourceKind::PortForwards,
        ResourceKind::Overview,
        ResourceKind::Toolbox,
        ResourceKind::Assistant,
        ResourceKind::Settings,
        ResourceKind::TuiConfig,
        ResourceKind::Workloads,
    ]
}

#[test]
fn display_names_are_unique_and_match_the_display_impl() {
    let mut seen = std::collections::HashSet::new();
    for kind in all_static_kinds() {
        let name = kind.display_name().to_string();
        assert!(!name.is_empty(), "{kind:?}");
        assert_eq!(kind.to_string(), name, "Display delegates to display_name");
        assert!(seen.insert(name.clone()), "duplicate display name {name}");
    }
    assert_eq!(ResourceKind::HelmReleases.display_name(), "Helm Releases");
    assert_eq!(ResourceKind::Overview.display_name(), "Cluster Overview");
    assert_eq!(
        ResourceKind::Settings.display_name(),
        "AI & Assistant Settings"
    );
    let custom = ResourceKind::CustomResource(cilium_pool());
    assert_eq!(custom.display_name(), "CiliumLoadBalancerIPPool");
    assert_eq!(custom.to_string(), "CiliumLoadBalancerIPPool");
}

#[test]
fn watch_kinds_are_lowercase_plurals_for_watchable_kinds_only() {
    let mut watchable = 0;
    for kind in all_static_kinds() {
        match kind.watch_kind() {
            Some(w) => {
                watchable += 1;
                assert_eq!(w, w.to_lowercase(), "{kind:?}");
                assert_eq!(w, kind.display_name().to_lowercase(), "{kind:?}");
            }
            None => assert!(
                matches!(
                    kind,
                    ResourceKind::Endpoints
                        | ResourceKind::CustomResourceDefinitions
                        | ResourceKind::HelmReleases
                        | ResourceKind::PortForwards
                        | ResourceKind::Overview
                        | ResourceKind::Toolbox
                        | ResourceKind::Assistant
                        | ResourceKind::Settings
                        | ResourceKind::TuiConfig
                        | ResourceKind::Workloads
                ),
                "{kind:?} unexpectedly has no watch kind"
            ),
        }
    }
    assert_eq!(watchable, 25);
    assert_eq!(
        ResourceKind::CustomResource(cilium_pool()).watch_kind(),
        None
    );
}

#[test]
fn k8s_kinds_are_singular_pascal_case_and_crds_use_their_own_kind() {
    for kind in all_static_kinds() {
        match kind.k8s_kind() {
            Some(k) => {
                assert!(k.chars().next().unwrap().is_ascii_uppercase(), "{kind:?}");
                // Every k8s kind is a singular of its display name (Endpoints stays plural).
                if kind != ResourceKind::Endpoints {
                    assert!(
                        kind.display_name().starts_with(&k[..k.len() - 1]),
                        "{kind:?}: {k}"
                    );
                }
            }
            None => assert!(
                matches!(
                    kind,
                    ResourceKind::HelmReleases
                        | ResourceKind::PortForwards
                        | ResourceKind::Overview
                        | ResourceKind::Toolbox
                        | ResourceKind::Assistant
                        | ResourceKind::Settings
                        | ResourceKind::TuiConfig
                        | ResourceKind::Workloads
                ),
                "{kind:?} unexpectedly has no k8s kind"
            ),
        }
    }
    assert_eq!(ResourceKind::Ingresses.k8s_kind(), Some("Ingress"));
    assert_eq!(
        ResourceKind::NetworkPolicies.k8s_kind(),
        Some("NetworkPolicy")
    );
    assert_eq!(
        ResourceKind::CustomResourceDefinitions.k8s_kind(),
        Some("CustomResourceDefinition")
    );
    assert_eq!(
        ResourceKind::CustomResource(cilium_pool()).k8s_kind(),
        Some("CiliumLoadBalancerIPPool")
    );
}

#[test]
fn cluster_scoped_kinds_are_not_namespaced() {
    let cluster_scoped = [
        ResourceKind::Nodes,
        ResourceKind::Namespaces,
        ResourceKind::PersistentVolumes,
        ResourceKind::StorageClasses,
        ResourceKind::ClusterRoles,
        ResourceKind::ClusterRoleBindings,
        ResourceKind::CustomResourceDefinitions,
        ResourceKind::Overview,
        ResourceKind::Toolbox,
        ResourceKind::Assistant,
    ];
    for kind in all_static_kinds() {
        assert_eq!(
            kind.is_namespaced(),
            !cluster_scoped.contains(&kind),
            "{kind:?}"
        );
    }
    assert!(ResourceKind::CustomResource(cilium_pool()).is_namespaced());
    let mut cluster_crd = cilium_pool();
    cluster_crd.namespaced = false;
    assert!(!ResourceKind::CustomResource(cluster_crd).is_namespaced());
}

// ---------------------------------------------------------------------------
// commands.rs: dynamic definitions, resolution, suggestions
// ---------------------------------------------------------------------------

#[test]
fn static_commands_convert_to_dynamic_definitions_verbatim() {
    let pods = COMMAND_REGISTRY.iter().find(|c| c.name == "pods").unwrap();
    let dynamic = DynamicCommandDef::from(pods);
    assert_eq!(dynamic.name, "pods");
    assert_eq!(dynamic.aliases, vec!["po".to_string(), "pod".to_string()]);
    assert_eq!(dynamic.description, pods.description);
    assert_eq!(dynamic.target, CommandTarget::Resource(ResourceKind::Pods));
}

#[test]
fn a_crd_definition_collects_singular_short_names_and_lb_shorthands_without_duplicates() {
    let def = DynamicCommandDef::from(&cilium_pool());
    assert_eq!(def.name, "ciliumloadbalancerippools");
    assert_eq!(
        def.aliases,
        vec![
            "ciliumloadbalancerippool",
            "ippool",
            "ciliumlbippools",
            "ciliumlbippool"
        ]
    );
    assert_eq!(def.description, "CRD: CiliumLoadBalancerIPPool (cilium.io)");
    assert_eq!(def.target, CommandTarget::CustomResource(cilium_pool()));

    // Same singular as plural, a short name equal to the singular, no "loadbalancer".
    let plain = crd(
        "widgets",
        "widgets",
        "Widget",
        "example.io",
        &["widgets", "wd"],
    );
    let def = DynamicCommandDef::from(&plain);
    assert_eq!(def.aliases, vec!["widgets", "wd"]);
}

#[test]
fn resolve_recognises_direct_urls_and_open_prefixes() {
    assert_eq!(
        resolve_command(":srelens://cluster/prod"),
        Some(CommandTarget::OpenUrl("srelens://cluster/prod".into()))
    );
    assert_eq!(
        resolve_command("open  pods/api "),
        Some(CommandTarget::OpenUrl("pods/api".into()))
    );
    assert_eq!(
        resolve_command(":open:srelens://x"),
        Some(CommandTarget::OpenUrl("srelens://x".into()))
    );
    assert_eq!(
        resolve_command("goto nodes"),
        Some(CommandTarget::OpenUrl("nodes".into()))
    );
    assert_eq!(
        resolve_command("goto:nodes"),
        Some(CommandTarget::OpenUrl("nodes".into()))
    );
    // Bare "open" is the registry entry with an empty URL.
    assert_eq!(
        resolve_command(":open"),
        Some(CommandTarget::OpenUrl(String::new()))
    );
    assert_eq!(resolve_command(""), None);
    assert_eq!(resolve_command("  :  "), None);
}

#[test]
fn resolve_matches_static_commands_case_insensitively_then_by_prefix() {
    assert_eq!(
        resolve_command(":PODS"),
        Some(CommandTarget::Resource(ResourceKind::Pods))
    );
    assert_eq!(
        resolve_command(":Deploy"),
        Some(CommandTarget::Resource(ResourceKind::Deployments))
    );
    assert_eq!(
        resolve_command(":statef"),
        Some(CommandTarget::Resource(ResourceKind::StatefulSets))
    );
    assert_eq!(
        resolve_command(":netpo"),
        Some(CommandTarget::Resource(ResourceKind::NetworkPolicies))
    );
    assert_eq!(resolve_command(":?"), Some(CommandTarget::Help));
    assert_eq!(resolve_command(":exit"), Some(CommandTarget::Quit));
    assert_eq!(resolve_command(":zzzz"), None);
}

#[test]
fn resolve_matches_crds_by_every_name_and_by_prefix_only_from_three_characters() {
    let crds = vec![
        cilium_pool(),
        crd("widgets", "widget", "Widget", "example.io", &["wg"]),
    ];
    let pool = CommandTarget::CustomResource(cilium_pool());
    let widget = CommandTarget::CustomResource(crds[1].clone());

    assert_eq!(
        resolve_command_with_crds(":CiliumLoadBalancerIPPool", &crds),
        Some(pool.clone()),
        "kind"
    );
    assert_eq!(
        resolve_command_with_crds("ciliumloadbalancerippools.cilium.io", &crds),
        Some(pool.clone()),
        "crd name"
    );
    assert_eq!(
        resolve_command_with_crds(":ippool", &crds),
        Some(pool.clone()),
        "short name"
    );
    assert_eq!(
        resolve_command_with_crds(":ciliumlbippools", &crds),
        Some(pool.clone()),
        "lb plural"
    );
    assert_eq!(
        resolve_command_with_crds(":widget", &crds),
        Some(widget.clone()),
        "singular"
    );
    assert_eq!(
        resolve_command_with_crds(":wg", &crds),
        Some(widget.clone()),
        "two-char short name is exact"
    );
    assert_eq!(
        resolve_command_with_crds(":ciliumlb", &crds),
        Some(pool.clone()),
        "prefix of lb shorthand"
    );
    assert_eq!(
        resolve_command_with_crds(":widg", &crds),
        Some(widget),
        "prefix of plural"
    );
    // Two characters never prefix-match a CRD; the static registry wins instead.
    assert_eq!(resolve_command_with_crds(":wi", &crds), None);
    assert_eq!(
        resolve_command_with_crds(":po", &crds),
        Some(CommandTarget::Resource(ResourceKind::Pods))
    );
    // CRD prefix beats static prefix.
    let cr_like = vec![crd(
        "crontabs",
        "crontab",
        "CronTab",
        "stable.example.com",
        &[],
    )];
    assert_eq!(
        resolve_command_with_crds(":cront", &cr_like),
        Some(CommandTarget::CustomResource(cr_like[0].clone()))
    );
}

#[test]
fn namespace_and_context_switch_forms_resolve_to_their_switchers() {
    assert_eq!(
        resolve_command(":ns kube-system"),
        Some(CommandTarget::Namespaces)
    );
    assert_eq!(
        resolve_command("ctx prod-eu"),
        Some(CommandTarget::Contexts)
    );
    assert_eq!(
        resolve_command("ns   "),
        Some(CommandTarget::Namespaces),
        "bare alias after trim"
    );
}

#[test]
fn an_empty_query_suggests_the_whole_registry_and_at_most_thirty_crds() {
    let crds: Vec<CrdMeta> = (0..35)
        .map(|i| {
            crd(
                &format!("things{i}"),
                &format!("thing{i}"),
                &format!("Thing{i}"),
                "x.io",
                &[],
            )
        })
        .collect();
    let all = command_suggestions_with_crds(" : ", &crds);
    assert_eq!(all.len(), COMMAND_REGISTRY.len() + 30);
    assert!(all.iter().all(|(_, score)| *score == 0));
    assert_eq!(
        all[0].0.name, COMMAND_REGISTRY[0].name,
        "registry order is preserved"
    );
    assert_eq!(all[COMMAND_REGISTRY.len()].0.name, "things0");
    assert_eq!(command_suggestions("").len(), COMMAND_REGISTRY.len());
}

#[test]
fn static_suggestions_are_scored_by_match_quality_and_sorted() {
    let po = command_suggestions("po");
    assert_eq!(po[0].0.name, "pods");
    assert_eq!(po[0].1, 120, "exact alias");
    assert_eq!(po[1].0.name, "portforwards");
    assert_eq!(po[1].1, 100, "name prefix");

    let sv = command_suggestions(":sv");
    assert_eq!(sv.len(), 1);
    assert_eq!(
        (sv[0].0.name.as_str(), sv[0].1),
        ("services", 80),
        "alias prefix"
    );

    let health = command_suggestions("health");
    assert_eq!(health.len(), 1);
    assert_eq!(
        (health[0].0.name.as_str(), health[0].1),
        ("overview", 50),
        "description contains"
    );

    let ole = command_suggestions("ole");
    assert!(
        ole.iter().any(|(d, s)| d.name == "roles" && *s == 50),
        "name contains: {:?}",
        ole
    );

    let ties = command_suggestions("c");
    let scores: Vec<usize> = ties.iter().map(|(_, s)| *s).collect();
    let mut sorted = scores.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(scores, sorted, "descending by score");
    let hundreds: Vec<&str> = ties
        .iter()
        .filter(|(_, s)| *s == 100)
        .map(|(d, _)| d.name.as_str())
        .collect();
    let mut alpha = hundreds.clone();
    alpha.sort_unstable();
    assert_eq!(hundreds, alpha, "ties break alphabetically");
}

#[test]
fn crd_suggestions_are_scored_by_exact_prefix_alias_and_group() {
    let crds = vec![cilium_pool()];
    let score = |q: &str| {
        command_suggestions_with_crds(q, &crds)
            .into_iter()
            .find(|(d, _)| d.name == "ciliumloadbalancerippools")
            .map(|(_, s)| s)
    };
    assert_eq!(score("ippool"), Some(120), "short-name alias exact");
    assert_eq!(score("CiliumLoadBalancerIPPool"), Some(120), "kind exact");
    assert_eq!(score("ciliumlbippools"), Some(120), "lb plural exact");
    assert_eq!(score("ciliumlb"), Some(105), "lb prefix");
    assert_eq!(score("ciliumload"), Some(105), "plural prefix");
    assert_eq!(score("ipp"), Some(95), "alias prefix only");
    assert_eq!(score("cilium.io"), Some(55), "group contains");
    assert_eq!(score("balancer"), Some(55), "plural contains");
    assert_eq!(score("unrelated"), None);

    // A CRD prefix (105) outranks a static name prefix (100) for the same query.
    let both = command_suggestions_with_crds("cilium", &crds);
    assert_eq!(both[0].0.name, "ciliumloadbalancerippools");
}

#[test]
fn tui_config_file_paths_clamping_and_round_trip() {
    struct Restore(Vec<(&'static str, Option<String>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
    let vars = ["SRELENS_TUI_CONFIG_PATH", "SRELENS_CONFIG_DIR"];
    let _restore = Restore(vars.iter().map(|k| (*k, std::env::var(k).ok())).collect());

    // 1. Explicit path override
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("nested").join("tui.json");
    std::env::set_var("SRELENS_TUI_CONFIG_PATH", &file);
    std::env::remove_var("SRELENS_CONFIG_DIR");
    assert_eq!(TuiConfig::config_file_path(), file);

    // 2. Round trip save and load
    let cfg = TuiConfig {
        command_popup_max_width: 120,
        command_popup_max_visible: 12,
        command_popup_density: CommandPopupDensity::Large,
    };
    cfg.save().expect("save succeeds");
    assert!(file.is_file());
    assert_eq!(TuiConfig::load(), cfg);

    // 3. Fallback to SRELENS_CONFIG_DIR
    std::env::remove_var("SRELENS_TUI_CONFIG_PATH");
    std::env::set_var("SRELENS_CONFIG_DIR", dir.path());
    assert_eq!(TuiConfig::config_file_path(), dir.path().join("tui.json"));

    // 4. Clamping out-of-range values
    let mut clamped = TuiConfig {
        command_popup_max_width: 500,
        command_popup_max_visible: 1,
        command_popup_density: CommandPopupDensity::Compact,
    };
    clamped.clamp();
    assert_eq!(clamped.command_popup_max_width, 200);
    assert_eq!(clamped.command_popup_max_visible, 3);
    assert_eq!(clamped.command_popup_density, CommandPopupDensity::Compact);

    let mut low = TuiConfig {
        command_popup_max_width: 10,
        command_popup_max_visible: 99,
        command_popup_density: CommandPopupDensity::Large,
    };
    low.clamp();
    assert_eq!(low.command_popup_max_width, 40);
    assert_eq!(low.command_popup_max_visible, 20);
    assert_eq!(low.command_popup_density, CommandPopupDensity::Large);

    // 5. Corrupt file gracefully falls back to default
    std::fs::write(&file, "{ corrupt json").unwrap();
    std::env::set_var("SRELENS_TUI_CONFIG_PATH", &file);
    assert_eq!(TuiConfig::load(), TuiConfig::default());
}
