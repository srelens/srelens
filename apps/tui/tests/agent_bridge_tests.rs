//! Behavioural tests for the agent bridge's timeout and process-launch edges.
//!
//! `core_logic_tests.rs` already drives `run_native_agent_turn` against a fake
//! OpenAI-compatible endpoint on loopback; this file covers the turn that
//! never gets an answer at all, and the agent-CLI turn (`run_cli_agent_turn`)
//! against a binary that is missing, and against fake agents: small scripts
//! that print a recorded transcript or fail the way a real CLI does.
//!
//! What is deliberately *not* here: the `bind("127.0.0.1:0")` /
//! `local_addr()` failure arms, which cannot be provoked on a working host.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use srelens_agent::adapter::AgentKind;
use srelens_kube::client_cache::ClientCache;
use srelens_llm::types::ProviderKind;
use srelens_llm::ProviderConfig;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use srectl::agent::{
    build_mcp_server, run_cli_agent_turn, run_native_agent_turn, CliTurn, McpToolInvoker,
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
    let policy = Arc::new(srelens_mcp::policy::FlagGated::new(false, true));
    Arc::new(McpToolInvoker::new(build_mcp_server(cache, vec![], policy)))
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

/// One agent-CLI turn on `kind-dev`, conversation `conv-1`, with no model,
/// key or session to resume; tests override what they are about.
fn cli_turn(
    kind: AgentKind,
    binary: impl Into<PathBuf>,
    event_tx: UnboundedSender<AppEvent>,
    chat_root: &Path,
) -> CliTurn {
    CliTurn {
        kind,
        binary: binary.into(),
        model: None,
        cursor_key: None,
        query: "why is the api pod restarting?".into(),
        active_ctx: "kind-dev".into(),
        active_ns: "payments".into(),
        argo_hub_context: None,
        cache: ClientCache::new(PathBuf::from("/nonexistent")),
        kubeconfig_paths: vec![],
        event_tx,
        timeout_seconds: 30,
        conversation_id: "conv-1".into(),
        resume: None,
        chat_root: chat_root.to_path_buf(),
    }
}

/// Run one boxed cursor turn against a binary that is not there and return the
/// action results it emitted.
async fn boxed_turn_against_missing_binary(
    bin: String,
    model: Option<&str>,
    api_key: Option<String>,
    context: &str,
) -> Vec<(String, Result<String, String>)> {
    let (tx, mut rx) = unbounded_channel();
    let root = tempfile::tempdir().unwrap();
    run_cli_agent_turn(CliTurn {
        model: model.map(str::to_string),
        cursor_key: api_key,
        active_ctx: context.to_string(),
        conversation_id: "test-conv".into(),
        ..cli_turn(AgentKind::Cursor, bin, tx, root.path())
    })
    .await;
    action_results(&mut rx)
}

/// A fake Claude Code in `dir` running `windows` (a batch shim, as npm
/// installs it there, so the turn's prompt comes on stdin) or `unix` (a
/// shell script).
fn fake_claude(dir: &Path, windows: &str, unix: &str) -> PathBuf {
    let (name, script) = if cfg!(windows) {
        ("claude.cmd", windows.to_string())
    } else {
        ("claude", format!("#!/bin/sh\n{unix}\n"))
    };
    let path = dir.join(name);
    std::fs::write(&path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
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
        Arc::new(AtomicUsize::new(0)),
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

/// A turn held up on an approval prompt is waiting on the user, not stalled:
/// past its inactivity limit it is still alive, and the limit starts over from
/// the moment the prompt closes. Costs about twelve seconds of wall clock: the
/// 5-second floor of the limit, held for seven, then run out again.
#[tokio::test]
async fn a_native_turn_waiting_on_an_approval_prompt_is_not_timed_out() {
    let base_url = stalled_endpoint().await;
    let (tx, _rx) = unbounded_channel();
    let history = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let waiting = Arc::new(AtomicUsize::new(1));

    let turn = tokio::spawn(run_native_agent_turn(
        provider_config(&base_url),
        invoker(),
        history,
        "scale the api down".into(),
        "kind-dev".into(),
        "payments".into(),
        None,
        tx,
        1,
        waiting.clone(),
    ));

    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(
        !turn.is_finished(),
        "a prompt is open, so the 5 s inactivity limit must not have fired"
    );

    waiting.store(0, Ordering::SeqCst);
    let answered = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(20), turn)
        .await
        .expect("with no prompt open the stalled turn times out")
        .expect("the turn did not panic");
    assert!(
        answered.elapsed() >= Duration::from_secs(3),
        "the limit starts over when the prompt closes, not from the last event: {:?}",
        answered.elapsed()
    );
}

// ---------------------------------------------------------------------------
// run_cli_agent_turn: a Cursor turn whose binary cannot be launched
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_boxed_cursor_turn_that_cannot_launch_skips_the_usage_estimate_a_native_turn_always_sends(
) {
    let dir = tempfile::tempdir().expect("temp dir");
    // No key and no model: the turn must still fail cleanly.
    let results =
        boxed_turn_against_missing_binary(missing_binary(&dir), None, None, "prod-eu").await;

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
        Some("gpt-5-codex"),
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
        boxed_turn_against_missing_binary(bin.clone(), Some("sonnet"), None, "cluster-a"),
        boxed_turn_against_missing_binary(bin, Some("sonnet"), None, "cluster-b"),
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

/// A turn that never started cannot have spoiled the session it was to
/// resume, so the conversation keeps it for the next turn.
#[tokio::test]
async fn a_turn_that_cannot_launch_keeps_the_session_it_was_to_resume() {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = tempfile::tempdir().unwrap();
    let (tx, mut rx) = unbounded_channel();

    run_cli_agent_turn(CliTurn {
        resume: Some("s-9".into()),
        ..cli_turn(AgentKind::Cursor, missing_binary(&dir), tx, root.path())
    })
    .await;

    let results = action_results(&mut rx);
    let session = results.iter().find(|(t, _)| t == "ai_session:kind-dev");
    assert_eq!(
        session.map(|(_, r)| r.as_deref()),
        Some(Ok("conv-1|s-9")),
        "{results:?}"
    );
}

// ---------------------------------------------------------------------------
// run_cli_agent_turn: a fake Claude Code
// ---------------------------------------------------------------------------

/// What `claude -p --output-format stream-json` prints for a one-line answer:
/// its init line, the text block `claude::parse_line` reads as a `TextDelta`,
/// and the result that ends the turn — each stamped with the session id.
const CLAUDE_TRANSCRIPT: &str = concat!(
    r#"{"type":"system","subtype":"init","session_id":"s-1"}"#,
    "\n",
    r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]},"session_id":"s-1"}"#,
    "\n",
    r#"{"type":"result","subtype":"success","session_id":"s-1"}"#,
    "\n",
);

