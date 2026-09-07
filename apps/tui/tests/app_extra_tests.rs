//! Integration tests for the parts of `App` that only run when a cluster
//! answers: the background refreshes, the CRD discovery pass, the cluster
//! overview roll-up, the YAML/describe fetchers and the write actions
//! (delete/restart/scale/cordon).
//!
//! None of this needs a real Kubernetes cluster. `fake_cluster` binds a
//! loopback socket, speaks just enough HTTP/1.1 to satisfy kube-rs, and hands
//! back a kubeconfig pointing at it; a test then points the app's client cache
//! at that kubeconfig. Watches are deliberately left pointing at the app's own
//! (empty) kubeconfig list so no long-poll ever reaches the fake server.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::UnboundedReceiver;

use srelens_kube::lineage::{LineageNode, LineageRelation};
use srelens_kube::node_inspector::{NodeInspectorDetails, NodePodItem};
use srelens_tui::app::{ActiveView, App, SuspendAction};
use srelens_tui::commands::{CrdMeta, ResourceKind};
use srelens_tui::event::AppEvent;
use srelens_tui::ui::Modal;
use srelens_tui::views::helm_view::{HelmReleaseItem, HelmViewState};
use srelens_tui::views::node_inspector_view::NodeInspectorState;
use srelens_tui::views::port_forward_view::{PortForwardEntry, PortForwardViewState};
use srelens_tui::views::resource_table::ResourceTableState;
use srelens_tui::views::toolbox_view::{ToolStatusItem, ToolboxViewState};
use srelens_tui::views::tree_view::TreeViewState;

// ---------------------------------------------------------------------------
// A loopback apiserver
// ---------------------------------------------------------------------------

/// The context name every fake cluster answers to. Deliberately unlikely to
/// exist in a developer's real kubeconfig, so a `kubectl` fallback that does
/// get spawned can only fail.
const FAKE_CONTEXT: &str = "srelens-fake-apiserver-ctx";

/// One canned reply: any request whose path starts with `prefix` gets
/// `status` and `body`. Routes are matched in the order they are given, so a
/// more specific prefix must come first.
#[derive(Clone)]
struct Route {
    prefix: String,
    status: u16,
    body: String,
}

fn route(prefix: &str, body: Value) -> Route {
    Route {
        prefix: prefix.to_string(),
        status: 200,
        body: body.to_string(),
    }
}

fn failing_route(prefix: &str, status: u16) -> Route {
    Route {
        prefix: prefix.to_string(),
        status,
        body: json!({
            "kind": "Status",
            "apiVersion": "v1",
            "metadata": {},
            "status": "Failure",
            "message": "fake apiserver says no",
            "reason": "InternalError",
            "code": status,
        })
        .to_string(),
    }
}

/// A running fake apiserver plus the kubeconfig that points at it. Dropping it
/// removes the kubeconfig; the serving task dies with the test's runtime.
struct FakeCluster {
    _dir: tempfile::TempDir,
    kubeconfig: PathBuf,
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

/// Serve exactly one request on `sock` and close. Closing after every reply
/// keeps the server single-threaded-simple; hyper just opens a new connection.
async fn serve_one(mut sock: tokio::net::TcpStream, routes: Arc<Vec<Route>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = find_head_end(&buf) {
            break end;
        }
        match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .split('?')
        .next()
        .unwrap_or("/")
        .to_string();
    let content_length: usize = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);

    // Drain the request body so the client never sees a reset mid-write.
    while buf.len() < head_end + content_length {
        match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }

    let (status, body) = routes
        .iter()
        .find(|r| path.starts_with(&r.prefix))
        .map(|r| (r.status, r.body.clone()))
        .unwrap_or_else(|| {
            let miss = failing_route("", 404);
            (404, miss.body)
        });

    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        status_text(status),
        body.len(),
        body
    );
    let _ = sock.write_all(response.as_bytes()).await;
    let _ = sock.flush().await;
    let _ = sock.shutdown().await;
}

/// Start a loopback apiserver serving `routes` and write a kubeconfig whose
/// `test-cluster` context points at it.
async fn fake_cluster(routes: Vec<Route>) -> FakeCluster {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let port = listener.local_addr().expect("bound address").port();
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((sock, _)) => {
                    let routes = routes.clone();
                    tokio::spawn(serve_one(sock, routes));
                }
                Err(_) => break,
            }
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let kubeconfig = dir.path().join("config");
    std::fs::write(
        &kubeconfig,
        format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: {FAKE_CONTEXT}\n\
             clusters:\n- name: fake\n  cluster:\n    server: http://127.0.0.1:{port}\n\
             contexts:\n- name: {FAKE_CONTEXT}\n  context:\n    cluster: fake\n    user: fake-user\n    namespace: default\n\
             users:\n- name: fake-user\n  user: {{}}\n"
        ),
    )
    .expect("write kubeconfig");

    FakeCluster {
        _dir: dir,
        kubeconfig,
    }
}

/// An app whose *client cache* talks to `cluster`. `app.kubeconfig_paths` is
/// left empty on purpose: watches and the kubectl fallbacks resolve nothing,
/// so only the code paths under test reach the fake server.
async fn app_on(cluster: &FakeCluster) -> (App, UnboundedReceiver<AppEvent>) {
    let (mut app, rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.client_cache
        .set_paths(vec![cluster.kubeconfig.clone()])
        .await;
    // `kubectl` fallbacks must never resolve: point KUBECONFIG at a file that
    // does not exist so any spawned kubectl fails immediately, on every OS and
    // whether or not kubectl is installed.
    app.kubeconfig_paths = vec![cluster.kubeconfig.with_file_name("no-such-kubeconfig")];
    (app, rx)
}

/// Wait (bounded) for the `ActionResult` carrying `title`.
async fn action_result(
    rx: &mut UnboundedReceiver<AppEvent>,
    title: &str,
) -> Result<String, String> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match rx.recv().await {
                Some(AppEvent::ActionResult { title: t, result }) if t == title => return result,
                Some(_) => continue,
                None => panic!("event channel closed before '{}'", title),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no '{}' event within 20s", title))
}

fn toast(app: &App) -> String {
    app.toast
        .as_ref()
        .map(|(m, _, _)| m.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Canned cluster payloads
// ---------------------------------------------------------------------------

fn version_body() -> Value {
    json!({
        "major": "1",
        "minor": "31",
        "gitVersion": "v1.31.7",
        "gitCommit": "deadbeef",
        "gitTreeState": "clean",
        "buildDate": "2026-01-01T00:00:00Z",
        "goVersion": "go1.23",
        "compiler": "gc",
        "platform": "linux/amd64",
    })
}

fn node(name: &str, ready: bool, extra_allocatable: Value) -> Value {
    let mut allocatable = json!({ "cpu": "4", "memory": "8Gi", "pods": "110" });
    if let (Some(dst), Some(src)) = (allocatable.as_object_mut(), extra_allocatable.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    json!({
        "apiVersion": "v1",
        "kind": "Node",
        "metadata": { "name": name, "creationTimestamp": "2026-01-01T00:00:00Z" },
        "spec": {},
        "status": {
            "capacity": { "cpu": "4", "memory": "8Gi", "pods": "110" },
            "allocatable": allocatable,
            "conditions": [{
                "type": "Ready",
                "status": if ready { "True" } else { "False" },
                "lastHeartbeatTime": "2026-01-01T00:00:00Z",
                "lastTransitionTime": "2026-01-01T00:00:00Z",
            }],
            "nodeInfo": {
                "architecture": "amd64",
                "bootID": "b",
                "containerRuntimeVersion": "containerd://1.7",
                "kernelVersion": "6.1",
                "kubeProxyVersion": "v1.31.7",
                "kubeletVersion": "v1.31.7",
                "machineID": "m",
                "operatingSystem": "linux",
                "osImage": "Ubuntu",
                "systemUUID": "u",
            },
        },
    })
}

fn node_list(items: Vec<Value>) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "NodeList",
        "metadata": { "resourceVersion": "1" },
        "items": items,
    })
}

/// A pod whose single container requests `requests`, in `phase`, optionally
/// with a container status that makes it unhealthy.
fn pod(name: &str, phase: &str, requests: Value, container_state: Option<Value>) -> Value {
    let mut status = json!({ "phase": phase });
    if let Some(state) = container_state {
        status["containerStatuses"] = json!([{
            "name": "app",
            "ready": false,
            "restartCount": 0,
            "image": "busybox",
            "imageID": "busybox@sha256:0",
            "state": state,
        }]);
    }
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": name,
            "namespace": "default",
            "creationTimestamp": "2026-01-01T00:00:00Z",
        },
        "spec": {
            "containers": [{
                "name": "app",
                "image": "busybox",
                "resources": { "requests": requests },
            }],
            "initContainers": [{
                "name": "init",
                "image": "busybox",
                "resources": { "requests": { "cpu": "10m", "memory": "16Mi" } },
            }],
        },
        "status": status,
    })
}

fn pod_list(items: Vec<Value>) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "PodList",
        "metadata": { "resourceVersion": "1" },
        "items": items,
    })
}

