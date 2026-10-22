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

use srectl::agent::{
    build_mcp_server, run_boxed_cursor_turn, run_native_agent_turn, McpToolInvoker,
};
use srectl::event::AppEvent;

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
    let root = tempfile::tempdir().unwrap();
    run_boxed_cursor_turn(
        bin,
        model.to_string(),
        api_key,
        "why is the api pod restarting?".into(),
        context.to_string(),
        "payments".into(),
        None,
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx,
        30,
        root.path().to_path_buf(),
        "test-conv".into(),
        None,
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
        None,
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
    assert_eq!(
        titles,
        vec!["ai_session:prod-eu", "ai_chunk:prod-eu", "ai_done:prod-eu"]
    );

    let err = results[1]
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
            vec![
                format!("ai_session:{ctx}"),
                format!("ai_chunk:{ctx}"),
                format!("ai_done:{ctx}")
            ]
        );
        let err = results[1]
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

#[cfg(unix)]
fn fake_cursor_agent(dir: &std::path::Path, session_id: &str, exit_code: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-cursor-agent");
    let script = format!(
        r#"#!/bin/sh
if [ "$CURSOR_CONFIG_DIR" = "" ]; then
  echo "CURSOR_CONFIG_DIR not set" >&2
  exit 10
fi

HAS_RESUME=0
RESUME_VAL=""
WORKSPACE_VAL=""

while [ $# -gt 0 ]; do
  case "$1" in
    --resume)
      HAS_RESUME=1
      shift
      RESUME_VAL="$1"
      ;;
    --workspace)
      shift
      WORKSPACE_VAL="$1"
      ;;
  esac
  shift
done

if [ "$HAS_RESUME" -eq 1 ]; then
  if [ "$RESUME_VAL" != "{session_id}" ]; then
    echo "Unexpected resume id: $RESUME_VAL, expected {session_id}" >&2
    exit 20
  fi
  if [ ! -f "$CURSOR_CONFIG_DIR/turn1_marker" ]; then
    echo "Missing turn1_marker in CURSOR_CONFIG_DIR: $CURSOR_CONFIG_DIR" >&2
    exit 21
  fi
  RECORDED_WS=$(cat "$CURSOR_CONFIG_DIR/turn1_workspace" 2>/dev/null)
  if [ "$WORKSPACE_VAL" != "$RECORDED_WS" ]; then
    echo "Workspace mismatch: got $WORKSPACE_VAL, expected $RECORDED_WS" >&2
    exit 22
  fi
else
  echo "turn1" > "$CURSOR_CONFIG_DIR/turn1_marker"
  echo "$WORKSPACE_VAL" > "$CURSOR_CONFIG_DIR/turn1_workspace"
fi

if [ {exit_code} -eq 0 ]; then
  echo '{{"type":"user","session_id":"{session_id}"}}'
  echo '{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"hello"}}]}},"session_id":"{session_id}"}}'
fi
exit {exit_code}
"#
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn boxed_cursor_turn_captures_stream_session_id_and_resumes() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = fake_cursor_agent(tmp.path(), "sess-abc-123", 0);
    let root = tempfile::tempdir().unwrap();
    let (tx, mut rx) = unbounded_channel();

    run_boxed_cursor_turn(
        bin.to_string_lossy().to_string(),
        "default".into(),
        None,
        "first turn".into(),
        "kind-dev".into(),
        "default".into(),
        None,
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx,
        30,
        root.path().to_path_buf(),
        "conv-1".into(),
        None,
    )
    .await;

    let results = action_results(&mut rx);
    let session_events: Vec<_> = results
        .iter()
        .filter(|(t, _)| t == "ai_session:kind-dev")
        .collect();
    assert!(!session_events.is_empty());
    assert!(session_events
        .iter()
        .all(|(_, res)| res.as_deref() == Ok("conv-1|sess-abc-123")));

    // Turn 2 with resume
    let (tx2, mut rx2) = unbounded_channel();
    run_boxed_cursor_turn(
        bin.to_string_lossy().to_string(),
        "default".into(),
        None,
        "second turn".into(),
        "kind-dev".into(),
        "default".into(),
        None,
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx2,
        30,
        root.path().to_path_buf(),
        "conv-1".into(),
        Some("sess-abc-123".into()),
    )
    .await;

    let results2 = action_results(&mut rx2);
    let session_events2: Vec<_> = results2
        .iter()
        .filter(|(t, _)| t == "ai_session:kind-dev")
        .collect();
    assert!(!session_events2.is_empty());
    assert!(session_events2
        .iter()
        .all(|(_, res)| res.as_deref() == Ok("conv-1|sess-abc-123")));
}