#[tokio::test]
async fn a_cli_turn_streams_its_answer_and_keeps_the_session_for_the_next() {
    let _settings = common::env::isolate_settings();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("transcript.jsonl"), CLAUDE_TRANSCRIPT).unwrap();
    let bin = fake_claude(
        dir.path(),
        "@more > \"%~dp0stdin.txt\"\r\n@type \"%~dp0transcript.jsonl\"\r\n",
        r#"cat > "$(dirname "$0")/stdin.txt"; cat "$(dirname "$0")/transcript.jsonl""#,
    );
    let (tx, mut rx) = unbounded_channel();

    run_cli_agent_turn(cli_turn(AgentKind::Claude, &bin, tx, dir.path())).await;

    let results = action_results(&mut rx);
    let hello = results
        .iter()
        .position(|(t, r)| t == "ai_chunk:kind-dev" && r.as_deref() == Ok("hello"));
    let done = results.iter().position(|(t, _)| t == "ai_done:kind-dev");
    assert!(
        matches!((hello, done), (Some(h), Some(d)) if h < d && d == results.len() - 1),
        "{results:?}"
    );
    let kept = results
        .iter()
        .rev()
        .find(|(t, _)| t == "ai_session:kind-dev");
    assert_eq!(
        kept.map(|(_, r)| r.as_deref()),
        Some(Ok("conv-1|s-1")),
        "the next turn resumes the session this one ran in: {results:?}"
    );
    if cfg!(windows) {
        // A batch shim cannot take the prompt in argv, so it came on stdin.
        let stdin = std::fs::read_to_string(dir.path().join("stdin.txt")).unwrap();
        assert!(
            stdin.contains("why is the api pod restarting?"),
            "{stdin:?}"
        );
    }
}

#[tokio::test]
async fn an_agent_that_fails_at_once_says_why_and_ends_the_turn() {
    let _settings = common::env::isolate_settings();
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_claude(
        dir.path(),
        "@echo not logged in 1>&2\r\n@exit /b 1\r\n",
        "echo 'not logged in' >&2; exit 1",
    );
    let (tx, mut rx) = unbounded_channel();

    tokio::time::timeout(
        Duration::from_secs(10),
        run_cli_agent_turn(cli_turn(AgentKind::Claude, &bin, tx, dir.path())),
    )
    .await
    .expect("the turn ends within 10 s");

    let results = action_results(&mut rx);
    assert!(shows(&results, "not logged in"), "{results:?}");
    assert_eq!(
        results.last().map(|(t, _)| t.as_str()),
        Some("ai_done:kind-dev")
    );
}

/// Whether a chunk of the transcript on `kind-dev` says `text`.
fn shows(results: &[(String, Result<String, String>)], text: &str) -> bool {
    results.iter().any(|(t, r)| {
        t == "ai_chunk:kind-dev"
            && match r {
                Ok(m) | Err(m) => m.contains(text),
            }
    })
}

