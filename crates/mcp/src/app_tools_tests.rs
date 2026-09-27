//! Installed apps' tools (#574), against a fake source: what a client is told when they
//! change, on both transports, and that one call is decided and run in one snapshot.
use crate::policy::{ConfirmPolicy, ConsentRequest, Decision};
use crate::stdio::{handle_request, serve};
use crate::{McpServer, ToolSource, Transport};
use serde_json::{json, Value};
use srelens_capability::{Annotations, Capability, CapabilityError, Registry};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// One tool in a fake snapshot: its id, its annotations, and what it answers.
type Spec = (&'static str, Annotations, &'static str);

/// A source whose snapshots the test replaces. Each snapshot's tools refuse once the
/// next one replaces it, as a revoked `Registration`'s do.
struct FakeTools {
    current: Mutex<(Arc<Registry>, Arc<AtomicBool>)>,
    changes: tokio::sync::watch::Sender<u64>,
    /// How often `tools` was asked, which is what a poll does.
    asked: AtomicUsize,
    /// A change another process made, found the next time `tools` is asked.
    elsewhere: Mutex<Option<Vec<Spec>>>,
}

impl FakeTools {
    fn new(tools: &[Spec]) -> Arc<Self> {
        let (registry, active) = snapshot(tools);
        Arc::new(Self {
            current: Mutex::new((registry, active)),
            changes: tokio::sync::watch::channel(0).0,
            asked: AtomicUsize::new(0),
            elsewhere: Mutex::default(),
        })
    }

    /// Replace the snapshot, revoking the old one, as an install or a disable does.
    fn replace(&self, tools: &[Spec]) {
        let (registry, active) = snapshot(tools);
        let (_, old) = std::mem::replace(&mut *self.current.lock().unwrap(), (registry, active));
        old.store(false, Ordering::SeqCst);
        self.changes.send_modify(|generation| *generation += 1);
    }
}

/// A snapshot of `tools`, each answering with its text while the snapshot is current.
fn snapshot(
    tools: &[Spec],
) -> (Arc<Registry>, Arc<AtomicBool>) {
    let active = Arc::new(AtomicBool::new(true));
    let mut registry = Registry::new();
    for (id, annotations, answer) in tools {
        let live = active.clone();
        let answer = *answer;
        let mut cap = Capability::read_only(id, "an app's tool", move |_| {
            let live = live.clone();
            async move {
                if live.load(Ordering::SeqCst) {
                    Ok(json!(answer))
                } else {
                    Err(CapabilityError::Handler("withdrawn".into()))
                }
            }
        });
        cap.annotations = *annotations;
        registry.register(cap);
    }
    (Arc::new(registry), active)
}

#[async_trait::async_trait]
impl ToolSource for FakeTools {
    async fn tools(&self) -> Arc<Registry> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if let Some(tools) = self.elsewhere.lock().unwrap().take() {
            self.replace(&tools);
        }
        self.current()
    }
    fn current(&self) -> Arc<Registry> {
        self.current.lock().unwrap().0.clone()
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }
    fn poll_interval(&self) -> Duration {
        Duration::from_millis(20)
    }
}

const SCAN: &str = "plugin/org.example.scanner/scan";
/// A sidecar operation's row: read-only, and sensitive, so its arguments are redacted.
const SIDECAR_ROW: Annotations = Annotations {
    sensitive: true,
    ..Annotations::READ_ONLY
};
const SYNC: &str = "plugin/org.example.argocd/sync";

fn host() -> Registry {
    let mut reg = Registry::new();
    reg.register(Capability::read_only(
        "ping",
        "health",
        |v| async move { Ok(v) },
    ));
    reg
}

fn server(tools: Arc<FakeTools>) -> McpServer {
    McpServer::new(Arc::new(host())).with_app_tools(tools)
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}

