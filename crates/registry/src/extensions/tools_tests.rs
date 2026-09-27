//! Installed apps' operations as MCP tools (#574), end to end over the real broker:
//! installing an app adds its tools and tells the client, disabling, updating or
//! removing it withdraws them from every snapshot, and each kind of tool runs through
//! the path the app's own screens use.
use super::executable_tests::{install_package, install_scanner, scanner_package, SCANNER};
use super::sidecars::fake::FakeSidecar;
use super::tests::{configure, fake_core, manifest};
use super::*;
use srelens_mcp::policy::{AlwaysDeny, ConfirmPolicy, FlagGated};
use srelens_mcp::{McpServer, ToolSource, Transport};
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const APPLICATIONS: &str = "plugin/org.example.argocd/applications";
const SYNC: &str = "plugin/org.example.argocd/sync";
const SCAN: &str = "plugin/org.example.scanner/scan";

struct Setup {
    _dir: tempfile::TempDir,
    path: PathBuf,
    registry: Registry,
    tools: Arc<tools::AppTools>,
}

/// A registry over a fresh inventory, as a desktop MCP host builds one, with the host
/// capabilities in `core`.
fn setup(core: Arc<Registry>) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut registry = Registry::new();
    let streams = register(
        &mut registry,
        path.clone(),
        core,
        srelens_kube::client_cache::ClientCache::new_many(vec![]),
    );
    let tools = streams.app_tools();
    Setup {
        _dir: dir,
        path,
        registry,
        tools,
    }
}

fn server(setup: &Setup, policy: Arc<dyn ConfirmPolicy>) -> McpServer {
    McpServer::new(Arc::new(setup.registry.clone()))
        .with_app_tools(setup.tools.clone())
        .with_policy(policy)
}

/// The host capabilities, with the action primitives answering with what they were
/// sent instead of patching a cluster.
fn acting_core() -> Arc<Registry> {
    let mut core = (*fake_core()).clone();
    for id in ["k8s.annotate", "k8s.mergePatch"] {
        let mut cap = core.get(id).unwrap().clone();
        cap.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
        core.register(cap);
    }
    Arc::new(core)
}

/// The Argo CD example, with its reader and three declared actions, under a local ID.
fn argocd_with_actions(path: &Path) -> u64 {
    let mut source: Value =
        serde_json::from_str(include_str!("../../../../examples/extensions/argocd.json")).unwrap();
    source["id"] = json!("org.example.argocd");
    configure(
        path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    configure(
        path,
        json!({"action": "install", "manifest": source.to_string(),
            "grants": ["k8s.listCustomResource", "k8s.annotate", "k8s.mergePatch"]}),
    )
    .unwrap()
    .plugins
    .iter()
    .find(|app| app.manifest.id == "org.example.argocd")
    .unwrap()
    .revision
}

fn install_reader(path: &Path) -> u64 {
    configure(
        path,
        json!({"action": "install", "manifest": manifest(), "grants": ["k8s.listCustomResource"]}),
    )
    .unwrap()
    .plugins[0]
        .revision
}

fn call(name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

async fn names(server: &McpServer) -> Vec<String> {
    let listed = srelens_mcp::stdio::handle_request(
        server,
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        Transport::Stdio,
    )
    .await
    .unwrap();
    listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .filter(|name| name.starts_with("plugin/"))
        .map(str::to_owned)
        .collect()
}

async fn next_message(lines: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>) -> Value {
    let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
        .await
        .expect("the server wrote nothing")
        .unwrap()
        .expect("the server hung up");
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn a_newly_installed_apps_tools_appear_and_the_client_is_told() {
    let setup = setup(fake_core());
    let (mut client, server_in) = tokio::io::duplex(1 << 16);
    let (server_out, client_out) = tokio::io::duplex(1 << 16);
    let mcp = server(&setup, Arc::new(AlwaysDeny));
    tokio::spawn(async move {
        srelens_mcp::stdio::serve(mcp, BufReader::new(server_in), server_out).await
    });
    let mut lines = BufReader::new(client_out).lines();
    let send = |message: Value| format!("{message}\n");
    client
        .write_all(send(json!({"jsonrpc":"2.0","id":1,"method":"initialize"})).as_bytes())
        .await
        .unwrap();
    let init = next_message(&mut lines).await;
    assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], true);
    client
        .write_all(send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).as_bytes())
        .await
        .unwrap();
    let before = next_message(&mut lines).await;
    assert!(!before.to_string().contains(APPLICATIONS));

    // Installed in this process, by the app's settings or an agent: the client is told.
    install_reader(&setup.path);
    assert_eq!(
        next_message(&mut lines).await,
        srelens_mcp::tools_list_changed_notification()
    );
    client
        .write_all(send(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"})).as_bytes())
        .await
        .unwrap();
    let listed = next_message(&mut lines).await;
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == APPLICATIONS)
        .unwrap_or_else(|| panic!("{APPLICATIONS} is not listed: {listed}"))
        .clone();
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["inputSchema"]["required"], json!(["context"]));
    // It reads through the broker, bound to the reader's own kind.
    client
        .write_all(
            send(call(
                APPLICATIONS,
                json!({"context":"cluster/a","namespace":"team"}),
            ))
            .as_bytes(),
        )
        .await
        .unwrap();
    let read = next_message(&mut lines).await;
    assert_eq!(read["result"]["isError"], false, "{read}");
    let text = read["result"]["content"][0]["text"].as_str().unwrap();
    let echoed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(echoed["group"], "argoproj.io");
    assert_eq!(echoed["namespace"], "team");
}