// ---------------------------------------------------------------------------
// refresh_cluster_info
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reachable_apiserver_reports_its_version_and_node_and_pod_counts() {
    let cluster = fake_cluster(vec![
        route("/version", version_body()),
        route(
            "/api/v1/nodes",
            node_list(vec![node("a", true, json!({})), node("b", true, json!({}))]),
        ),
        route(
            "/api/v1/pods",
            pod_list(vec![
                pod("p1", "Running", json!({}), None),
                pod("p2", "Running", json!({}), None),
                pod("p3", "Running", json!({}), None),
            ]),
        ),
    ])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.refresh_cluster_info();
    let payload = action_result(&mut rx, "cluster_info_updated")
        .await
        .expect("a reachable apiserver reports Ok");
    assert_eq!(payload, "v1.31.7|2|3");

    app.handle_cluster_info_update(&payload);
    assert!(app.is_connected);
    assert_eq!(app.cluster_version, "v1.31.7");
    assert_eq!((app.node_count, app.pod_count), (2, 3));
}

#[tokio::test]
async fn a_version_call_that_fails_reports_the_apiserver_error_not_a_count() {
    let cluster = fake_cluster(vec![failing_route("/version", 500)]).await;
    let (app, mut rx) = app_on(&cluster).await;

    app.refresh_cluster_info();
    let err = action_result(&mut rx, "cluster_info_failed")
        .await
        .expect_err("a 500 from /version is a failure");
    assert!(
        err.contains("500") || err.to_lowercase().contains("internal"),
        "the apiserver's own error is surfaced: {err}"
    );
}

#[tokio::test]
async fn unlistable_nodes_and_pods_still_report_a_version_with_zero_counts() {
    // Only /version answers; the two metadata lists 404, and each falls back
    // to zero rather than failing the whole refresh.
    let cluster = fake_cluster(vec![route("/version", version_body())]).await;
    let (app, mut rx) = app_on(&cluster).await;

    app.refresh_cluster_info();
    let payload = action_result(&mut rx, "cluster_info_updated")
        .await
        .expect("version alone is enough");
    assert_eq!(payload, "v1.31.7|0|0");
}

// ---------------------------------------------------------------------------
// refresh_crds
// ---------------------------------------------------------------------------

fn crd_object(name: &str, spec: Value) -> Value {
    json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": { "name": name },
        "spec": spec,
    })
}

#[tokio::test]
async fn crd_discovery_picks_the_storage_version_its_columns_and_skips_groupless_definitions() {
    let cluster = fake_cluster(vec![route(
        "/apis/apiextensions.k8s.io/v1/customresourcedefinitions",
        json!({
            "apiVersion": "apiextensions.k8s.io/v1",
            "kind": "CustomResourceDefinitionList",
            "metadata": { "resourceVersion": "1" },
            "items": [
                // Storage version wins over the older served one, and brings
                // its own printer columns.
                crd_object("widgets.example.com", json!({
                    "group": "example.com",
                    "scope": "Namespaced",
                    "names": { "kind": "Widget", "plural": "widgets", "singular": "widget", "shortNames": ["wd"] },
                    "versions": [
                        { "name": "v1beta1", "served": true, "storage": false },
                        { "name": "v1", "served": true, "storage": true, "additionalPrinterColumns": [
                            { "name": "Size", "jsonPath": ".spec.size", "type": "integer", "priority": 1, "description": "how big" }
                        ] }
                    ],
                })),
                // No storage version at all: the first served one is used, and
                // the columns come from the spec-level fallback.
                crd_object("gadgets.example.com", json!({
                    "group": "example.com",
                    "scope": "Cluster",
                    "names": { "kind": "Gadget", "plural": "gadgets", "singular": "gadget" },
                    "additionalPrinterColumns": [
                        { "name": "Phase", "jsonPath": ".status.phase", "type": "string" }
                    ],
                    "versions": [ { "name": "v2", "served": true, "storage": false } ],
                })),
                // No versions block: falls back to v1 with no columns.
                crd_object("sprockets.example.com", json!({
                    "group": "example.com",
                    "scope": "Namespaced",
                    "names": { "kind": "Sprocket", "plural": "sprockets", "singular": "sprocket" },
                })),
                // No group: not a usable CRD, so it is dropped.
                crd_object("broken", json!({
                    "scope": "Namespaced",
                    "names": { "kind": "Broken", "plural": "brokens", "singular": "broken" },
                })),
            ],
        }),
    )])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.refresh_crds();
    let payload = action_result(&mut rx, "crds_updated")
        .await
        .expect("discovery serialises its findings");
    app.handle_crds_update(&payload);

    let names: Vec<&str> = app.crds.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(names, vec!["Widget", "Gadget", "Sprocket"]);

    let widget = &app.crds[0];
    assert_eq!(widget.crd_name, "widgets.example.com");
    assert_eq!(widget.version, "v1", "the storage version wins");
    assert!(widget.namespaced);
    assert_eq!(widget.short_names, vec!["wd".to_string()]);
    assert_eq!(widget.printer_columns.len(), 1);
    assert_eq!(widget.printer_columns[0].name, "Size");
    assert_eq!(widget.printer_columns[0].json_path, ".spec.size");
    assert_eq!(widget.printer_columns[0].priority, 1);
    assert_eq!(
        widget.printer_columns[0].description.as_deref(),
        Some("how big")
    );

    let gadget = &app.crds[1];
    assert_eq!(
        gadget.version, "v2",
        "no storage version, so the served one"
    );
    assert!(!gadget.namespaced);
    assert_eq!(
        gadget.printer_columns[0].name, "Phase",
        "columns fall back to the spec-level list"
    );

    let sprocket = &app.crds[2];
    assert_eq!(sprocket.version, "v1", "no versions block means v1");
    assert!(sprocket.printer_columns.is_empty());
}

#[tokio::test]
async fn crd_discovery_stays_silent_when_the_definitions_cannot_be_listed() {
    let cluster = fake_cluster(vec![failing_route("/apis/apiextensions.k8s.io", 404)]).await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.refresh_crds();
    tokio::task::yield_now().await;
    app.handle_crds_update(&json!([]).to_string());
    assert!(app.crds.is_empty());
    assert!(
        common::drain(&mut rx).iter().all(
            |ev| !matches!(ev, AppEvent::ActionResult { title, .. } if title == "crds_updated")
        ),
        "a failed list publishes no CRD update"
    );
}

// ---------------------------------------------------------------------------
// refresh_cluster_overview
// ---------------------------------------------------------------------------

/// Two nodes whose allocatable maps exercise every GPU key shape and every
/// suffix `parse_gpu_mem_mib` understands.
fn gpu_nodes() -> Value {
    node_list(vec![
        node(
            "gpu-a",
            true,
            json!({
                "nvidia.com/gpu": "2",
                "example.com/gpu-mem-ki": "2048Ki",
                "example.com/gpu-mem-mi": "512Mi",
            }),
        ),
        node(
            "gpu-b",
            false,
            json!({
                "amd.com/gpu": "1",
                "example.com/gpu-mem-gi": "2Gi",
                "example.com/gpu-vram-ti": "1Ti",
                "example.com/gpu-mem-bytes": "34359738368",
                "example.com/gpu-mem-plain": "512",
                "example.com/gpu-mem-nonsense": "abc",
                "example.com/vgpu": "4",
                "nvidia.com/gpu-cores": "8",
            }),
        ),
    ])
}

/// 2Ki + 512Mi + 2Gi + 1Ti + 32GiB-as-bytes + a bare 512, all in MiB. The
/// unparseable value contributes nothing.
const EXPECTED_GPU_MEM_MIB: i64 = 2 + 512 + 2048 + 1_048_576 + 32_768 + 512;

fn overview_pods() -> Value {
    pod_list(vec![
        pod(
            "p-running",
            "Running",
            json!({
                "cpu": "250m",
                "memory": "512Mi",
                "nvidia.com/gpu": "1",
                "example.com/gpu-mem": "256Mi",
            }),
            None,
        ),
        pod("p-pending", "Pending", json!({}), None),
        pod("p-failed", "Failed", json!({}), None),
        pod(
            "p-crashing",
            "Running",
            json!({}),
            Some(json!({ "waiting": { "reason": "CrashLoopBackOff" } })),
        ),
        pod(
            "p-oomkilled",
            "Running",
            json!({}),
            Some(json!({ "terminated": { "exitCode": 137, "reason": "OOMKilled" } })),
        ),
    ])
}