async fn names(server: &McpServer) -> Vec<String> {
    let listed = handle_request(
        server,
        &request(1, "tools/list", json!({})),
        Transport::Stdio,
    )
    .await
    .unwrap();
    listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_server_with_app_tools_says_its_tool_list_changes() {
    let init = request(1, "initialize", json!({}));
    let plain = handle_request(&McpServer::new(Arc::new(host())), &init, Transport::Stdio)
        .await
        .unwrap();
    assert_eq!(plain["result"]["capabilities"]["tools"], json!({}));
    let with = handle_request(&server(FakeTools::new(&[])), &init, Transport::Stdio)
        .await
        .unwrap();
    assert_eq!(
        with["result"]["capabilities"]["tools"],
        json!({"listChanged": true})
    );
}

#[tokio::test]
async fn app_tools_are_listed_beside_the_hosts_and_follow_each_snapshot() {
    let tools = FakeTools::new(&[(SCAN, SIDECAR_ROW, "scanned")]);
    let server = server(tools.clone());
    assert_eq!(names(&server).await, ["ping", SCAN]);
    let listed = handle_request(
        &server,
        &request(2, "tools/list", json!({})),
        Transport::Stdio,
    )
    .await
    .unwrap();
    let scan = &listed["result"]["tools"][1];
    assert_eq!(scan["annotations"]["readOnlyHint"], true);
    assert_eq!(scan["annotations"]["destructiveHint"], false);
    // A sensitive tool's arguments are redacted whole in the audit log.
    assert!(server.is_sensitive(SCAN));

    tools.replace(&[]);
    assert_eq!(names(&server).await, ["ping"]);
    let gone = handle_request(
        &server,
        &request(3, "tools/call", json!({"name": SCAN, "arguments": {}})),
        Transport::Stdio,
    )
    .await
    .unwrap();
    assert_eq!(gone["result"]["isError"], true);
    // A host tool is never an app's, whatever the snapshot holds.
    let collision = FakeTools::new(&[("ping", Annotations::READ_ONLY, "not the host")]);
    let server = crate::McpServer::new(Arc::new(host())).with_app_tools(collision);
    assert_eq!(names(&server).await, ["ping"]);
    assert_eq!(
        server.call_tool("ping", json!("pong")).await.unwrap(),
        "pong"
    );
}

/// Approves, but only after the app has changed underneath the call.
struct ChangesWhileAsking(Arc<FakeTools>, Mutex<Vec<ConsentRequest>>);

#[async_trait::async_trait]
impl ConfirmPolicy for ChangesWhileAsking {
    async fn confirm(&self, request: &ConsentRequest) -> Decision {
        self.1.lock().unwrap().push(request.clone());
        // An update lands while the person is reading the prompt.
        self.0
            .replace(&[(SYNC, Annotations::MUTATING, "synced by the new version")]);
        Decision::Approved
    }
}

#[tokio::test]
async fn a_call_runs_in_the_snapshot_it_was_asked_about_or_not_at_all() {
    let tools = FakeTools::new(&[(SYNC, Annotations::MUTATING, "synced")]);
    let policy = Arc::new(ChangesWhileAsking(tools.clone(), Mutex::default()));
    let server = server(tools.clone()).with_policy(policy.clone());
    let called = handle_request(
        &server,
        &request(
            1,
            "tools/call",
            json!({"name": SYNC, "arguments": {"context": "prod"}}),
        ),
        Transport::Stdio,
    )
    .await
    .unwrap();
    // Consent went through the one host policy, with the tool's own gate.
    let asked = policy.1.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].tool, SYNC);
    assert_eq!(asked[0].kind, crate::policy::ConsentKind::Destructive);
    // The old snapshot was revoked mid-prompt: refused, not run as the new version.
    assert_eq!(called["result"]["isError"], true, "{called}");
    let text = called["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("withdrawn"), "{text}");
    assert!(!text.contains("new version"), "{text}");
}

#[tokio::test]
async fn a_gated_app_tool_is_denied_where_nobody_can_be_asked() {
    // The default policy: no consent mechanism, so no approval.
    let server = McpServer::new(Arc::new(host())).with_app_tools(FakeTools::new(&[(
        SYNC,
        Annotations::MUTATING,
        "synced",
    )]));
    let called = handle_request(
        &server,
        &request(
            1,
            "tools/call",
            json!({"name": SYNC, "arguments": {"context": "prod"}}),
        ),
        Transport::Stdio,
    )
    .await
    .unwrap();
    assert_eq!(
        called["result"]["_meta"]["srelens/denied"], true,
        "{called}"
    );
}

