//! Installed apps' operations as MCP tools (#574), end to end over the real broker:
//! installing an app adds its tools and tells the client, disabling, updating or
//! removing it withdraws them from every snapshot, and each kind of tool runs through
//! the path the app's own screens use.
use super::executable_tests::{
    install_package, install_scanner, package_with_binaries, scanner_package, SCANNER,
};
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

#[tokio::test]
async fn native_operations_check_revision_inputs_and_scope_before_running() {
    let setup = setup(fake_core());
    let fake = FakeSidecar::default();
    setup.tools.script_sidecars(Arc::new(fake));
    let revision = install_scanner(&setup.path);
    let input = json!({"id":SCANNER,"revision":revision,"context":"demo","operation":"scan","params":{"image":"alpine:3.9"}});
    let cap = setup.registry.get("extensions.callOperation").expect("native operation capability");
    let answer = (cap.handler)(input.clone()).await.unwrap();
    assert_eq!(answer["params"], json!({"image":"alpine:3.9"}));
    for refused in [
        json!({"revision":revision + 1}),
        json!({"operation":"undeclared"}),
        json!({"params":{"image":"a".repeat(513)}}),
        json!({"params":{"image":"alpine","unknown":true}}),
        json!({"params":{"image":"alpine","clusterId":"other"}}),
        json!({"context":""}),
    ] {
        let mut request = input.clone();
        request.as_object_mut().unwrap().extend(refused.as_object().unwrap().clone());
        assert!((cap.handler)(request.clone()).await.is_err(), "accepted {request}");
    }
    configure(&setup.path, json!({"action":"enable","id":SCANNER,"enabled":false})).unwrap();
    assert!((cap.handler)(input).await.is_err());
}

#[tokio::test]
async fn streaming_operations_are_native_streams_not_broken_request_tools() {
    let setup = setup(fake_core());
    install_scanner(&setup.path);
    let mut manifest = super::executable_tests::scanner_manifest();
    manifest["srelensApiVersion"] = json!("^0.8");
    manifest["sidecar"]["operations"][0]["view"] = json!({"stream":true});
    install_package(&setup.path, &package_with_binaries(&manifest)).unwrap();
    let tools = setup.tools.tools().await;
    assert!(tools.get(SCAN).is_none());
    assert!(tools.get("plugin/org.example.scanner/status").is_some());
}

#[tokio::test]
async fn binding_availability_distinguishes_absence_from_failed_discovery() {
    let mut core = (*fake_core()).clone();
    core.register(Capability::read_only(super::crd::CHECK, "Discover report APIs", |input| async move {
        match input["context"].as_str() {
            Some("served") => Ok(json!("v1alpha1")),
            Some("absent") => Ok(Value::Null),
            _ => Err(CapabilityError::Handler("discovery forbidden".into())),
        }
    }));
    let setup = setup(Arc::new(core));
    let revision = install_reader(&setup.path);
    for (context, expected) in [("served", "served"), ("absent", "absent"), ("failed", "unknown")] {
        let out = setup.registry.invoke("extensions.bindingAvailability", json!({"id":"org.example.argocd","revision":revision,"context":context,"bindings":["applications"]})).await.unwrap();
        assert_eq!(out["bindings"][0]["state"], expected);
        if context == "failed" { assert!(out["bindings"][0]["reason"].as_str().unwrap().contains("forbidden")); }
    }
    for bindings in [json!([]), json!(["undeclared"]), json!(["applications", "applications"]), json!(vec!["applications"; 17])] {
        assert!(setup.registry.invoke("extensions.bindingAvailability", json!({"id":"org.example.argocd","revision":revision,"context":"served","bindings":bindings})).await.is_err());
    }
}