#[tokio::test]
async fn the_cluster_overview_rolls_up_nodes_pods_gpus_and_metrics_server_usage() {
    let cluster = fake_cluster(vec![
        route("/version", version_body()),
        route("/api/v1/nodes", gpu_nodes()),
        route(
            "/apis/metrics.k8s.io/v1beta1/",
            json!({
                "apiVersion": "metrics.k8s.io/v1beta1",
                "kind": "NodeMetricsList",
                "metadata": { "resourceVersion": "1" },
                "items": [
                    { "metadata": { "name": "gpu-a" }, "usage": { "cpu": "1500m", "memory": "2048Mi" } },
                    { "metadata": { "name": "gpu-b" }, "usage": { "cpu": "1500m", "memory": "2048Mi" } },
                ],
            }),
        ),
        route("/api/v1/pods", overview_pods()),
    ])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.refresh_cluster_overview();
    let payload = action_result(&mut rx, "cluster_overview_updated")
        .await
        .expect("the roll-up serialises");
    app.handle_cluster_overview_update(&payload);
    let data = app
        .cluster_overview_data
        .as_ref()
        .expect("the overview is stored");

    assert!(data.is_reachable);
    assert_eq!(data.k8s_version, "v1.31.7");
    assert_eq!(data.node_count, 2);
    assert_eq!(data.ready_nodes, 1, "only one node reports Ready=True");
    assert_eq!(data.total_cpu_millicores, 8_000);
    assert_eq!(data.total_mem_mib, 16_384);
    assert_eq!(data.total_gpus, 3, "nvidia + amd, ignoring vgpu and cores");
    assert_eq!(data.total_gpu_mem_mib, EXPECTED_GPU_MEM_MIB);

    assert_eq!(data.total_pods, 5);
    assert_eq!(data.running_pods, 1);
    assert_eq!(data.pending_pods, 1);
    assert_eq!(
        data.failed_pods, 3,
        "phase Failed plus the crash-looping and OOMKilled pods"
    );
    assert_eq!(data.allocated_gpus, 1);
    assert_eq!(data.used_gpu_mem_mib, 256);

    assert_eq!(
        data.used_cpu_millicores, 3_000,
        "metrics-server usage wins over pod requests"
    );
    assert_eq!(data.used_mem_mib, 4_096);
}

#[tokio::test]
async fn the_cluster_overview_falls_back_to_pod_requests_without_a_metrics_server() {
    let cluster = fake_cluster(vec![
        route("/version", version_body()),
        route("/api/v1/nodes", gpu_nodes()),
        failing_route("/apis/metrics.k8s.io", 404),
        route("/api/v1/pods", overview_pods()),
    ])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.refresh_cluster_overview();
    let payload = action_result(&mut rx, "cluster_overview_updated")
        .await
        .expect("the roll-up serialises");
    app.handle_cluster_overview_update(&payload);
    let data = app.cluster_overview_data.as_ref().expect("overview");

    // Every pod carries a 10m/16Mi init container; only p-running asks for more.
    assert_eq!(data.used_cpu_millicores, 5 * 10 + 250);
    assert_eq!(data.used_mem_mib, 5 * 16 + 512);
}

#[tokio::test]
async fn an_unreachable_apiserver_still_publishes_an_overview_marked_unreachable() {
    // No /version route: the apiserver answers 404 and the roll-up records an
    // unknown version rather than giving up.
    let cluster = fake_cluster(vec![
        route("/api/v1/nodes", node_list(vec![node("n", true, json!({}))])),
        route("/api/v1/pods", pod_list(vec![])),
    ])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.refresh_cluster_overview();
    let payload = action_result(&mut rx, "cluster_overview_updated")
        .await
        .expect("the roll-up still publishes");
    app.handle_cluster_overview_update(&payload);
    let data = app.cluster_overview_data.as_ref().expect("overview");
    assert!(!data.is_reachable);
    assert_eq!(data.k8s_version, "unknown");
    assert_eq!(data.node_count, 1);
    assert!(!app.is_connected);
}

// ---------------------------------------------------------------------------
// Custom resource instances
// ---------------------------------------------------------------------------

fn widget_crd() -> CrdMeta {
    CrdMeta {
        crd_name: "widgets.example.com".to_string(),
        group: "example.com".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
        singular: "widget".to_string(),
        namespaced: true,
        short_names: vec![],
        printer_columns: vec![],
    }
}

fn gadget_crd() -> CrdMeta {
    CrdMeta {
        namespaced: false,
        crd_name: "gadgets.example.com".to_string(),
        kind: "Gadget".to_string(),
        plural: "gadgets".to_string(),
        singular: "gadget".to_string(),
        ..widget_crd()
    }
}

#[tokio::test]
async fn custom_resource_instances_are_published_with_name_namespace_and_age() {
    let cluster = fake_cluster(vec![
        route(
            "/apis/example.com/v1/namespaces/default/widgets",
            json!({
                "apiVersion": "example.com/v1",
                "kind": "WidgetList",
                "metadata": { "resourceVersion": "1" },
                "items": [{
                    "apiVersion": "example.com/v1",
                    "kind": "Widget",
                    "metadata": {
                        "name": "w-1",
                        "namespace": "default",
                        "creationTimestamp": "2026-01-01T00:00:00Z",
                    },
                    "spec": { "size": 3 },
                }],
            }),
        ),
        route(
            "/apis/example.com/v1/gadgets",
            json!({
                "apiVersion": "example.com/v1",
                "kind": "GadgetList",
                "metadata": { "resourceVersion": "1" },
                "items": [{
                    "metadata": { "name": "g-1" },
                    "spec": { "size": 9 },
                }],
            }),
        ),
    ])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.fetch_crd_instances(widget_crd());
    let payload = action_result(&mut rx, "crd_instances:Widget")
        .await
        .expect("the namespaced list is published");
    let items: Vec<Value> = serde_json::from_str(&payload).expect("a JSON array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "w-1");
    assert_eq!(items[0]["namespace"], "default");
    assert_eq!(items[0]["spec"]["size"], 3);
    assert_eq!(
        items[0]["metadata"]["name"], "w-1",
        "the object metadata is folded back in"
    );
    assert!(
        items[0]["age"].is_string() && items[0]["createdAt"].is_string(),
        "creation time becomes an age and an ISO stamp: {}",
        items[0]
    );

    app.fetch_crd_instances(gadget_crd());
    let payload = action_result(&mut rx, "crd_instances:Gadget")
        .await
        .expect("the cluster-scoped list is published");
    let items: Vec<Value> = serde_json::from_str(&payload).expect("a JSON array");
    assert_eq!(items[0]["name"], "g-1");
    assert!(
        items[0].get("namespace").is_none(),
        "a cluster-scoped object gets no namespace"
    );
    assert!(
        items[0].get("age").is_none(),
        "no creation timestamp means no age"
    );

    // Feeding a payload back through the handler fills the CRD table.
    app.switch_view_to_crd(widget_crd()).await;
    app.handle_crd_instances_update(
        "crd_instances:Widget",
        &json!([{ "name": "w-1", "namespace": "default" }]).to_string(),
    );
    match &app.active_view {
        ActiveView::Table(t) => assert_eq!(t.raw_items.len(), 1),
        _ => panic!("expected the widget table"),
    }
}

// ---------------------------------------------------------------------------
// Pod containers from the API
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pod_containers_come_from_the_api_when_the_informer_cache_has_nothing() {
    let cluster = fake_cluster(vec![route(
        "/api/v1/namespaces/default/pods/web-0",
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": { "name": "web-0", "namespace": "default" },
            "spec": {
                "containers": [
                    { "name": "app", "image": "busybox" },
                    { "name": "sidecar", "image": "busybox" }
                ],
                "initContainers": [{ "name": "init", "image": "busybox" }],
                "ephemeralContainers": [{ "name": "debugger", "image": "busybox" }],
            },
        }),
    )])
    .await;
    let (app, _rx) = app_on(&cluster).await;

    let containers = app.get_pod_containers("web-0", Some("default")).await;
    assert_eq!(containers, vec!["app", "sidecar", "init", "debugger"]);

    assert!(
        app.get_pod_containers("missing", Some("default"))
            .await
            .is_empty(),
        "a pod the apiserver does not know has no containers"
    );
}

// ---------------------------------------------------------------------------
// Live YAML
// ---------------------------------------------------------------------------

fn yaml_text(app: &App) -> String {
    match &app.active_view {
        ActiveView::Yaml(y) => y.yaml_content.clone(),
        other => panic!(
            "expected the YAML view, got {:?}",
            std::mem::discriminant(other)
        ),
    }
}

fn describe_text(app: &App) -> String {
    match &app.active_view {
        ActiveView::Describe(d) => d.content.clone(),
        other => panic!(
            "expected the describe view, got {:?}",
            std::mem::discriminant(other)
        ),
    }
}