/// The serve loop over a real pipe, and the lines it writes.
fn serve_over_pipe(
    server: McpServer,
) -> (
    tokio::io::DuplexStream,
    tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
) {
    let (client_in, server_in) = tokio::io::duplex(1 << 16);
    let (server_out, client_out) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move { serve(server, BufReader::new(server_in), server_out).await });
    (client_in, BufReader::new(client_out).lines())
}

async fn next_message(lines: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>) -> Value {
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("the server wrote nothing")
        .unwrap()
        .expect("the server hung up");
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn stdio_tells_the_client_each_time_the_tools_change() {
    let tools = FakeTools::new(&[]);
    let (mut client, mut lines) = serve_over_pipe(server(tools.clone()));
    client
        .write_all(format!("{}\n", request(1, "initialize", json!({}))).as_bytes())
        .await
        .unwrap();
    assert_eq!(next_message(&mut lines).await["id"], 1);

    // Installed in this process: told at once.
    tools.replace(&[(SCAN, SIDECAR_ROW, "scanned")]);
    let told = next_message(&mut lines).await;
    assert_eq!(told, crate::tools_list_changed_notification());
    client
        .write_all(format!("{}\n", request(2, "tools/list", json!({}))).as_bytes())
        .await
        .unwrap();
    let listed = next_message(&mut lines).await;
    assert_eq!(listed["result"]["tools"][1]["name"], SCAN);

    // Removed by another process: found by the poll, then told.
    *tools.elsewhere.lock().unwrap() = Some(Vec::new());
    let told = next_message(&mut lines).await;
    assert_eq!(told, crate::tools_list_changed_notification());
    assert!(tools.asked.load(Ordering::SeqCst) >= 2);
    client
        .write_all(format!("{}\n", request(3, "tools/list", json!({}))).as_bytes())
        .await
        .unwrap();
    let listed = next_message(&mut lines).await;
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_request_that_changed_the_tools_is_answered_before_the_notification() {
    // A host tool that installs an app, as `extensions.configure` does.
    let tools = FakeTools::new(&[]);
    let installer = tools.clone();
    let mut reg = host();
    reg.register(Capability::read_only("install", "installs", move |_| {
        let installer = installer.clone();
        async move {
            installer.replace(&[(SCAN, SIDECAR_ROW, "scanned")]);
            Ok(json!("installed"))
        }
    }));
    let server = McpServer::new(Arc::new(reg)).with_app_tools(tools);
    let (mut client, mut lines) = serve_over_pipe(server);
    client
        .write_all(
            format!(
                "{}\n",
                request(1, "tools/call", json!({"name":"install","arguments":{}}))
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(next_message(&mut lines).await["id"], 1);
    assert_eq!(
        next_message(&mut lines).await,
        crate::tools_list_changed_notification()
    );
}

#[tokio::test]
async fn the_http_stream_tells_its_client_when_the_tools_change() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    let tools = FakeTools::new(&[]);
    let app = crate::http::router(server(tools.clone()));
    let get = Request::builder()
        .method("GET")
        .uri("/mcp")
        .header("host", "127.0.0.1:8765")
        .header("accept", "text/event-stream")
        .body(Body::empty())
        .unwrap();
    let stream = app.oneshot(get).await.unwrap();
    let mut body = stream.into_body().into_data_stream();
    tools.replace(&[(SCAN, SIDECAR_ROW, "scanned")]);
    let mut seen = String::new();
    for _ in 0..5 {
        use futures_core::Stream as _;
        let next = std::future::poll_fn(|cx| std::pin::Pin::new(&mut body).poll_next(cx));
        match tokio::time::timeout(Duration::from_secs(2), next).await {
            Ok(Some(Ok(bytes))) => {
                seen.push_str(&String::from_utf8_lossy(&bytes));
                if seen.contains("notifications/tools/list_changed") {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(
        seen.contains("notifications/tools/list_changed"),
        "the HTTP client was not told: {seen:?}"
    );
}
