//! Opportunistic real end-to-end test for the in-app AI assistant, issue #56.
//!
//! `e2e.rs` drives the capability registry directly; this test instead drives
//! the actual path a user hits: spawn a real `claude` CLI against a real
//! (ephemeral, loopback) srelens MCP HTTP server and watch it stream back
//! `AgentEvent`s over stdout, exactly as `assistant::chat_send` does. It does
//! not go through a `tauri::AppHandle` at all — `chat_send` only borrows the
//! already-running `McpHttpManager`'s URL/token, and a plain read like
//! `k8s.listPods` never touches `McpServer`'s confirm policy — so this test
//! builds the same `McpServer`/registry pieces standalone, without standing
//! up a Tauri app.
//!
//! Needs two things neither CI nor most workstations have by default, so this
//! is `#[ignore]`d and self-skips (prints why, returns without failing)
//! rather than hard-failing when either is missing:
//!   - a real `claude` CLI on PATH, already authenticated (its own
//!     `claude auth` / `ANTHROPIC_API_KEY`) — this test does not configure
//!     that, only points it at srelens's MCP server;
//!   - a reachable kind cluster (same convention as `e2e.rs`).
//!
//! Run for real:
//!
//! ```sh
//! kind create cluster --name srelens-assistant-e2e
//! cargo test -p srelens-desktop --test assistant_e2e -- --ignored --nocapture
//! ```
//!
//! Override the context with `SRELENS_E2E_CONTEXT` (default
//! `kind-srelens-assistant-e2e`), matching `e2e.rs`'s convention.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use srelens_agent::adapter::{claude_command, McpConfig};
use srelens_agent::claude::parse_line;
use srelens_agent::event::AgentEvent;
use srelens_desktop_lib::capabilities::build_registry_with;
use srelens_kube::client_cache::ClientCache;
use srelens_mcp::auth::Token;
use srelens_mcp::McpServer;
use tokio::io::{AsyncBufReadExt, BufReader};

fn context() -> String {
    std::env::var("SRELENS_E2E_CONTEXT").unwrap_or_else(|_| "kind-srelens-assistant-e2e".to_string())
}

fn kubeconfig_paths() -> Vec<PathBuf> {
    if let Ok(kc) = std::env::var("KUBECONFIG") {
        return std::env::split_paths(&kc).collect();
    }
    let home = std::env::var("HOME").expect("HOME");
    vec![PathBuf::from(home).join(".kube/config")]
}

/// First directory on `PATH` holding an executable `claude`, as an absolute
/// path — mirrors `assistant::which_on_path`, duplicated rather than reused
/// since that helper is private to the desktop lib and not worth exporting
/// just for this opportunistic test.
fn claude_on_path() -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    std::env::split_paths(&path).find_map(|dir| {
        let candidate = dir.join("claude");
        candidate.is_file().then(|| candidate.to_string_lossy().into_owned())
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live cluster and a real, authenticated `claude` CLI on PATH"]
async fn one_real_turn_lists_pods_via_the_assistants_mcp_server() {
    let Some(claude_path) = claude_on_path() else {
        eprintln!("skipping: no `claude` on PATH");
        return;
    };

    let ctx = context();
    let cache = ClientCache::new_many(kubeconfig_paths());
    if cache.get(&ctx).await.is_err() {
        eprintln!("skipping: cluster context {ctx} is not reachable");
        return;
    }

    // Same registry the desktop app and its MCP server use; no confirm
    // policy is wired since `k8s.listPods` is a plain read and never
    // consults one (see `McpServer::new`'s `AlwaysDeny` default: it only
    // gates capabilities that actually check it).
    let registry = build_registry_with(cache);
    let server = McpServer::new(Arc::new(registry));

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let token = Token::generate();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(srelens_mcp::http::serve_http_with_shutdown(
        server,
        listener,
        async {
            let _ = shutdown_rx.await;
        },
        token.clone(),
    ));

    // Same config shape `chat_send` writes: an HTTP MCP server with a bearer
    // token, on a temp file the CLI reads by path.
    let url = format!("http://{addr}/mcp");
    let cfg = McpConfig::http(&url, token.as_str());
    let cfg_path =
        std::env::temp_dir().join(format!("srelens-assistant-e2e-{}.json", std::process::id()));
    std::fs::write(&cfg_path, serde_json::to_vec(&cfg).unwrap()).expect("write mcp config");

    // Same context preface the drawer prepends to every prompt.
    let prompt = format!("Current context: cluster {ctx}.\n\nlist pods in namespace default");
    let cmd = claude_command(&claude_path, &prompt, &cfg_path.to_string_lossy(), None);

    let mut child = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn claude");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();

    let mut saw_k8s_tool_call = false;
    let mut saw_turn_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while tokio::time::Instant::now() < deadline && !saw_turn_done {
        match tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                for event in parse_line(&line) {
                    match &event {
                        AgentEvent::ToolCallStart { tool, .. } if tool.starts_with("k8s.") => {
                            saw_k8s_tool_call = true;
                        }
                        AgentEvent::TurnDone => saw_turn_done = true,
                        _ => {}
                    }
                }
            }
            Ok(Ok(None)) => break, // stdout closed without a result line
            Ok(Err(_)) => break,   // read error
            Err(_) => {}           // per-read timeout: keep polling until the deadline
        }
    }

    let _ = child.start_kill();
    let _ = shutdown_tx.send(());
    let _ = server_task.await;
    let _ = std::fs::remove_file(&cfg_path);

    assert!(saw_k8s_tool_call, "expected at least one k8s.* toolCallStart");
    assert!(saw_turn_done, "expected a turnDone");
}