#[tokio::test]
async fn the_yaml_view_serialises_the_live_object_for_builtin_and_custom_kinds() {
    let cluster = fake_cluster(vec![
        route(
            "/api/v1/namespaces/default/pods/web-0",
            json!({
                "apiVersion": "v1",
                "kind": "Pod",
                "metadata": {
                    "name": "web-0",
                    "namespace": "default",
                    "managedFields": [{ "manager": "kubectl", "operation": "Apply" }],
                },
                "spec": { "containers": [{ "name": "app", "image": "busybox" }] },
            }),
        ),
        route(
            "/api/v1/nodes/node-a",
            json!({
                "apiVersion": "v1",
                "kind": "Node",
                "metadata": { "name": "node-a" },
                "spec": {},
            }),
        ),
        route(
            "/apis/example.com/v1/namespaces/default/widgets/w-1",
            json!({
                "apiVersion": "example.com/v1",
                "kind": "Widget",
                "metadata": { "name": "w-1", "namespace": "default" },
                "spec": { "size": 3 },
            }),
        ),
        route(
            "/apis/example.com/v1/gadgets/g-1",
            json!({
                "apiVersion": "example.com/v1",
                "kind": "Gadget",
                "metadata": { "name": "g-1" },
                "spec": { "size": 9 },
            }),
        ),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;
    app.crds = vec![widget_crd(), gadget_crd()];

    app.open_yaml_view("web-0".into(), "Pod".into(), Some("default".into()))
        .await;
    let text = yaml_text(&app);
    assert!(text.contains("name: web-0"), "{text}");
    assert!(
        !text.contains("managedFields"),
        "server-side apply bookkeeping is stripped:\n{text}"
    );
    assert_eq!(app.nav_stack.len(), 1);

    app.open_yaml_view("node-a".into(), "Node".into(), None)
        .await;
    assert!(
        yaml_text(&app).contains("name: node-a"),
        "a cluster-scoped builtin is fetched without a namespace"
    );

    app.open_yaml_view("w-1".into(), "Widget".into(), Some("default".into()))
        .await;
    let text = yaml_text(&app);
    assert!(text.contains("name: w-1"), "{text}");
    assert!(text.contains("size: 3"), "{text}");

    app.open_yaml_view("g-1".into(), "gadgets".into(), None)
        .await;
    assert!(
        yaml_text(&app).contains("name: g-1"),
        "a CRD is matched by its plural too"
    );
    assert_eq!(app.nav_stack.len(), 4, "each view stacks on the last");
}

#[tokio::test]
async fn the_yaml_view_reports_the_kubectl_fallback_failing_when_nothing_can_serve_the_manifest() {
    // No cluster and a KUBECONFIG that does not exist: the API path fails,
    // the kubectl fallback fails, and the view shows the error manifest.
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig-a"), PathBuf::from("b")];

    app.open_yaml_view("web-0".into(), "Pod".into(), Some("default".into()))
        .await;
    assert_eq!(
        yaml_text(&app),
        "# Error: Unable to fetch live manifest for Pod/web-0 in namespace default\n"
    );

    // An explicitly empty namespace is not passed to kubectl as `-n`, and it
    // is reported verbatim rather than as "default".
    app.open_yaml_view("cm-1".into(), "ConfigMap".into(), Some(String::new()))
        .await;
    assert_eq!(
        yaml_text(&app),
        "# Error: Unable to fetch live manifest for ConfigMap/cm-1 in namespace \n"
    );

    // No namespace and no context at all still produces the error manifest.
    app.active_context = String::new();
    app.kubeconfig_paths.clear();
    app.open_yaml_view("node-a".into(), "Node".into(), None)
        .await;
    assert_eq!(
        yaml_text(&app),
        "# Error: Unable to fetch live manifest for Node/node-a in namespace default\n"
    );
}

// ---------------------------------------------------------------------------
// Describe
// ---------------------------------------------------------------------------

#[tokio::test]
async fn describing_a_service_renders_its_metadata_spec_and_events() {
    let cluster = fake_cluster(vec![
        route(
            "/api/v1/namespaces/default/services/web",
            json!({
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": {
                    "name": "web",
                    "namespace": "default",
                    "labels": { "app": "web", "tier": "front" },
                    "annotations": {
                        "owner": "team-a",
                        "note": "second",
                        "example.com/managed-fields-hint": "skipped",
                    },
                },
                "spec": {
                    "selector": { "app": "web" },
                    "type": "ClusterIP",
                    "clusterIP": "10.0.0.9",
                    "ports": [
                        { "name": "http", "port": 80, "protocol": "TCP", "targetPort": 8080 },
                        { "port": 443 },
                    ],
                },
            }),
        ),
        route(
            "/api/v1/namespaces/default/events",
            json!({
                "apiVersion": "v1",
                "kind": "EventList",
                "metadata": { "resourceVersion": "1" },
                "items": [{
                    "metadata": { "name": "e1", "namespace": "default" },
                    "involvedObject": { "kind": "Service", "name": "web" },
                    "type": "Warning",
                    "reason": "NoEndpoints",
                    "message": "no backends",
                    "source": { "component": "endpoint-controller" },
                    "lastTimestamp": "2026-01-01T00:00:00Z",
                }],
            }),
        ),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;

    app.open_describe_view("web".into(), "Service".into(), Some("default".into()))
        .await;
    let text = describe_text(&app);
    assert!(text.contains("Name:                     web"), "{text}");
    assert!(text.contains("Namespace:                default"), "{text}");
    assert!(text.contains("app=web"), "{text}");
    assert!(text.contains("tier=front"), "labels are listed:\n{text}");
    assert!(text.contains("owner: team-a"), "{text}");
    assert!(
        !text.contains("skipped"),
        "managed-fields annotations are dropped:\n{text}"
    );
    assert!(text.contains("Selector:"), "{text}");
    assert!(text.contains("ClusterIP"), "{text}");
    assert!(text.contains("10.0.0.9"), "{text}");
    assert!(text.contains("http  80/TCP"), "{text}");
    assert!(text.contains("TargetPort:"), "{text}");
    assert!(
        text.contains("443/TCP"),
        "a port without a name still shows"
    );
    assert!(text.contains("NoEndpoints"), "{text}");
    assert!(text.contains("endpoint-controller"), "{text}");
    assert_eq!(app.nav_stack.len(), 1);
}

#[tokio::test]
async fn describing_a_cluster_scoped_object_without_events_says_none() {
    let cluster = fake_cluster(vec![
        route(
            "/api/v1/nodes/node-a",
            json!({
                "apiVersion": "v1",
                "kind": "Node",
                "metadata": { "name": "node-a" },
                "spec": {},
            }),
        ),
        route(
            "/api/v1/events",
            json!({
                "apiVersion": "v1",
                "kind": "EventList",
                "metadata": { "resourceVersion": "1" },
                "items": [],
            }),
        ),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;

    app.open_describe_view("node-a".into(), "Node".into(), None)
        .await;
    let text = describe_text(&app);
    assert!(text.contains("Name:                     node-a"), "{text}");
    assert!(
        !text.contains("Namespace:"),
        "a cluster-scoped object has no namespace line:\n{text}"
    );
    assert!(text.contains("Events:"), "{text}");
    assert!(text.contains("<none>"), "{text}");
}

#[tokio::test]
async fn describing_a_resource_whose_events_cannot_be_listed_still_renders_the_object() {
    let cluster = fake_cluster(vec![
        route(
            "/api/v1/namespaces/default/configmaps/settings",
            json!({
                "apiVersion": "v1",
                "kind": "ConfigMap",
                "metadata": { "name": "settings", "namespace": "default" },
                "data": { "key": "value" },
            }),
        ),
        failing_route("/api/v1/namespaces/default/events", 500),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;

    app.open_describe_view(
        "settings".into(),
        "ConfigMap".into(),
        Some("default".into()),
    )
    .await;
    let text = describe_text(&app);
    assert!(
        text.contains("Name:                     settings"),
        "{text}"
    );
    assert!(
        text.contains("Events:") && text.contains("<none>"),
        "an unlistable event feed reads as none:\n{text}"
    );
}

#[tokio::test]
async fn describing_something_no_backend_can_reach_reports_the_failure() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig")];

    app.open_describe_view("web-0".into(), "Pod".into(), Some("default".into()))
        .await;
    assert_eq!(
        describe_text(&app),
        "Error: Unable to describe Pod/web-0 in namespace default\n"
    );

    app.active_context = String::new();
    app.kubeconfig_paths.clear();
    app.open_describe_view("web-0".into(), "Pod".into(), Some(String::new()))
        .await;
    assert_eq!(
        describe_text(&app),
        "Error: Unable to describe Pod/web-0 in namespace \n",
        "an explicitly empty namespace stays empty in the message"
    );
}

// ---------------------------------------------------------------------------
// Destructive confirmations: delete, restart, scale
// ---------------------------------------------------------------------------

fn pods_table(app: &mut App, names: &[&str]) {
    let mut table = ResourceTableState::new(ResourceKind::Pods);
    table.set_items(
        names
            .iter()
            .map(|n| json!({ "name": n, "namespace": "default", "status": "Running" }))
            .collect(),
        "",
    );
    table.is_loading = false;
    app.active_view = ActiveView::Table(table);
}

fn table_names(app: &App) -> Vec<String> {
    match &app.active_view {
        ActiveView::Table(t) => t
            .raw_items
            .iter()
            .filter_map(|i| i.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect(),
        _ => panic!("expected a table"),
    }
}

#[tokio::test]
async fn confirming_a_delete_drops_the_row_and_a_rejected_delete_toasts_the_error() {
    let cluster = fake_cluster(vec![
        route(
            "/api/v1/namespaces/default/pods/web-0",
            json!({ "apiVersion": "v1", "kind": "Pod", "metadata": { "name": "web-0" } }),
        ),
        failing_route("/api/v1/namespaces/default/pods/web-1", 409),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;
    pods_table(&mut app, &["web-0", "web-1"]);

    app.execute_modal_confirm("delete:Pod:default:web-0".into())
        .await;
    assert_eq!(toast(&app), "✓ Deleted Pod 'web-0' in 'default'");
    assert_eq!(
        table_names(&app),
        vec!["web-1".to_string()],
        "the deleted row leaves the table immediately"
    );

    app.execute_modal_confirm("delete:Pod:default:web-1".into())
        .await;
    assert!(
        toast(&app).starts_with("Delete failed:"),
        "toast: {}",
        toast(&app)
    );
    assert_eq!(
        table_names(&app),
        vec!["web-1".to_string()],
        "a rejected delete leaves the row alone"
    );
}

#[tokio::test]
async fn a_delete_resolves_custom_kinds_and_refuses_ones_it_cannot_place() {
    let cluster = fake_cluster(vec![
        route(
            "/apis/example.com/v1/namespaces/default/widgets/w-1",
            json!({ "apiVersion": "example.com/v1", "kind": "Widget", "metadata": { "name": "w-1" } }),
        ),
        route(
            "/apis/example.com/v1/gadgets/g-1",
            json!({ "apiVersion": "example.com/v1", "kind": "Gadget", "metadata": { "name": "g-1" } }),
        ),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;
    app.crds = vec![widget_crd(), gadget_crd()];

    app.execute_modal_confirm("delete:Widget:default:w-1".into())
        .await;
    assert_eq!(toast(&app), "✓ Deleted Widget 'w-1' in 'default'");

    app.execute_modal_confirm("delete:gadgets::g-1".into())
        .await;
    assert_eq!(
        toast(&app),
        "✓ Deleted gadgets 'g-1' in ''",
        "a cluster-scoped custom resource needs no namespace"
    );

    app.execute_modal_confirm("delete:Nonesuch:default:x".into())
        .await;
    assert_eq!(toast(&app), "Cannot resolve resource kind 'Nonesuch'");

    app.execute_modal_confirm("delete:malformed".into()).await;
    assert_eq!(
        toast(&app),
        "Resource deleted successfully",
        "a malformed action name is treated as already done"
    );
}

#[tokio::test]
async fn a_restart_patches_the_workload_and_reports_what_it_could_not_restart() {
    let cluster = fake_cluster(vec![
        route(
            "/apis/apps/v1/namespaces/default/deployments/web",
            json!({ "apiVersion": "apps/v1", "kind": "Deployment", "metadata": { "name": "web" } }),
        ),
        failing_route("/apis/apps/v1/namespaces/default/statefulsets/db", 404),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;

    app.execute_modal_confirm("restart:Deployment:default:web".into())
        .await;
    assert_eq!(
        toast(&app),
        "✓ Rollout restart triggered for Deployment 'web'"
    );

    app.execute_modal_confirm("restart:StatefulSet:default:db".into())
        .await;
    assert!(
        toast(&app).starts_with("Restart failed:"),
        "toast: {}",
        toast(&app)
    );

    app.execute_modal_confirm("restart:Nonesuch:default:x".into())
        .await;
    assert_eq!(toast(&app), "Cannot restart resource kind 'Nonesuch'");

    app.execute_modal_confirm("restart:malformed".into()).await;
    assert_eq!(toast(&app), "Rollout restart triggered");

    app.execute_modal_confirm("stop-pf:pf-1".into()).await;
    assert_eq!(toast(&app), "Port forward stopped");
}

#[tokio::test]
async fn a_confirmation_without_a_reachable_cluster_toasts_a_connection_error() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;

    app.execute_modal_confirm("delete:Pod:default:web-0".into())
        .await;
    assert!(
        toast(&app).starts_with("Connection error:"),
        "toast: {}",
        toast(&app)
    );

    app.execute_modal_confirm("restart:Deployment:default:web".into())
        .await;
    assert!(
        toast(&app).starts_with("Connection error:"),
        "toast: {}",
        toast(&app)
    );

    app.execute_scale_workload("web".into(), 3).await;
    assert!(
        toast(&app).starts_with("Connection error:"),
        "toast: {}",
        toast(&app)
    );
}

#[tokio::test]
async fn scaling_patches_the_selected_workload_and_surfaces_a_rejection() {
    let cluster = fake_cluster(vec![
        route(
            "/apis/apps/v1/namespaces/prod/deployments/web",
            json!({ "apiVersion": "apps/v1", "kind": "Deployment", "metadata": { "name": "web" } }),
        ),
        failing_route("/apis/apps/v1/namespaces/default/deployments/ghost", 404),
    ])
    .await;
    let (mut app, _rx) = app_on(&cluster).await;

    // The namespace comes from the selected row, not the active namespace.
    let mut table = ResourceTableState::new(ResourceKind::Deployments);
    table.set_items(
        vec![json!({ "name": "web", "namespace": "prod", "ready": "1/1" })],
        "",
    );
    table.is_loading = false;
    app.active_view = ActiveView::Table(table);

    app.execute_scale_workload("web".into(), 5).await;
    assert_eq!(toast(&app), "✓ Scaled Deployments 'web' to 5 replicas");

    // Off a table the kind defaults to Deployment in the active namespace.
    app.active_view = ActiveView::Assistant;
    app.execute_scale_workload("ghost".into(), 2).await;
    assert!(
        toast(&app).starts_with("Scale failed:"),
        "toast: {}",
        toast(&app)
    );
}

// ---------------------------------------------------------------------------
// Node inspector keys
// ---------------------------------------------------------------------------

fn node_pod(name: &str, ns: &str) -> NodePodItem {
    NodePodItem {
        name: name.into(),
        namespace: ns.into(),
        phase: "Running".into(),
        ready_containers: "1/1".into(),
        restarts: 0,
        age: "1d".into(),
        cpu_requests_millicores: 100,
        mem_requests_mib: 128,
        gpu_requests: 0,
        gpu_mem_requests_mib: 0,
        pod_ip: "10.0.0.2".into(),
    }
}

fn inspector(name: &str, unschedulable: bool, with_pods: bool) -> NodeInspectorState {
    let mut state = NodeInspectorState::new(name.into());
    state.set_details(NodeInspectorDetails {
        name: name.into(),
        status: "Ready".into(),
        unschedulable,
        roles: "worker".into(),
        instance_type: "m5.large".into(),
        zone: None,
        region: None,
        nodepool: None,
        internal_ip: None,
        external_ip: None,
        os_image: "Ubuntu".into(),
        kernel_version: "6.1".into(),
        container_runtime: "containerd".into(),
        kubelet_version: "v1.31.7".into(),
        architecture: "amd64".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        cpu_capacity_millicores: 4000,
        cpu_allocatable_millicores: 3800,
        cpu_requests_millicores: 100,
        mem_capacity_mib: 8192,
        mem_allocatable_mib: 7800,
        mem_requests_mib: 128,
        pods_capacity: 110,
        pods_allocatable: 110,
        pods_count: if with_pods { 1 } else { 0 },
        has_gpu: false,
        gpu_model: None,
        gpu_driver_version: None,
        gpu_cuda_version: None,
        gpu_capacity_count: 0,
        gpu_allocatable_count: 0,
        gpu_requests_count: 0,
        gpu_memory_total_mib: None,
        gpu_memory_requests_mib: 0,
        conditions: vec![],
        taints: vec![],
        pods: if with_pods {
            vec![node_pod("api-0", "default")]
        } else {
            vec![]
        },
    });
    state
}

#[tokio::test]
async fn cordoning_and_uncordoning_a_node_patches_it_and_reports_which_way_it_went() {
    let cluster = fake_cluster(vec![route(
        "/api/v1/nodes/gpu-1",
        json!({ "apiVersion": "v1", "kind": "Node", "metadata": { "name": "gpu-1" } }),
    )])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, false));
    app.handle_key_event(common::ch('c')).await;
    common::type_str(&mut app, "confirm").await;
    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter)).await;
    assert_eq!(toast(&app), "Cordoning node 'gpu-1'...");
    assert_eq!(
        action_result(&mut rx, "cordon_node:gpu-1").await,
        Ok("✓ Cordoned node 'gpu-1'".to_string())
    );

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", true, false));
    app.handle_key_event(common::ch('c')).await;
    common::type_str(&mut app, "confirm").await;
    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter)).await;
    assert_eq!(toast(&app), "Uncordoning node 'gpu-1'...");
    assert_eq!(
        action_result(&mut rx, "cordon_node:gpu-1").await,
        Ok("✓ Uncordoned node 'gpu-1'".to_string())
    );
}

#[tokio::test]
async fn a_cordon_the_apiserver_rejects_is_reported_as_an_error() {
    let cluster = fake_cluster(vec![failing_route("/api/v1/nodes/gpu-1", 409)]).await;
    let (mut app, mut rx) = app_on(&cluster).await;

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, false));
    app.handle_key_event(common::ch('c')).await;
    common::type_str(&mut app, "confirm").await;
    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter)).await;
    let err = action_result(&mut rx, "cordon_node:gpu-1")
        .await
        .expect_err("a 409 fails the patch");
    assert!(!err.is_empty(), "the apiserver error is carried back");
}

#[tokio::test]
async fn the_node_inspector_describes_and_shows_yaml_for_the_node_or_the_highlighted_pod() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig")];

    // With pods listed, the pod-scoped keys act on the highlighted pod.
    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, true));
    app.handle_key_event(common::ch('d')).await;
    match &app.active_view {
        ActiveView::Describe(d) => {
            assert_eq!(d.resource_name, "api-0");
            assert_eq!(d.resource_kind, "Pod");
        }
        _ => panic!("expected a describe view for the highlighted pod"),
    }

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, true));
    app.handle_key_event(common::ch('y')).await;
    match &app.active_view {
        ActiveView::Yaml(y) => assert_eq!(y.resource_name, "api-0"),
        _ => panic!("expected the pod's YAML"),
    }

    // The upper-case variants always target the node itself.
    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, true));
    app.handle_key_event(common::ch('D')).await;
    match &app.active_view {
        ActiveView::Describe(d) => assert_eq!(d.resource_kind, "Node"),
        _ => panic!("expected the node's describe"),
    }

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, true));
    app.handle_key_event(common::ch('Y')).await;
    match &app.active_view {
        ActiveView::Yaml(y) => {
            assert_eq!(y.resource_name, "gpu-1");
            assert_eq!(y.resource_kind, "Node");
        }
        _ => panic!("expected the node's YAML"),
    }

    // With no pods at all the lower-case keys fall back to the node.
    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, false));
    app.handle_key_event(common::ch('d')).await;
    match &app.active_view {
        ActiveView::Describe(d) => assert_eq!(d.resource_name, "gpu-1"),
        _ => panic!("expected the node's describe"),
    }

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, false));
    app.handle_key_event(common::ch('y')).await;
    match &app.active_view {
        ActiveView::Yaml(y) => assert_eq!(y.resource_name, "gpu-1"),
        _ => panic!("expected the node's YAML"),
    }
}