#[tokio::test]
async fn a_disabled_or_removed_apps_tools_disappear_and_an_old_snapshot_cannot_run_them() {
    let setup = setup(fake_core());
    let mcp = server(&setup, Arc::new(AlwaysDeny));
    let mut changes = setup.tools.changes();
    install_reader(&setup.path);
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    let held = setup.tools.tools().await;
    let args = json!({"context":"cluster/a","namespace":"team"});
    held.invoke(APPLICATIONS, args.clone()).await.unwrap();

    // Disabled: gone from the list, and the snapshot a caller still holds is revoked.
    configure(
        &setup.path,
        json!({"action":"enable","id":"org.example.argocd","enabled":false}),
    )
    .unwrap();
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    assert!(names(&mcp).await.is_empty());
    let refused = held
        .invoke(APPLICATIONS, args.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("withdrawn"), "{refused}");

    // Enabled again: offered again, in a new snapshot; the old one stays revoked.
    configure(
        &setup.path,
        json!({"action":"enable","id":"org.example.argocd","enabled":true}),
    )
    .unwrap();
    assert_eq!(names(&mcp).await, [APPLICATIONS]);
    assert!(held.invoke(APPLICATIONS, args.clone()).await.is_err());
    let again = setup.tools.current();

    // Removed: gone, and neither snapshot runs it.
    configure(
        &setup.path,
        json!({"action":"remove","id":"org.example.argocd"}),
    )
    .unwrap();
    assert!(names(&mcp).await.is_empty());
    assert!(again.invoke(APPLICATIONS, args.clone()).await.is_err());
    let called =
        srelens_mcp::stdio::handle_request(&mcp, &call(APPLICATIONS, args), Transport::Stdio)
            .await
            .unwrap();
    assert_eq!(called["result"]["isError"], true);
}

#[tokio::test]
async fn an_update_replaces_the_tools_and_revokes_the_previous_versions() {
    let setup = setup(fake_core());
    install_reader(&setup.path);
    let first = setup.tools.tools().await;
    let mut changes = setup.tools.changes();
    let mut updated: Value = serde_json::from_str(&manifest()).unwrap();
    updated["version"] = json!("0.2.0");
    configure(
        &setup.path,
        json!({"action":"install","manifest":updated.to_string(),"grants":["k8s.listCustomResource"]}),
    )
    .unwrap();
    assert!(
        changes.has_changed().unwrap(),
        "an update is a change to the list"
    );
    let args = json!({"context":"cluster/a","namespace":"team"});
    assert!(first.invoke(APPLICATIONS, args.clone()).await.is_err());
    setup
        .tools
        .current()
        .invoke(APPLICATIONS, args)
        .await
        .unwrap();
    // A settings save changes no tool: nothing is rebuilt, nothing is revoked.
    let before = setup.tools.current();
    changes.borrow_and_update();
    configure(
        &setup.path,
        json!({"action":"settings","id":"org.example.argocd","settings":{"team":"platform"}}),
    )
    .unwrap();
    assert!(!changes.has_changed().unwrap());
    assert!(Arc::ptr_eq(&before, &setup.tools.current()));
}