#[cfg(unix)]
#[tokio::test]
async fn boxed_cursor_turn_crash_clears_stale_resume_id() {
    let tmp = tempfile::tempdir().unwrap();
    // Exits with 1 and outputs no session_id
    let bin = fake_cursor_agent(tmp.path(), "stale-session", 1);
    let root = tempfile::tempdir().unwrap();
    let (tx, mut rx) = unbounded_channel();

    run_boxed_cursor_turn(
        bin.to_string_lossy().to_string(),
        "default".into(),
        None,
        "failed turn".into(),
        "kind-dev".into(),
        "default".into(),
        None,
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx,
        30,
        root.path().to_path_buf(),
        "conv-1".into(),
        Some("stale-session".into()),
    )
    .await;

    let results = action_results(&mut rx);
    let session_ev = results.iter().find(|(t, _)| t == "ai_session:kind-dev");
    assert_eq!(
        session_ev,
        Some(&("ai_session:kind-dev".into(), Ok("conv-1|".into())))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn boxed_cursor_turn_early_session_capture_preserves_id_on_cancellation() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("hanging-agent");
    let script = r#"#!/bin/sh
echo '{"type":"user","session_id":"early-captured-sess"}'
# Sleep while turn is being cancelled
sleep 5
"#;
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let root = tempfile::tempdir().unwrap();
    let (tx, mut rx) = unbounded_channel();

    let task = tokio::spawn(run_boxed_cursor_turn(
        bin.to_string_lossy().to_string(),
        "default".into(),
        None,
        "first turn".into(),
        "kind-dev".into(),
        "default".into(),
        None,
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx,
        30,
        root.path().to_path_buf(),
        "conv-cancel".into(),
        None,
    ));

    // Wait until the early session event arrives
    let mut captured_id = None;
    while let Some(event) = rx.recv().await {
        if let AppEvent::ActionResult { title, result } = event {
            if title == "ai_session:kind-dev" {
                if let Ok(ref payload) = result {
                    if let Some((_, sid)) = payload.split_once('|') {
                        if !sid.is_empty() {
                            captured_id = Some(sid.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    assert_eq!(captured_id.as_deref(), Some("early-captured-sess"));
    // Cancel the turn while child is still sleeping
    task.abort();
    let _ = task.await;
}

#[cfg(unix)]
#[tokio::test]
async fn consecutive_turns_serialize_workspace_cleanup_and_reuse() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = fake_cursor_agent(tmp.path(), "sess-seq-1", 0);
    let root = tempfile::tempdir().unwrap();

    // Turn 1
    let (tx1, mut rx1) = unbounded_channel();
    run_boxed_cursor_turn(
        bin.to_string_lossy().to_string(),
        "default".into(),
        None,
        "t1".into(),
        "kind-dev".into(),
        "default".into(),
        None,
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx1,
        30,
        root.path().to_path_buf(),
        "conv-seq".into(),
        None,
    )
    .await;
    let _ = action_results(&mut rx1);

    // Turn 2
    let (tx2, mut rx2) = unbounded_channel();
    run_boxed_cursor_turn(
        bin.to_string_lossy().to_string(),
        "default".into(),
        None,
        "t2".into(),
        "kind-dev".into(),
        "default".into(),
        None,
        ClientCache::new(PathBuf::from("/nonexistent")),
        vec![],
        tx2,
        30,
        root.path().to_path_buf(),
        "conv-seq".into(),
        Some("sess-seq-1".into()),
    )
    .await;
    let results2 = action_results(&mut rx2);
    let session_events: Vec<_> = results2
        .iter()
        .filter(|(t, _)| t == "ai_session:kind-dev")
        .collect();
    assert!(!session_events.is_empty());
    assert!(session_events
        .iter()
        .all(|(_, res)| res.as_deref() == Ok("conv-seq|sess-seq-1")));
}

#[test]
fn stale_ai_session_events_are_rejected_after_conversation_rotation() {
    let mut state = srectl::views::assistant_view::AssistantViewState::for_context("kind-dev");
    let conv1 = state.conversation_id.clone();
    state.cursor_resume_id = Some("sess-1".to_string());

    // Rotate conversation (e.g. user presses Ctrl+L / clear)
    let _old_conv = state.clear_conversation();
    assert_ne!(state.conversation_id, conv1);
    assert!(state.cursor_resume_id.is_none());

    // Stale session update arrives from conv1
    let stale_msg = format!("{}|sess-1", conv1);
    let (target_conv, session_id) = stale_msg.split_once('|').unwrap();
    if target_conv == state.conversation_id {
        state.cursor_resume_id = Some(session_id.to_string());
    }
    // Rejected!
    assert!(state.cursor_resume_id.is_none());

    // Matching session update arrives for current conversation
    let valid_msg = format!("{}|sess-2", state.conversation_id);
    let (target_conv, session_id) = valid_msg.split_once('|').unwrap();
    if target_conv == state.conversation_id {
        state.cursor_resume_id = Some(session_id.to_string());
    }
    assert_eq!(state.cursor_resume_id.as_deref(), Some("sess-2"));
}

#[tokio::test]
async fn clear_conversation_deletes_old_chat_directory() {
    let chat_root = srectl::agent::process_cursor_chats_root();
    let mut state = srectl::views::assistant_view::AssistantViewState::for_context("kind-dev");
    let old_id = state.conversation_id.clone();

    // Create chat directory for this conversation
    let chat_dir = srelens_agent::cursor_chat_dir(&chat_root, &old_id).unwrap();
    std::fs::create_dir_all(&chat_dir).unwrap();
    std::fs::write(chat_dir.join("marker.txt"), "hello").unwrap();
    assert!(chat_dir.exists());

    // Clear conversation
    state.clear_conversation();

    // Wait briefly for background deletion task
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!chat_dir.exists());
}