#[tokio::test]
async fn the_node_inspector_offers_a_debug_shell_command_and_an_action_palette() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, true));
    app.handle_key_event(common::ch('s')).await;
    assert_eq!(
        toast(&app),
        "Node debug command: kubectl debug node/gpu-1 -it --image=busybox"
    );

    app.handle_key_event(common::ch('x')).await;
    match &app.modal {
        Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            ..
        }) => {
            assert_eq!(resource_kind, "Pod");
            assert_eq!(resource_name, "api-0", "the palette targets the pod");
        }
        other => panic!("expected the action palette, got {:?}", other),
    }
    app.modal = None;

    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, false));
    app.handle_key_event(common::ch('x')).await;
    match &app.modal {
        Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            ..
        }) => {
            assert_eq!(resource_kind, "Node");
            assert_eq!(resource_name, "gpu-1");
        }
        other => panic!("expected the action palette, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Table keys that open a resource
// ---------------------------------------------------------------------------

#[tokio::test]
async fn table_keys_open_yaml_describe_and_the_editor_for_the_selected_row() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig")];

    pods_table(&mut app, &["web-0"]);
    app.handle_key_event(common::ch('y')).await;
    match &app.active_view {
        ActiveView::Yaml(y) => {
            assert_eq!(y.resource_name, "web-0");
            assert_eq!(y.resource_kind, "Pod");
        }
        _ => panic!("expected the YAML view"),
    }

    pods_table(&mut app, &["web-0"]);
    app.handle_key_event(common::ch('d')).await;
    match &app.active_view {
        ActiveView::Describe(d) => assert_eq!(d.resource_name, "web-0"),
        _ => panic!("expected the describe view"),
    }

    pods_table(&mut app, &["web-0"]);
    app.handle_key_event(common::ch('e')).await;
    assert!(matches!(app.active_view, ActiveView::Yaml(_)));
    assert!(
        matches!(app.requires_terminal_suspend, Some(SuspendAction::EditYaml)),
        "'e' opens the manifest and asks the shell to hand over the terminal"
    );
}

