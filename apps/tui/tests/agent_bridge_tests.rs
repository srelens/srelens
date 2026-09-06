//! Behavioural tests for the agent bridge's timeout and process-launch edges.
//!
//! `core_logic_tests.rs` already drives `run_native_agent_turn` against a fake
//! OpenAI-compatible endpoint on loopback; this file covers the two paths it
//! leaves out: the turn that never gets an answer at all, and the boxed
//! cursor-agent path's setup when the binary cannot be launched.
//!
//! What is deliberately *not* here, because no test can reach it without
//! breaking determinism:
//!
//! * `agent.rs` 469-639 — everything after a successful `cmd.spawn()` of the
//!   cursor-agent binary (stdout/stderr pumps, exit-status handling, the
//!   fallback usage estimate). `AgentCommand::program` is the caller-supplied
//!   binary path with no injection seam, so reaching it means really executing
//!   an external program.
//! * `agent.rs` 294-317 — the `bind("127.0.0.1:0")` / `local_addr()` failure
//!   arms, which cannot be provoked on a working host.
//! * `agent.rs` 347-418 — the `tempfile::tempdir()` and `std::fs::write`
//!   failure arms, which need an unwritable temp directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use srelens_kube::client_cache::ClientCache;
use srelens_llm::types::ProviderKind;
use srelens_llm::ProviderConfig;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use srelens_tui::agent::{
    build_mcp_server, run_boxed_cursor_turn, run_native_agent_turn, McpToolInvoker,
};
use srelens_tui::event::AppEvent;

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

/// A loopback endpoint that accepts connections and never answers them. Each
/// socket is kept alive for the life of the task, so the client waits on a
/// healthy connection rather than seeing a reset — the only way to make the
/// turn's own `tokio::time::timeout` fire.
async fn stalled_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let base_url = format!("http://{}/v1", listener.local_addr().expect("local addr"));
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock);
        }
    });
    base_url
}

/// A path inside a fresh temp directory that is guaranteed not to exist, so
/// `Command::spawn` fails instead of executing anything.
fn missing_binary(dir: &tempfile::TempDir) -> String {
    dir.path()
        .join("no-such-cursor-agent")
        .to_string_lossy()
        .to_string()
}

/// Run one boxed cursor turn against a binary that is not there and return the
/// action results it emitted.
async fn boxed_turn_against_missing_binary(
    bin: String,
    model: &str,
    api_key: Option<String>,
    context: &str,
) -> Vec<(String, Result<String, String>)> {
    let (tx, mut rx) = unbounded_channel();
    run_boxed_cursor_turn(
        bin,
        model.to_string(),
        api_key,
        "why is the api pod restarting?".into(),
        context.to_string(),
        "payments".into(),
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx,
        30,
    )
    .await;
    action_results(&mut rx)
}

// ---------------------------------------------------------------------------
// run_native_agent_turn: the turn that never gets an answer
// ---------------------------------------------------------------------------

/// The turn clamps its own deadline with `timeout_seconds.max(5)`, so this test
/// costs about five seconds of wall clock no matter how small a timeout is
/// asked for. It also pins the mismatch that clamp creates: the message quotes
/// the *requested* number of seconds, not the number actually waited.
#[tokio::test]
async fn a_native_turn_that_never_gets_a_reply_times_out_and_still_reports_usage_and_done() {
    let base_url = stalled_endpoint().await;
    let (tx, mut rx) = unbounded_channel();
    let history = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let started = std::time::Instant::now();
    run_native_agent_turn(
        provider_config(&base_url),
        invoker(),
        history.clone(),
        "how are the pods?".into(),
        "kind-dev".into(),
        "payments".into(),
        tx,
        1,
    )
    .await;
    let elapsed = started.elapsed();

    let results = action_results(&mut rx);
    let titles: Vec<&str> = results.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "ai_chunk:kind-dev",
            "ai_usage:kind-dev",
            "ai_chunk:kind-dev",
            "ai_done:kind-dev"
        ],
        "the timeout is announced as a chunk, then usage, then the failure, then done"
    );

    // 1. The user-visible chunk explains the timeout and points at settings.
    let announced = results[0]
        .1
        .as_ref()
        .expect("the timeout notice is an Ok chunk");
    assert_eq!(
        announced,
        "\n[Error: AI assistant turn timed out after 1 seconds. You can increase the timeout in settings (<Ctrl+s>).]"
    );

    // 2. ...but a 1-second timeout is silently raised to the 5-second floor, so
    //    the number in that message is not the time the user actually waited.
    assert!(
        elapsed >= Duration::from_secs(4),
        "timeout_seconds is clamped by .max(5), so the turn cannot return in ~1s: {elapsed:?}"
    );

    // 3. Usage is still estimated, with no completion characters to count.
    let usage: Vec<u64> = results[1]
        .1
        .as_ref()
        .expect("usage payload")
        .split('|')
        .map(|f| f.parse().expect("numeric usage field"))
        .collect();
    let prompt_est = (("how are the pods?".len() + 200) / 4) as u64;
    assert_eq!(usage[0], prompt_est);
    assert_eq!(
        usage[1], 0,
        "nothing streamed back, so max(1)/4 rounds to zero"
    );
    assert_eq!(usage[2], 0);
    assert_eq!(usage[3], prompt_est);
    assert!(
        usage[4] >= 4000,
        "the reported duration is the real one: {usage:?}"
    );

    // 4. The turn is reported as a hard error and then closed out.
    let err = results[2]
        .1
        .as_ref()
        .expect_err("the failure is an Err chunk");
    assert_eq!(err, "AI Agent Error: AI assistant turn timed out after 1 seconds. You can increase the timeout in settings (<Ctrl+s>).");
    assert_eq!(results[3].1, Ok(String::new()));

    // 5. A timed-out turn leaves history untouched, so a retry starts clean.
    assert!(history.lock().await.is_empty());
}