#[test]
fn workload_image_bindings_fix_kind_and_expose_only_cluster_and_namespace() {
    let core = fake_core();
    let mut value: Value = serde_json::from_str(&manifest()).unwrap();
    value["srelensApiVersion"] = json!("^0.8");
    value["permissions"] = json!(["k8s.listWorkloadImages"]);
    value["kind"] = json!("executable");
    value["sidecar"] = super::executable_tests::scanner_manifest()["sidecar"].clone();
    value["contributions"] = json!({"pages":[],"detailTabs":[],"detailLinks":[]});
    value["capabilities"] = json!([{"name":"applications","title":"Images","target":"k8s.listWorkloadImages","inputs":["context","namespace"],"arguments":{"kind":"Deployment"}}]);
    let parsed = Manifest::parse(&value.to_string()).unwrap();
    let validation = validate_app(&parsed, &["k8s.listWorkloadImages".into()], core.clone());
    assert!(validation.is_ok(), "{validation:?}");
    value["capabilities"][0]["inputs"] = json!(["context", "namespace", "cursor"]);
    let paged = Manifest::parse(&value.to_string()).unwrap();
    let paged_validation = validate_app(&paged, &["k8s.listWorkloadImages".into()], core.clone());
    assert!(paged_validation.is_ok(), "{paged_validation:?}");
    for bad in [json!({"kind":"Pod"}), json!({"kind":"Deployment","manifest":true}), json!({})] {
        value["capabilities"][0]["arguments"] = bad;
        let parsed = Manifest::parse(&value.to_string()).unwrap();
        assert!(validate_app(&parsed, &["k8s.listWorkloadImages".into()], core.clone()).is_err());
    }
}

#[tokio::test]
async fn image_cursor_errors_distinguish_binding_support_from_invalid_values() {
    let setup = setup(fake_core());
    let mut source: Value = serde_json::from_str(&manifest()).unwrap();
    source["srelensApiVersion"] = json!("^0.8");
    source["permissions"] = json!(["k8s.listWorkloadImages"]);
    source["contributions"] = json!({"pages":[],"detailTabs":[],"detailLinks":[]});
    source["capabilities"] = json!([{"name":"applications","title":"Images","target":"k8s.listWorkloadImages","inputs":["context","namespace","cursor"],"arguments":{"kind":"Deployment"}}]);
    for accepts_cursor in [true, false] {
        if !accepts_cursor {
            source["capabilities"][0]["inputs"] = json!(["context", "namespace"]);
        }
        let installed = configure(&setup.path, json!({"action":"install","manifest":source.to_string(),"grants":["k8s.listWorkloadImages"]})).unwrap();
        let revision = installed.plugins[0].revision;
        for cursor in ["x".repeat(8193), "with space".into(), "é".into()] {
            let error = setup.registry.invoke("extensions.read", json!({"id":"org.example.argocd","revision":revision,"capability":"applications","context":"demo","namespace":"team","cursor":cursor})).await.unwrap_err().to_string();
            let expected = if accepts_cursor {
                "cursor must contain at most 8192 ASCII graphic bytes"
            } else {
                "binding does not accept a page cursor"
            };
            assert!(error.contains(expected), "{error}");
        }
    }
}

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

/// Every call a sidecar's broker put to a person, approved or not.
struct Asked {
    approve: bool,
    requests: Mutex<Vec<srelens_plugin_host::sidecar::ConsentRequest>>,
}

impl srelens_plugin_host::sidecar::Consent for Asked {
    fn confirm<'a>(
        &'a self,
        request: &'a srelens_plugin_host::sidecar::ConsentRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
        self.requests.lock().unwrap().push(request.clone());
        let approve = self.approve;
        Box::pin(async move {
            if approve {
                Ok(())
            } else {
                Err("the person declined".into())
            }
        })
    }
}

/// Argo CD, as an executable app: a reader, a declared refresh, and a sidecar whose
/// `relay` operation makes one call back into the host.
fn relaying_app() -> Value {
    let mut app: Value = serde_json::from_str(&manifest()).unwrap();
    app["srelensApiVersion"] = json!("^0.6");
    app["kind"] = json!("executable");
    app["permissions"] = json!(["k8s.listCustomResource", "k8s.annotate"]);
    app["actions"] = json!([{"name":"refresh","title":"Refresh","target":"k8s.annotate",
        "resource":"applications","arguments":{"key":"argocd.argoproj.io/refresh","value":"normal"}}]);
    let binaries: serde_json::Map<String, Value> = srelens_plugin_host::SIDECAR_PLATFORMS
        .iter()
        .map(|platform| {
            (
                platform.to_string(),
                json!(format!("bin/{platform}/argocd-helper")),
            )
        })
        .collect();
    app["sidecar"] = json!({"binaries": binaries, "operations": [{"name": "relay",
        "title": "Relay a call", "inputs": [
            {"name": "method", "type": "string", "required": true},
            {"name": "params", "type": "string", "required": true, "maxLength": 4096}]}]});
    app
}