#[tokio::test]
async fn on_an_events_table_yaml_and_describe_follow_the_involved_object() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig")];

    let events = vec![json!({
        "name": "e1",
        "namespace": "default",
        "reason": "BackOff",
        "type": "Warning",
        "object": "Pod/web-1",
        "age": "2m",
        "message": "restarting",
    })];
    let events_table = |app: &mut App| {
        let mut table = ResourceTableState::new(ResourceKind::Events);
        table.set_items(events.clone(), "");
        table.is_loading = false;
        app.active_view = ActiveView::Table(table);
    };

    events_table(&mut app);
    app.handle_key_event(common::ch('y')).await;
    match &app.active_view {
        ActiveView::Yaml(y) => {
            assert_eq!(y.resource_name, "web-1");
            assert_eq!(y.resource_kind, "Pod");
        }
        _ => panic!("expected the involved object's YAML"),
    }

    events_table(&mut app);
    app.handle_key_event(common::ch('d')).await;
    match &app.active_view {
        ActiveView::Describe(d) => {
            assert_eq!(d.resource_name, "web-1");
            assert_eq!(d.resource_kind, "Pod");
        }
        _ => panic!("expected the involved object's describe"),
    }

    events_table(&mut app);
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied event message to clipboard");
}

// ---------------------------------------------------------------------------
// Tree, port-forward, Helm and toolbox keys
// ---------------------------------------------------------------------------

fn tree_view() -> TreeViewState {
    let mut root = LineageNode::new(
        "Deployment",
        "web",
        Some("default".to_string()),
        LineageRelation::Target,
    );
    root.children.push(LineageNode::new(
        "Pod",
        "web-1",
        Some("default".to_string()),
        LineageRelation::Child,
    ));
    let mut tree = TreeViewState::new("Deployment".into(), "web".into(), Some("default".into()));
    tree.set_tree(root);
    tree
}

#[tokio::test]
async fn tree_keys_copy_a_node_the_whole_tree_and_open_the_selection() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig")];

    app.active_view = ActiveView::Tree(tree_view());
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied 'web' to clipboard");

    app.handle_key_event(common::shift(crossterm::event::KeyCode::Char('C')))
        .await;
    assert_eq!(
        toast(&app),
        "✓ Copied resource relationship tree to clipboard"
    );

    app.handle_key_event(common::ch('y')).await;
    match &app.active_view {
        ActiveView::Yaml(y) => {
            assert_eq!(y.resource_name, "web");
            assert_eq!(y.resource_kind, "Deployment");
        }
        _ => panic!("expected the selected node's YAML"),
    }

    app.active_view = ActiveView::Tree(tree_view());
    app.handle_key_event(common::ch('d')).await;
    match &app.active_view {
        ActiveView::Describe(d) => assert_eq!(d.resource_name, "web"),
        _ => panic!("expected the selected node's describe"),
    }

    app.active_view = ActiveView::Tree(tree_view());
    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter))
        .await;
    match &app.active_view {
        ActiveView::Describe(d) => assert_eq!(d.resource_kind, "Deployment"),
        _ => panic!("enter describes the selection"),
    }

    // Logs are only offered for pods.
    app.active_view = ActiveView::Tree(tree_view());
    app.handle_key_event(common::ch('l')).await;
    assert_eq!(
        toast(&app),
        "Logs only available for Pods (selected Deployment)"
    );

    app.active_view = ActiveView::Tree(tree_view());
    app.handle_key_event(common::ch('x')).await;
    match &app.modal {
        Some(Modal::ActionPalette { resource_name, .. }) => assert_eq!(resource_name, "web"),
        other => panic!("expected the action palette, got {:?}", other),
    }
}

#[tokio::test]
async fn port_forward_keys_copy_the_local_url_a_deep_link_and_confirm_a_stop() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    let mut pf = PortForwardViewState::new();
    pf.set_forwards(vec![PortForwardEntry {
        id: "pf-1".into(),
        context: FAKE_CONTEXT.into(),
        namespace: "default".into(),
        target_type: "Pod".into(),
        target_name: "web-0".into(),
        local_port: 8080,
        container_port: 80,
        active_connections: 0,
        bytes_rx: 0,
        bytes_tx: 0,
        status: "Active".into(),
    }]);
    app.active_view = ActiveView::PortForwards(pf);

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied forward URL: http://127.0.0.1:8080");

    app.handle_key_event(common::ctrl('y')).await;
    assert!(
        toast(&app).starts_with("Copied deep link:") && toast(&app).contains("web-0"),
        "toast: {}",
        toast(&app)
    );

    app.handle_key_event(common::ch('d')).await;
    match &app.modal {
        Some(Modal::Confirm {
            title, action_name, ..
        }) => {
            assert_eq!(title, "Stop Port Forward [127.0.0.1:8080]");
            assert_eq!(action_name, "stop-pf:pf-1");
        }
        other => panic!("expected the stop confirmation, got {:?}", other),
    }
}