// ---------------------------------------------------------------------------
// run_boxed_cursor_turn: the setup that runs before the binary is launched
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_boxed_cursor_turn_that_cannot_launch_skips_the_usage_estimate_a_native_turn_always_sends(
) {
    let dir = tempfile::tempdir().expect("temp dir");
    // No key and the sentinel "default" model: neither `--api-key` nor
    // `--model` is appended to the argv, and the turn must still fail cleanly.
    let results =
        boxed_turn_against_missing_binary(missing_binary(&dir), "default", None, "prod-eu").await;

    let titles: Vec<&str> = results.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(titles, vec!["ai_chunk:prod-eu", "ai_done:prod-eu"]);

    let err = results[0]
        .1
        .as_ref()
        .expect_err("a launch failure is an Err chunk");
    assert!(err.starts_with("Failed to launch cursor-agent: "), "{err}");

    // The native path emits `ai_usage` on every outcome, success or failure;
    // the boxed path emits it only once a child process has been started, so a
    // turn that never launches reports no token estimate at all.
    assert!(
        !titles.iter().any(|t| t.starts_with("ai_usage")),
        "no usage estimate without a child process: {titles:?}"
    );
}

#[tokio::test]
async fn a_boxed_cursor_turn_tags_every_event_with_the_context_it_was_started_for() {
    let dir = tempfile::tempdir().expect("temp dir");
    let results = boxed_turn_against_missing_binary(
        missing_binary(&dir),
        "gpt-5-codex",
        Some("k-1".into()),
        "eks-staging",
    )
    .await;

    // Both events carry the context so the UI can drop a reply that arrives
    // after the user switched clusters.
    assert!(
        results.iter().all(|(t, _)| t.ends_with(":eks-staging")),
        "{:?}",
        results.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(
        results.last().map(|(t, _)| t.as_str()),
        Some("ai_done:eks-staging")
    );
}

/// Each turn stands up its own MCP server on an ephemeral loopback port before
/// launching the agent, so two turns in flight at once must not collide on a
/// port or cross-talk on each other's channels.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_boxed_cursor_turns_at_once_each_get_their_own_port_and_channel() {
    let dir = tempfile::tempdir().expect("temp dir");
    let bin = missing_binary(&dir);

    let (left, right) = tokio::join!(
        boxed_turn_against_missing_binary(bin.clone(), "sonnet", None, "cluster-a"),
        boxed_turn_against_missing_binary(bin, "sonnet", None, "cluster-b"),
    );

    for (results, ctx) in [(&left, "cluster-a"), (&right, "cluster-b")] {
        let titles: Vec<&str> = results.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            titles,
            vec![format!("ai_chunk:{ctx}"), format!("ai_done:{ctx}")]
        );
        let err = results[0]
            .1
            .as_ref()
            .expect_err("a launch failure is an Err chunk");
        assert!(
            err.starts_with("Failed to launch cursor-agent: "),
            "neither turn may fail earlier, at the port bind: {err}"
        );
    }

    // Let each turn's shutdown signal reach its MCP server task before the
    // runtime is torn down underneath it.
    tokio::task::yield_now().await;
}