#[tokio::test]
async fn a_change_another_process_made_is_found_the_next_time_the_tools_are_asked_for() {
    let setup = setup(fake_core());
    install_reader(&setup.path);
    setup.tools.tools().await;
    // Another process disables the app: this one hears no announcement.
    let mut stored: Value = serde_json::from_slice(&fs::read(&setup.path).unwrap()).unwrap();
    stored["plugins"][0]["enabled"] = json!(false);
    fs::write(&setup.path, serde_json::to_vec(&stored).unwrap()).unwrap();
    assert!(setup.tools.current().get(APPLICATIONS).is_some());
    let mut changes = setup.tools.changes();
    assert!(setup.tools.tools().await.get(APPLICATIONS).is_none());
    assert!(changes.has_changed().unwrap());
    changes.borrow_and_update();
    // And asking again changes nothing more.
    setup.tools.tools().await;
    assert!(!changes.has_changed().unwrap());
    assert_eq!(setup.tools.poll_interval(), tools::POLL_INTERVAL);
}

#[tokio::test]
async fn a_declared_action_is_a_gated_tool_that_runs_through_the_host_action_path() {
    let setup = setup(acting_core());
    argocd_with_actions(&setup.path);
    let denied = server(&setup, Arc::new(AlwaysDeny));
    let mut listed = names(&denied).await;
    listed.sort();
    assert_eq!(
        listed,
        [
            APPLICATIONS,
            "plugin/org.example.argocd/hard-refresh",
            "plugin/org.example.argocd/refresh",
            SYNC,
        ]
    );
    let args = json!({"context":"cluster/a","namespace":"argocd","name":"web","uid":"u-1",
        "resourceVersion":"7"});
    // Nobody to ask: denied, never run.
    let refused =
        srelens_mcp::stdio::handle_request(&denied, &call(SYNC, args.clone()), Transport::Stdio)
            .await
            .unwrap();
    assert_eq!(
        refused["result"]["_meta"]["srelens/denied"], true,
        "{refused}"
    );
    // Asked through the one host policy, in the primitive's own words and level.
    let request = denied.consent_request(SYNC, &args).unwrap();
    let primitive = acting_core().get("k8s.mergePatch").unwrap().annotations;
    assert_eq!(request.kind, srelens_mcp::policy::ConsentKind::Destructive);
    assert_eq!(
        request.impact,
        primitive.impact.max(srelens_capability::Impact::Medium)
    );
    assert_eq!(request.confirm_text, primitive.confirm_text(&args));

    // Approved: the host builds the write from the manifest, for the kind its reader lists.
    let approved = server(&setup, Arc::new(FlagGated::new(true, false)));
    let mut confirmed = args.clone();
    confirmed["_confirm"] = json!(true);
    let ran = srelens_mcp::stdio::handle_request(
        &approved,
        &call(SYNC, confirmed.clone()),
        Transport::Stdio,
    )
    .await
    .unwrap();
    assert_eq!(ran["result"]["isError"], false, "{ran}");
    let sent: Value =
        serde_json::from_str(ran["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(sent["group"], "argoproj.io");
    assert_eq!(sent["kind"], "Application");
    assert_eq!(sent["name"], "web");
    assert_eq!(sent["resourceVersion"], "7");
    assert!(sent["patch"]["operation"]["sync"].is_object(), "{sent}");
    // A caller cannot hand the action a patch of its own.
    let mut forged = confirmed;
    forged["patch"] = json!({"spec":{}});
    let forged =
        srelens_mcp::stdio::handle_request(&approved, &call(SYNC, forged), Transport::Stdio)
            .await
            .unwrap();
    assert_eq!(forged["result"]["isError"], true);
}

#[tokio::test]
async fn a_policy_blocked_app_offers_no_tools() {
    let setup = setup(acting_core());
    argocd_with_actions(&setup.path);
    let mcp = server(&setup, Arc::new(AlwaysDeny));
    assert_eq!(names(&mcp).await.len(), 4);
    configure(
        &setup.path,
        json!({"action":"unsignedApps","allowUnsignedApps":false}),
    )
    .unwrap();
    assert!(names(&mcp).await.is_empty());
}

/// Every record the audit log is sent.
#[derive(Default)]
struct Spy(Mutex<Vec<srelens_mcp::audit::AuditRecord>>);

impl srelens_mcp::audit::AuditSink for Spy {
    fn record(&self, record: srelens_mcp::audit::AuditRecord) {
        self.0.lock().unwrap().push(record);
    }
}

#[tokio::test]
async fn a_sidecar_operation_is_a_sensitive_read_only_tool_its_sidecar_answers() {
    let setup = setup(fake_core());
    let fake = FakeSidecar::default();
    setup.tools.script_sidecars(Arc::new(fake.clone()));
    install_scanner(&setup.path);
    let audit = Arc::new(Spy::default());
    let mcp = server(&setup, Arc::new(AlwaysDeny)).with_audit(audit.clone());
    let listed = srelens_mcp::stdio::handle_request(
        &mcp,
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        Transport::Stdio,
    )
    .await
    .unwrap();
    let scan = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == SCAN)
        .unwrap()
        .clone();
    // Read-only and ungated: it can change nothing outside its sandbox.
    assert_eq!(scan["annotations"]["readOnlyHint"], true);
    assert_eq!(scan["annotations"]["destructiveHint"], false);
    assert_eq!(scan["inputSchema"]["properties"]["image"]["maxLength"], 512);
    assert!(mcp.consent_kind(SCAN).is_none());
    // Sensitive: its arguments are the app's vocabulary, redacted whole in the log.
    assert!(mcp.is_sensitive(SCAN));

    let ran = srelens_mcp::stdio::handle_request(
        &mcp,
        &call(SCAN, json!({"image":"nginx:1.27"})),
        Transport::Stdio,
    )
    .await
    .unwrap();
    assert_eq!(ran["result"]["isError"], false, "{ran}");
    let answer: Value =
        serde_json::from_str(ran["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        answer,
        json!({"operation":"scan","params":{"image":"nginx:1.27"},"launch":1})
    );
    let records = audit.0.lock().unwrap().clone();
    let record = records.iter().find(|record| record.tool == SCAN).unwrap();
    assert_ne!(record.args["image"], "nginx:1.27", "{:?}", record.args);

    // Held to its declaration before the sidecar sees it (#610).
    let refused = srelens_mcp::stdio::handle_request(
        &mcp,
        &call(SCAN, json!({"image":"n".repeat(513)})),
        Transport::Stdio,
    )
    .await
    .unwrap();
    assert_eq!(refused["result"]["isError"], true);
    let why = refused["result"]["content"][0]["text"].as_str().unwrap();
    assert!(why.contains("`image` is at most 512 bytes"), "{why}");
    let scans = fake
        .methods()
        .iter()
        .filter(|(_, method)| method == "scan")
        .count();
    assert_eq!(scans, 1);

    // Disabled: the tool goes, and so does the sidecar.
    configure(
        &setup.path,
        json!({"action":"enable","id":SCANNER,"enabled":false}),
    )
    .unwrap();
    assert!(names(&mcp).await.is_empty());
    for _ in 0..200 {
        if fake.exited() == [1] {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(fake.exited(), [1]);
    // Installing the same package again is an update: a new revision, a new sidecar.
    configure(
        &setup.path,
        json!({"action":"enable","id":SCANNER,"enabled":true}),
    )
    .unwrap();
    install_package(&setup.path, &scanner_package()).unwrap();
    let ran = srelens_mcp::stdio::handle_request(
        &mcp,
        &call(SCAN, json!({"image":"redis"})),
        Transport::Stdio,
    )
    .await
    .unwrap();
    let answer: Value =
        serde_json::from_str(ran["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(answer["launch"], 2);
}