#[tokio::test]
async fn helm_keys_copy_a_deep_link_and_open_the_values_and_manifest() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig")];
    let helm = || {
        let mut helm = HelmViewState::new();
        helm.set_releases(vec![HelmReleaseItem {
            name: "nginx".into(),
            namespace: "default".into(),
            revision: 3,
            status: "deployed".into(),
            chart: "nginx-15.0.0".into(),
            chart_version: "15.0.0".into(),
            app_version: "1.25".into(),
            updated: "2026-01-01".into(),
        }]);
        helm
    };

    app.active_view = ActiveView::Helm(helm());
    app.handle_key_event(common::ch('c')).await;
    assert!(
        toast(&app).starts_with("Copied deep link:") && toast(&app).contains("nginx"),
        "toast: {}",
        toast(&app)
    );

    app.handle_key_event(common::ctrl('y')).await;
    assert!(
        toast(&app).contains("HelmRelease"),
        "toast: {}",
        toast(&app)
    );

    app.active_view = ActiveView::Helm(helm());
    app.handle_key_event(common::ch('v')).await;
    match &app.active_view {
        ActiveView::HelmDetail(d) => {
            assert_eq!(d.active_tab, srelens_tui::views::HelmDetailTab::ValuesDiff);
            assert_eq!(d.release_name, "nginx");
        }
        _ => panic!("expected the Helm values view"),
    }

    app.active_view = ActiveView::Helm(helm());
    app.handle_key_event(common::ch('y')).await;
    match &app.active_view {
        ActiveView::HelmDetail(d) => {
            assert_eq!(d.active_tab, srelens_tui::views::HelmDetailTab::Manifest);
            assert_eq!(d.release_name, "nginx");
        }
        _ => panic!("expected the Helm manifest view"),
    }
}

#[tokio::test]
async fn toolbox_keys_move_the_cursor_and_copy_the_tool_path_or_name() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.active_view = ActiveView::Toolbox(ToolboxViewState {
        tools: vec![
            ToolStatusItem {
                name: "kubectl".into(),
                installed: true,
                version: Some("v1.31.7".into()),
                path: Some("/usr/local/bin/kubectl".into()),
                required: true,
            },
            ToolStatusItem {
                name: "helm".into(),
                installed: false,
                version: None,
                path: None,
                required: false,
            },
        ],
        selected_idx: 0,
    });

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied tool info: /usr/local/bin/kubectl");

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::ch('y')).await;
    assert_eq!(
        toast(&app),
        "Copied tool info: helm",
        "a tool with no path falls back to its name"
    );

    app.handle_key_event(common::ch('k')).await;
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied tool info: /usr/local/bin/kubectl");
}

// ---------------------------------------------------------------------------
// Assistant chords
// ---------------------------------------------------------------------------

#[tokio::test]
async fn assistant_chords_clear_the_conversation_and_toggle_tool_chips() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.active_view = ActiveView::Assistant;
    app.assistant_state
        .add_assistant_message("the cluster is fine".into());

    let expanded = app.assistant_state.expand_tools;
    app.handle_key_event(common::ctrl('t')).await;
    assert_ne!(
        app.assistant_state.expand_tools, expanded,
        "ctrl+t toggles tool-chip expansion"
    );

    app.handle_key_event(common::ctrl('l')).await;
    assert_eq!(toast(&app), "✓ Conversation cleared");
    assert_eq!(
        app.assistant_state.messages.len(),
        1,
        "clearing leaves only the seeded greeting"
    );
    assert!(app.assistant_state.messages[0]
        .content
        .starts_with("Hello!"));
}

#[tokio::test]
async fn ctrl_c_in_the_assistant_quits_instead_of_copying_the_last_answer() {
    // `handle_key_event` treats Ctrl+C as the global quit before the assistant
    // ever sees it, so the assistant's own Ctrl+C copy chord cannot fire.
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.active_view = ActiveView::Assistant;
    app.assistant_state
        .add_assistant_message("the cluster is fine".into());

    app.handle_key_event(common::ctrl('c')).await;
    assert!(!app.is_running, "Ctrl+C exits the TUI");
    assert_eq!(toast(&app), "", "nothing is copied on the way out");
}

// ---------------------------------------------------------------------------
// Watch pool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_full_watch_pool_evicts_its_oldest_channel_when_a_workloads_view_opens() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    for i in 0..20 {
        let channel = format!("watch:filler:{}", i);
        app.active_watch_pool.push(channel.clone());
        app.active_watch_channels.insert(channel);
    }

    app.switch_view_to_kind(ResourceKind::Workloads).await;

    assert!(
        !app.active_watch_channels.contains("watch:filler:0"),
        "the oldest channel is evicted to make room"
    );
    assert!(
        app.active_watch_channels
            .contains(&format!("watch:{}:default:deployments", FAKE_CONTEXT)),
        "the workloads view registers its constituent watches: {:?}",
        app.active_watch_channels
    );
}

// ---------------------------------------------------------------------------
// Live metrics refreshes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_metrics_server_feeds_the_pod_table_and_the_node_history() {
    let cluster = fake_cluster(vec![
        route(
            "/apis/metrics.k8s.io/v1beta1/namespaces/default/pods",
            json!({
                "apiVersion": "metrics.k8s.io/v1beta1",
                "kind": "PodMetricsList",
                "metadata": { "resourceVersion": "1" },
                "items": [{
                    "metadata": { "name": "web-0", "namespace": "default" },
                    "containers": [
                        { "name": "app", "usage": { "cpu": "200m", "memory": "1024Mi" } },
                        { "name": "sidecar", "usage": { "cpu": "50m", "memory": "512Mi" } },
                    ],
                }],
            }),
        ),
        route(
            "/apis/metrics.k8s.io/v1beta1/nodes",
            json!({
                "apiVersion": "metrics.k8s.io/v1beta1",
                "kind": "NodeMetricsList",
                "metadata": { "resourceVersion": "1" },
                "items": [{
                    "metadata": { "name": "node-a" },
                    "usage": { "cpu": "750m", "memory": "3072Mi" },
                }],
            }),
        ),
    ])
    .await;
    let (mut app, mut rx) = app_on(&cluster).await;
    pods_table(&mut app, &["web-0"]);

    app.refresh_pod_metrics();
    let payload = action_result(&mut rx, "pod_metrics_updated")
        .await
        .expect("pod metrics are published");
    app.handle_pod_metrics_update(&payload);
    match &app.active_view {
        ActiveView::Table(t) => {
            assert_eq!(
                t.raw_items[0]["cpu"], "250m",
                "both containers count towards the pod's usage"
            );
            assert_eq!(t.raw_items[0]["memory"], "1.5Gi");
        }
        _ => panic!("expected the pods table"),
    }
    assert_eq!(app.pod_metrics_history["web-0"].len(), 1);

    app.active_view = ActiveView::NodeInspector(inspector("node-a", false, false));
    app.refresh_node_metrics();
    let payload = action_result(&mut rx, "node_metrics_updated")
        .await
        .expect("node metrics are published");
    app.handle_node_metrics_update(&payload);
    match &app.active_view {
        ActiveView::NodeInspector(ni) => {
            assert_eq!(ni.cpu_history, vec![750]);
            assert_eq!(ni.mem_history, vec![3072]);
        }
        _ => panic!("expected the node inspector"),
    }
}

#[tokio::test]
async fn a_cluster_without_a_metrics_server_publishes_no_metrics_at_all() {
    let cluster = fake_cluster(vec![failing_route("/apis/metrics.k8s.io", 404)]).await;
    let (app, mut rx) = app_on(&cluster).await;

    app.refresh_pod_metrics();
    app.refresh_node_metrics();
    tokio::task::yield_now().await;
    let titles: Vec<String> = common::drain(&mut rx)
        .into_iter()
        .filter_map(|ev| match ev {
            AppEvent::ActionResult { title, .. } => Some(title),
            _ => None,
        })
        .collect();
    assert!(
        !titles.iter().any(|t| t.ends_with("metrics_updated")),
        "a 404 from metrics.k8s.io publishes nothing: {titles:?}"
    );
    assert!(app.pod_metrics_history.is_empty());
    assert!(app.node_metrics_history.is_empty());
}

// ---------------------------------------------------------------------------
// Command resolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_command_nothing_can_explain_toasts_that_it_is_unknown() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;

    app.execute_colon_command("qqqqqqqqqq").await;
    assert_eq!(
        toast(&app),
        "Unknown command: 'qqqqqqqqqq' (type :help or ?)"
    );
}

#[tokio::test]
async fn enter_on_a_plain_table_row_describes_it() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.kubeconfig_paths = vec![PathBuf::from("no-such-kubeconfig")];

    let mut table = ResourceTableState::new(ResourceKind::ConfigMaps);
    table.set_items(
        vec![json!({ "name": "settings", "namespace": "default" })],
        "",
    );
    table.is_loading = false;
    app.active_view = ActiveView::Table(table);

    app.handle_key_event(common::key(crossterm::event::KeyCode::Enter))
        .await;
    match &app.active_view {
        ActiveView::Describe(d) => {
            assert_eq!(d.resource_name, "settings");
            assert_eq!(d.resource_kind, "ConfigMap");
        }
        _ => panic!("enter describes the selected row"),
    }
}