/// The relaying app installed over the broker's own facade, its sidecar a fake, its
/// calls answered with `consent` and recorded to `audit`.
fn relaying(
    consent: Arc<dyn srelens_plugin_host::sidecar::Consent>,
    audit: Arc<Spy>,
) -> (Setup, McpServer, FakeSidecar) {
    let mut core = (*acting_core()).clone();
    let mut read = core.get("k8s.getCustomResource").unwrap().clone();
    read.handler = Arc::new(|args| Box::pin(async move { Ok(args) }));
    core.register(read);
    let setup = setup(Arc::new(core));
    configure(
        &setup.path,
        json!({"action": "unsignedApps", "allowUnsignedApps": true}),
    )
    .unwrap();
    use base64::Engine as _;
    configure(
        &setup.path,
        json!({"action": "installPackage", "grants": ["k8s.listCustomResource", "k8s.annotate"],
            "package": base64::engine::general_purpose::STANDARD.encode(package_with_binaries(&relaying_app()))}),
    )
    .unwrap();
    let fake = FakeSidecar::default();
    setup.tools.script_sidecars(Arc::new(fake.clone()));
    setup.tools.serve_sidecars(sidecars::SidecarHost {
        registry: Arc::new(setup.registry.clone()),
        consent,
        audit,
    });
    let mcp = server(&setup, Arc::new(FlagGated::new(true, true)));
    (setup, mcp, fake)
}

const RELAY: &str = "plugin/org.example.argocd/relay";

async fn relay(mcp: &McpServer, method: &str, params: Value) -> Value {
    let called = srelens_mcp::stdio::handle_request(
        mcp,
        &call(
            RELAY,
            json!({"method": method, "params": params.to_string(), "_confirm": true}),
        ),
        Transport::Stdio,
    )
    .await
    .unwrap();
    assert_eq!(called["result"]["isError"], false, "{called}");
    serde_json::from_str(called["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn a_sidecar_reads_through_the_broker_and_each_write_it_asks_for_is_confirmed_naming_the_app()
{
    let asked = Arc::new(Asked {
        approve: true,
        requests: Mutex::default(),
    });
    let (setup, mcp, _fake) = relaying(asked.clone(), Arc::new(Spy::default()));
    // Its app declares a write the sidecar may ask for, so the operation itself is gated.
    assert_eq!(
        mcp.consent_kind(RELAY),
        Some(srelens_mcp::policy::ConsentKind::Destructive)
    );
    let context = json!({"clusterId": "cluster/a", "namespace": "team"});

    // A read through the app's own reader, bound to its kind; nobody is asked.
    let read = relay(
        &mcp,
        "host/read",
        json!({"capability": "applications", "context": context}),
    )
    .await;
    assert_eq!(read["answer"]["group"], "argoproj.io", "{read}");
    assert!(asked.requests.lock().unwrap().is_empty());

    // A write: put to the person first, naming the app the host started the sidecar for.
    let wrote = relay(
        &mcp,
        "host/action",
        json!({"capability": "applications", "name": "web",
        "action": "refresh", "uid": "u-1", "resourceVersion": "7", "context": context}),
    )
    .await;
    assert_eq!(
        wrote["answer"]["key"], "argocd.argoproj.io/refresh",
        "{wrote}"
    );
    let requests = asked.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tool, "extensions.action");
    assert_eq!(requests[0].app.id, "org.example.argocd");
    assert_eq!(
        requests[0].app.publisher, None,
        "an unsigned app is named as unsigned"
    );

    // The Inspector sees its process (#575).
    let inspected = setup
        .registry
        .invoke("extensions.inspect", json!({"id": "org.example.argocd"}))
        .await
        .unwrap();
    assert!(inspected["process"].is_object(), "{inspected}");
}

#[tokio::test]
async fn where_nobody_can_be_asked_a_sidecars_write_is_refused_and_recorded() {
    let audit = Arc::new(Spy::default());
    let (_setup, mcp, _fake) = relaying(
        Arc::new(srelens_plugin_host::sidecar::NoConsent),
        audit.clone(),
    );
    let refused = relay(
        &mcp,
        "host/action",
        json!({"capability": "applications", "name": "web",
        "action": "refresh", "uid": "u-1", "resourceVersion": "7",
        "context": {"clusterId": "cluster/a", "namespace": "team"}}),
    )
    .await;
    let why = refused["refused"]["message"].as_str().unwrap_or_default();
    assert!(why.contains("needs a person's confirmation"), "{refused}");
    let records = audit.0.lock().unwrap().clone();
    assert!(
        records
            .iter()
            .any(|record| record.tool == "extensions.action" && record.decision == "denied"),
        "the refusal is in the audit trail"
    );
}