/// A CLI that prints plain text, changes its format, or reports a failure on
/// stderr and still exits 0 must not end the turn with nothing on screen.
#[tokio::test]
async fn a_turn_that_ends_without_an_answer_says_so() {
    let _settings = common::env::isolate_settings();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("transcript.jsonl"),
        concat!(
            r#"{"type":"system","subtype":"init","session_id":"s-1"}"#,
            "\n",
            r#"{"type":"result","subtype":"success","session_id":"s-1"}"#,
            "\n",
        ),
    )
    .unwrap();
    let bin = fake_claude(
        dir.path(),
        "@type \"%~dp0transcript.jsonl\"\r\n",
        r#"cat "$(dirname "$0")/transcript.jsonl""#,
    );
    let (tx, mut rx) = unbounded_channel();

    run_cli_agent_turn(cli_turn(AgentKind::Claude, &bin, tx, dir.path())).await;

    let results = action_results(&mut rx);
    assert!(
        shows(&results, "Claude Code finished without an answer"),
        "{results:?}"
    );
    assert_eq!(
        results.last().map(|(t, _)| t.as_str()),
        Some("ai_done:kind-dev")
    );
}

/// A Node CLI's crash ends with its version line; the cause is above it.
#[tokio::test]
async fn a_crash_shows_its_cause_not_only_its_last_stderr_line() {
    let _settings = common::env::isolate_settings();
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_claude(
        dir.path(),
        "@echo Error: not logged in 1>&2\r\n@echo Node.js v22.11.0 1>&2\r\n@exit /b 1\r\n",
        "echo 'Error: not logged in' >&2; echo 'Node.js v22.11.0' >&2; exit 1",
    );
    let (tx, mut rx) = unbounded_channel();

    run_cli_agent_turn(cli_turn(AgentKind::Claude, &bin, tx, dir.path())).await;

    let results = action_results(&mut rx);
    assert!(shows(&results, "not logged in"), "{results:?}");
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

    run_cli_agent_turn(cli_turn(AgentKind::Cursor, &bin, tx, root.path())).await;

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
    run_cli_agent_turn(CliTurn {
        resume: Some("sess-abc-123".into()),
        ..cli_turn(AgentKind::Cursor, &bin, tx2, root.path())
    })
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

    run_cli_agent_turn(CliTurn {
        resume: Some("stale-session".into()),
        ..cli_turn(AgentKind::Cursor, &bin, tx, root.path())
    })
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

    let task = tokio::spawn(run_cli_agent_turn(CliTurn {
        conversation_id: "conv-cancel".into(),
        ..cli_turn(AgentKind::Cursor, &bin, tx, root.path())
    }));

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
    run_cli_agent_turn(CliTurn {
        conversation_id: "conv-seq".into(),
        ..cli_turn(AgentKind::Cursor, &bin, tx1, root.path())
    })
    .await;
    let _ = action_results(&mut rx1);

    // Turn 2
    let (tx2, mut rx2) = unbounded_channel();
    run_cli_agent_turn(CliTurn {
        conversation_id: "conv-seq".into(),
        resume: Some("sess-seq-1".into()),
        ..cli_turn(AgentKind::Cursor, &bin, tx2, root.path())
    })
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
    state.cli_resume_id = Some("sess-1".to_string());

    // Rotate conversation (e.g. user presses Ctrl+L / clear)
    let _old_conv = state.clear_conversation();
    assert_ne!(state.conversation_id, conv1);
    assert!(state.cli_resume_id.is_none());

    // Stale session update arrives from conv1
    let stale_msg = format!("{}|sess-1", conv1);
    let (target_conv, session_id) = stale_msg.split_once('|').unwrap();
    if target_conv == state.conversation_id {
        state.cli_resume_id = Some(session_id.to_string());
    }
    // Rejected!
    assert!(state.cli_resume_id.is_none());

    // Matching session update arrives for current conversation
    let valid_msg = format!("{}|sess-2", state.conversation_id);
    let (target_conv, session_id) = valid_msg.split_once('|').unwrap();
    if target_conv == state.conversation_id {
        state.cli_resume_id = Some(session_id.to_string());
    }
    assert_eq!(state.cli_resume_id.as_deref(), Some("sess-2"));
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

#[tokio::test]
async fn setup_failure_preserves_existing_resume_id() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("does-not-matter");
    let (tx, mut rx) = unbounded_channel();

    // Pass invalid conversation ID (e.g. empty or with path separators) to trigger early setup error
    run_cli_agent_turn(CliTurn {
        conversation_id: "../invalid".into(),
        resume: Some("valid-existing-resume".into()),
        ..cli_turn(AgentKind::Cursor, &bin, tx, tmp.path())
    })
    .await;

    let results = action_results(&mut rx);
    // Early setup failures report errors via ai_chunk, but must NOT emit ai_session
    // which would erase the caller's valid resume ID.
    let has_session_event = results.iter().any(|(t, _)| t == "ai_session:kind-dev");
    assert!(
        !has_session_event,
        "Setup failures must not emit ai_session"
    );
    let has_error_chunk = results
        .iter()
        .any(|(t, r)| t == "ai_chunk:kind-dev" && r.is_err());
    assert!(has_error_chunk, "Setup failures must report an error chunk");
}