#[tokio::test]
async fn a_command_only_a_crd_short_name_can_explain_still_opens_that_crd() {
    // Resolution never looks at CRD short names for a two-letter prefix, but
    // the suggestion list does and scores it well above the confidence bar, so
    // the app runs the top suggestion instead of complaining.
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    let mut crd = widget_crd();
    crd.plural = "zzthings".into();
    crd.singular = "zzthing".into();
    crd.short_names = vec!["wdgt".into()];
    app.crds = vec![crd];

    assert!(
        srelens_tui::commands::resolve_command_with_crds("wd", &app.crds).is_none(),
        "'wd' resolves to nothing on its own"
    );

    app.execute_colon_command("wd").await;
    match &app.active_view {
        ActiveView::Table(t) => match &t.kind {
            ResourceKind::CustomResource(found) => assert_eq!(found.kind, "Widget"),
            other => panic!("expected the custom resource table, got {}", other),
        },
        _ => panic!("expected a table"),
    }
    assert_eq!(toast(&app), "", "no complaint for a confident correction");
}

// ---------------------------------------------------------------------------
// Clipboard copies that happen on a spawned task
// ---------------------------------------------------------------------------

/// Let the tasks the app just spawned (the clipboard writes) run to completion.
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn copying_from_a_table_covers_one_row_a_marked_set_and_a_whole_manifest() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    pods_table(&mut app, &["web-0", "web-1", "web-2"]);

    app.handle_key_event(common::ch('c')).await;
    settle().await;
    assert_eq!(toast(&app), "✓ Copied 'web-0' to clipboard");

    // Space marks a row; two marks make the copy a name list.
    app.handle_key_event(common::ch(' ')).await;
    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::ch(' ')).await;
    app.handle_key_event(common::ch('c')).await;
    settle().await;
    assert_eq!(toast(&app), "✓ Copied 2 resource names to clipboard");

    app.handle_key_event(common::shift(crossterm::event::KeyCode::Char('C')))
        .await;
    settle().await;
    assert!(
        toast(&app).contains("Copied"),
        "shift+C copies the row's manifest: {}",
        toast(&app)
    );
}

#[tokio::test]
async fn copying_an_event_summary_a_tree_a_forward_link_and_a_tool_path_reaches_the_clipboard() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;

    let mut table = ResourceTableState::new(ResourceKind::Events);
    table.set_items(
        vec![json!({
            "name": "e1", "namespace": "default", "reason": "BackOff",
            "type": "Warning", "object": "Pod/web-1", "age": "2m", "message": "restarting",
        })],
        "",
    );
    table.is_loading = false;
    app.active_view = ActiveView::Table(table);
    app.handle_key_event(common::ch('c')).await;
    settle().await;
    assert_eq!(toast(&app), "✓ Copied event message to clipboard");

    app.active_view = ActiveView::Tree(tree_view());
    app.handle_key_event(common::ch('c')).await;
    settle().await;
    assert_eq!(toast(&app), "✓ Copied 'web' to clipboard");
    app.handle_key_event(common::shift(crossterm::event::KeyCode::Char('C')))
        .await;
    settle().await;
    assert_eq!(
        toast(&app),
        "✓ Copied resource relationship tree to clipboard"
    );

    let mut pf = PortForwardViewState::new();
    pf.set_forwards(vec![PortForwardEntry {
        id: "pf-1".into(),
        context: FAKE_CONTEXT.into(),
        namespace: "default".into(),
        target_type: "Pod".into(),
        target_name: "web-0".into(),
        local_port: 9090,
        container_port: 90,
        active_connections: 0,
        bytes_rx: 0,
        bytes_tx: 0,
        status: "Active".into(),
    }]);
    app.active_view = ActiveView::PortForwards(pf);
    app.handle_key_event(common::ch('c')).await;
    settle().await;
    assert_eq!(toast(&app), "Copied forward URL: http://127.0.0.1:9090");
    app.handle_key_event(common::ctrl('y')).await;
    settle().await;
    assert!(toast(&app).starts_with("Copied deep link:"));

    app.active_view = ActiveView::Toolbox(ToolboxViewState {
        tools: vec![ToolStatusItem {
            name: "kubectl".into(),
            installed: true,
            version: None,
            path: Some("/usr/local/bin/kubectl".into()),
            required: true,
        }],
        selected_idx: 0,
    });
    app.handle_key_event(common::ch('c')).await;
    settle().await;
    assert_eq!(toast(&app), "Copied tool info: /usr/local/bin/kubectl");
}

#[tokio::test]
async fn releasing_a_drag_in_the_yaml_view_copies_the_dragged_lines() {
    use crossterm::event::{MouseButton, MouseEventKind};

    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    let content: String = (0..12).map(|i| format!("line-{i}\n")).collect();
    app.active_view = ActiveView::Yaml(srelens_tui::views::yaml_view::YamlViewState::new(
        "web-0".into(),
        "Pod".into(),
        None,
        content,
    ));
    common::render_app(&mut app, 120, 40);
    let viewport = match &app.active_view {
        ActiveView::Yaml(y) => y.last_viewport_rect.get(),
        _ => unreachable!(),
    };

    app.handle_mouse(common::click(viewport.x + 1, viewport.y))
        .await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Drag(MouseButton::Left),
        viewport.x + 1,
        viewport.y + 2,
    ))
    .await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Up(MouseButton::Left),
        viewport.x + 1,
        viewport.y + 2,
    ))
    .await;
    settle().await;
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
    match &app.active_view {
        ActiveView::Yaml(y) => {
            assert_eq!(y.selected_text().as_deref(), Some("line-0\nline-1\nline-2"))
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn a_mouse_event_the_node_inspector_has_no_use_for_leaves_it_alone() {
    use crossterm::event::MouseEventKind;

    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.active_view = ActiveView::NodeInspector(inspector("gpu-1", false, true));
    common::render_app(&mut app, 160, 45);
    let rect = match &app.active_view {
        ActiveView::NodeInspector(ni) => ni.last_pods_table_rect.get(),
        _ => unreachable!(),
    };

    app.handle_mouse(common::mouse(MouseEventKind::Moved, rect.x + 1, rect.y + 1))
        .await;
    match &app.active_view {
        ActiveView::NodeInspector(ni) => assert_eq!(ni.selected_pod_idx, 0),
        _ => panic!("still the node inspector"),
    }
}

// ---------------------------------------------------------------------------
// Action palettes
// ---------------------------------------------------------------------------

fn palette_titles(app: &App) -> Vec<String> {
    match &app.modal {
        Some(Modal::ActionPalette { actions, .. }) => {
            actions.iter().map(|a| a.title.clone()).collect()
        }
        other => panic!("expected the action palette, got {:?}", other),
    }
}

#[tokio::test]
async fn the_ingress_action_palette_offers_the_tree_describe_yaml_and_delete() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;

    app.open_action_palette("ingresses".into(), "web".into(), Some("default".into()));
    let titles = palette_titles(&app);
    assert_eq!(titles.len(), 4);
    assert!(
        titles[0].contains("Resource Relationship Tree"),
        "titles: {titles:?}"
    );
    assert!(titles[1].contains("Describe Ingress"), "titles: {titles:?}");
    assert!(titles[2].contains("View YAML"), "titles: {titles:?}");
    assert!(titles[3].contains("Delete Ingress"), "titles: {titles:?}");

    let screen = common::render_app(&mut app, 140, 40);
    assert!(
        screen.contains("Actions: ingresses/web (default)"),
        "{screen}"
    );

    // The singular spelling reaches the same list.
    app.open_action_palette("ingress".into(), "web".into(), None);
    assert_eq!(palette_titles(&app).len(), 4);
}

// ---------------------------------------------------------------------------
// Assistant composer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tab_in_the_assistant_completes_the_highlighted_slash_command() {
    let (mut app, _rx) = common::app_with(FAKE_CONTEXT, "default").await;
    app.active_view = ActiveView::Assistant;

    common::type_str(&mut app, "/cav").await;
    assert!(
        !app.assistant_state.slash_suggestions.is_empty(),
        "'/cav' offers at least one slash command"
    );
    let first = app.assistant_state.slash_suggestions[0].name;

    app.handle_key_event(common::key(crossterm::event::KeyCode::Tab))
        .await;
    assert!(
        app.assistant_state
            .input
            .starts_with(&format!("/{}", first)),
        "tab completes to the highlighted suggestion: {:?}",
        app.assistant_state.input
    );

    // With nothing to complete, Tab is not swallowed as a completion.
    app.assistant_state.input.clear();
    app.assistant_state.slash_suggestions.clear();
    app.handle_key_event(common::key(crossterm::event::KeyCode::Tab))
        .await;
    assert_eq!(app.assistant_state.input, "");
}
