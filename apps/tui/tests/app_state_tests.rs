//! Integration tests for `App` state handling: incoming events, rendering
//! across every view and modal, mouse handling, per-view key handling, and
//! the free helpers at the bottom of `app.rs`.

mod common;

use std::collections::VecDeque;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use serde_json::{json, Value};
use srelens_kube::lineage::{LineageNode, LineageRelation};
use srelens_kube::metrics::MetricSample;
use srelens_kube::node_inspector::{
    NodeConditionInfo, NodeInspectorDetails, NodePodItem, NodeTaintInfo,
};
use srelens_tui::ai_skills::CavemanLevel;
use srelens_tui::app::{
    copy_to_clipboard, copy_via_osc52, extract_tool_call_completed_info,
    extract_tool_call_start_info, format_event_summary, get_clipboard_text, parse_involved_object,
    parse_ready_ratio, ActiveView, App, SuspendAction,
};
use srelens_tui::commands::{CommandTarget, CrdMeta, PrinterColumn, ResourceKind};
use srelens_tui::event::AppEvent;
use srelens_tui::ui::dialogs::{QuickActionId, QuickActionItem};
use srelens_tui::ui::{ContainerAction, InputMode, Modal};
use srelens_tui::views::describe_view::DescribeViewState;
use srelens_tui::views::helm_view::{HelmReleaseItem, HelmViewState};
use srelens_tui::views::logs_view::LogsViewState;
use srelens_tui::views::metrics_panel_view::MetricsPanelState;
use srelens_tui::views::node_inspector_view::NodeInspectorState;
use srelens_tui::views::overview_view::{ClusterOverviewData, OverviewViewState};
use srelens_tui::views::port_forward_view::{PortForwardEntry, PortForwardViewState};
use srelens_tui::views::resource_table::ResourceTableState;
use srelens_tui::views::toolbox_view::{ToolStatusItem, ToolboxViewState};
use srelens_tui::views::tree_view::TreeViewState;
use srelens_tui::views::yaml_view::YamlViewState;
use srelens_tui::{AiProvider, AiSettings, DeepLink};
use tokio::sync::mpsc::UnboundedReceiver;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const WIDE: (u16, u16) = (200, 50);
const NARROW: (u16, u16) = (80, 24);

/// Point AI settings persistence at a scratch file so no test touches the
/// user's real configuration.
fn isolate_ai_settings() {
    let dir = std::env::temp_dir().join("srelens-app-state-tests");
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("SRELENS_AI_SETTINGS_PATH", dir.join("ai_settings.json"));
}

/// An assistant configuration that can never reach a provider: Gemini with no
/// key anywhere, so a submitted query is answered by the "No API key" reply.
fn keyless_assistant(app: &mut App) {
    std::env::remove_var("GEMINI_API_KEY");
    let mut settings = AiSettings::default();
    settings.default_provider = AiProvider::Gemini;
    settings.api_keys.clear();
    app.ai_settings = settings;
    app.assistant_state.caveman_level = None;
}

fn toast(app: &App) -> String {
    app.toast
        .as_ref()
        .map(|(m, _, _)| m.clone())
        .unwrap_or_default()
}

fn wide(app: &mut App) -> String {
    common::render_app(app, WIDE.0, WIDE.1)
}

fn narrow(app: &mut App) -> String {
    common::render_app(app, NARROW.0, NARROW.1)
}

fn pod(name: &str, ns: &str) -> Value {
    json!({
        "name": name,
        "namespace": ns,
        "status": "Running",
        "ready": "1/1",
        "restarts": 0,
        "age": "5m",
        "node": "node-1",
    })
}

fn table_with(kind: ResourceKind, items: Vec<Value>) -> ResourceTableState {
    let mut table = ResourceTableState::new(kind);
    table.set_items(items, "");
    table.is_loading = false;
    table
}

fn sample(ts: u64, cpu: u64, mem: u64) -> MetricSample {
    MetricSample {
        timestamp_epoch_ms: ts,
        cpu_millicores: cpu,
        memory_mib: mem,
    }
}

fn overview_data() -> ClusterOverviewData {
    ClusterOverviewData {
        context_name: "test-cluster".into(),
        cluster_name: "kubernetes".into(),
        server_url: "https://10.0.0.1:6443".into(),
        k8s_version: "v1.31.0".into(),
        is_reachable: true,
        node_count: 3,
        ready_nodes: 3,
        total_pods: 40,
        running_pods: 38,
        pending_pods: 1,
        failed_pods: 1,
        total_cpu_millicores: 12_000,
        used_cpu_millicores: 4_000,
        total_mem_mib: 32 * 1024,
        used_mem_mib: 8 * 1024,
        total_gpus: 2,
        allocated_gpus: 1,
        total_gpu_mem_mib: 32 * 1024,
        used_gpu_mem_mib: 4 * 1024,
    }
}

/// A toolbox built by hand: `ToolboxViewState::new()` shells out to `which`.
fn toolbox() -> ToolboxViewState {
    ToolboxViewState {
        tools: vec![
            ToolStatusItem {
                name: "kubectl".into(),
                installed: true,
                version: Some("v1.31.0".into()),
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
    }
}

fn port_forwards() -> PortForwardViewState {
    let mut pf = PortForwardViewState::new();
    pf.set_forwards(vec![PortForwardEntry {
        id: "pf-1".into(),
        context: "test-cluster".into(),
        namespace: "default".into(),
        target_type: "Pod".into(),
        target_name: "web-0".into(),
        local_port: 8080,
        container_port: 80,
        active_connections: 1,
        bytes_rx: 10,
        bytes_tx: 20,
        status: "Active".into(),
    }]);
    pf
}

fn helm_releases() -> HelmViewState {
    let mut helm = HelmViewState::new();
    helm.set_releases(vec![HelmReleaseItem {
        name: "nginx".into(),
        namespace: "default".into(),
        revision: 3,
        status: "deployed".into(),
        chart: "nginx-15.0.0".into(),
        app_version: "1.25".into(),
        updated: "2026-01-01".into(),
    }]);
    helm
}

fn lineage_tree() -> LineageNode {
    let mut root = LineageNode::new(
        "Deployment",
        "web",
        Some("default".to_string()),
        LineageRelation::Target,
    );
    let mut rs = LineageNode::new(
        "ReplicaSet",
        "web-abc",
        Some("default".to_string()),
        LineageRelation::Child,
    );
    rs.children.push(LineageNode::new(
        "Pod",
        "web-abc-1",
        Some("default".to_string()),
        LineageRelation::Child,
    ));
    root.children.push(rs);
    root
}

fn tree_view() -> TreeViewState {
    let mut tree = TreeViewState::new("Deployment".into(), "web".into(), Some("default".into()));
    tree.set_tree(lineage_tree());
    tree
}

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

fn node_details(name: &str, unschedulable: bool) -> NodeInspectorDetails {
    NodeInspectorDetails {
        name: name.into(),
        status: "Ready".into(),
        unschedulable,
        roles: "worker".into(),
        instance_type: "m5.large".into(),
        zone: Some("eu-west-1a".into()),
        region: Some("eu-west-1".into()),
        nodepool: None,
        internal_ip: Some("10.0.0.1".into()),
        external_ip: None,
        os_image: "Ubuntu".into(),
        kernel_version: "6.1".into(),
        container_runtime: "containerd".into(),
        kubelet_version: "v1.31.0".into(),
        architecture: "amd64".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        cpu_capacity_millicores: 4000,
        cpu_allocatable_millicores: 3800,
        cpu_requests_millicores: 1000,
        mem_capacity_mib: 8192,
        mem_allocatable_mib: 7800,
        mem_requests_mib: 2048,
        pods_capacity: 110,
        pods_allocatable: 110,
        pods_count: 2,
        has_gpu: true,
        gpu_model: Some("A10".into()),
        gpu_driver_version: Some("550".into()),
        gpu_cuda_version: Some("12.4".into()),
        gpu_capacity_count: 1,
        gpu_allocatable_count: 1,
        gpu_requests_count: 0,
        gpu_memory_total_mib: Some(24_576),
        gpu_memory_requests_mib: 0,
        conditions: vec![NodeConditionInfo {
            type_: "Ready".into(),
            status: "True".into(),
            reason: None,
            message: None,
        }],
        taints: vec![NodeTaintInfo {
            key: "dedicated".into(),
            value: Some("gpu".into()),
            effect: "NoSchedule".into(),
        }],
        pods: vec![
            node_pod("api-0", "default"),
            node_pod("exporter", "monitoring"),
        ],
    }
}

fn inspector_with_details(name: &str, unschedulable: bool) -> NodeInspectorState {
    let mut ni = NodeInspectorState::new(name.into());
    ni.set_details(node_details(name, unschedulable));
    ni
}

fn action(id: QuickActionId, title: &str) -> QuickActionItem {
    QuickActionItem {
        id,
        key_hint: "k".into(),
        title: title.into(),
        description: "desc".into(),
    }
}

fn palette(kind: &str, name: &str, ns: Option<&str>, id: QuickActionId) -> Modal {
    Modal::ActionPalette {
        resource_kind: kind.into(),
        resource_name: name.into(),
        namespace: ns.map(String::from),
        actions: vec![action(id, "Chosen action")],
        selected_idx: 0,
        filter: String::new(),
    }
}

/// Wait for the first emitted event matching `pred`; spawned cluster calls
/// fail fast, so this never waits on a network round trip.
async fn wait_for<F>(rx: &mut UnboundedReceiver<AppEvent>, mut pred: F) -> AppEvent
where
    F: FnMut(&AppEvent) -> bool,
{
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let ev = rx.recv().await.expect("event channel stays open");
            if pred(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("expected event was emitted")
}

fn last_message(app: &App) -> String {
    app.assistant_state
        .messages
        .last()
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

/// A key press with Alt held.
fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

/// Type into the assistant composer. A leading `c` on an empty input is the
/// "copy the last answer" shortcut rather than a character, so the first
/// character is seeded directly when it would be swallowed.
async fn compose(app: &mut App, text: &str) {
    for (i, c) in text.chars().enumerate() {
        if i == 0 && c == 'c' && app.assistant_state.input.is_empty() {
            app.assistant_state.input.push('c');
            continue;
        }
        app.handle_key_event(common::ch(c)).await;
    }
}

/// The cell column of `needle` within a rendered row. `str::find` returns a
/// byte offset and the frame is full of multi-byte box-drawing characters, so
/// the count has to be in characters.
fn cell_col(line: &str, needle: &str) -> u16 {
    let byte = line.find(needle).expect("text is on this row");
    line[..byte].chars().count() as u16
}

/// A conversation always opens with one seeded assistant greeting.
fn assert_only_greeting(app: &App) {
    assert_eq!(app.assistant_state.messages.len(), 1);
    assert_eq!(app.assistant_state.messages[0].role, "assistant");
    assert!(
        app.assistant_state.messages[0]
            .content
            .starts_with("Hello!"),
        "{}",
        app.assistant_state.messages[0].content
    );
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

#[test]
fn copying_to_the_clipboard_reports_a_result_even_without_a_display() {
    // OSC 52 + native command fallbacks: the call must not panic and must
    // return the io::Result shape regardless of what the runner has installed.
    copy_via_osc52("selected text");
    let result = copy_to_clipboard("line one\nline two");
    assert!(result.is_ok() || result.is_err());
    // Base64 boundary cases (1, 2 and 3 byte tails) all go through the encoder.
    copy_via_osc52("a");
    copy_via_osc52("ab");
    copy_via_osc52("");
}

#[test]
fn reading_the_clipboard_never_yields_an_empty_string() {
    match get_clipboard_text() {
        Some(text) => assert!(!text.is_empty(), "Some(...) must carry text"),
        None => {}
    }
}

#[test]
fn a_tool_call_start_summarises_the_arguments_by_kind() {
    let bash = json!({
        "call_id": "c1",
        "tool_call": { "bashToolCall": { "args": { "command": "kubectl get pods" } } }
    });
    assert_eq!(
        extract_tool_call_start_info(&bash),
        Some(("c1".into(), "bash".into(), "kubectl get pods".into()))
    );

    let read = json!({
        "callId": "c2",
        "toolCall": { "readToolCall": { "args": { "path": "/etc/hosts" } } }
    });
    assert_eq!(
        extract_tool_call_start_info(&read),
        Some(("c2".into(), "read".into(), "path: /etc/hosts".into()))
    );

    let grep = json!({ "toolCall": { "grepToolCall": { "args": { "pattern": "ERROR" } } } });
    assert_eq!(
        extract_tool_call_start_info(&grep).unwrap().2,
        "pattern: ERROR"
    );

    let search = json!({ "toolCall": { "webSearchToolCall": { "args": { "query": "k8s" } } } });
    assert_eq!(
        extract_tool_call_start_info(&search).unwrap().2,
        "query: k8s"
    );

    let named = json!({ "toolCall": { "skillToolCall": { "args": { "name": "triage" } } } });
    assert_eq!(
        extract_tool_call_start_info(&named).unwrap().2,
        "name: triage"
    );

    let str_args = json!({ "toolCall": { "echoToolCall": { "args": "raw" } } });
    assert_eq!(extract_tool_call_start_info(&str_args).unwrap().2, "raw");

    let empty_obj = json!({ "toolCall": { "listToolCall": { "args": {} } } });
    assert_eq!(extract_tool_call_start_info(&empty_obj).unwrap().2, "");

    let other_obj = json!({ "toolCall": { "xToolCall": { "args": { "k": 1 } } } });
    assert_eq!(
        extract_tool_call_start_info(&other_obj).unwrap().2,
        "{\"k\":1}"
    );

    let num_args = json!({ "toolCall": { "xToolCall": { "args": 42 } } });
    assert_eq!(extract_tool_call_start_info(&num_args).unwrap().2, "42");

    let no_args = json!({ "toolCall": { "xToolCall": {} } });
    assert_eq!(extract_tool_call_start_info(&no_args).unwrap().2, "");
}

#[test]
fn an_mcp_tool_call_start_uses_the_nested_tool_name_and_arguments() {
    // The nested name is only unwrapped once the key strips down to exactly
    // "callMcpTool" or "mcp".
    let mcp = json!({
        "call_id": "m1",
        "tool_call": { "callMcpToolToolCall": {
            "args": { "tool": "list_pods", "arguments": { "namespace": "default" } }
        } }
    });
    assert_eq!(
        extract_tool_call_start_info(&mcp),
        Some((
            "m1".into(),
            "list_pods".into(),
            "{\"namespace\":\"default\"}".into()
        ))
    );

    let scalar_args = json!({
        "tool_call": { "mcpToolCall": { "args": { "name": "get_logs", "arguments": "web-0" } } }
    });
    assert_eq!(
        extract_tool_call_start_info(&scalar_args),
        Some(("".into(), "get_logs".into(), "\"web-0\"".into()))
    );

    // Any other key keeps the stripped key as the tool name.
    let plain = json!({
        "tool_call": { "callMcpToolCall": { "args": { "tool": "list_pods", "arguments": {} } } }
    });
    assert_eq!(extract_tool_call_start_info(&plain).unwrap().1, "callMcp");
}

#[test]
fn a_tool_call_start_without_a_tool_call_payload_is_ignored() {
    assert_eq!(
        extract_tool_call_start_info(&json!({ "call_id": "x" })),
        None
    );
    assert_eq!(
        extract_tool_call_start_info(&json!({ "tool_call": "not-an-object" })),
        None
    );
    // Only metadata siblings, no *ToolCall key.
    let meta_only = json!({ "tool_call": { "hookAdditionalContexts": {}, "startedAtMs": 1 } });
    assert_eq!(extract_tool_call_start_info(&meta_only), None);
}

#[test]
fn a_tool_call_completion_carries_its_id_and_error_flag() {
    assert_eq!(
        extract_tool_call_completed_info(&json!({ "call_id": "c1", "is_error": true })),
        Some(("c1".into(), true))
    );
    assert_eq!(
        extract_tool_call_completed_info(&json!({ "callId": "c2", "isError": false })),
        Some(("c2".into(), false))
    );
    assert_eq!(
        extract_tool_call_completed_info(&json!({})),
        Some(("".into(), false))
    );
    let with_tool = json!({ "call_id": "c3", "toolCall": { "bashToolCall": {} } });
    assert_eq!(
        extract_tool_call_completed_info(&with_tool),
        Some(("c3".into(), false))
    );
    let meta_only = json!({ "call_id": "c4", "tool_call": { "hookAdditionalContexts": {} } });
    assert_eq!(extract_tool_call_completed_info(&meta_only), None);
}

#[test]
fn an_events_involved_object_is_read_from_either_shape() {
    assert_eq!(
        parse_involved_object(&json!({ "object": "Pod/web-0" })),
        ("Pod".into(), "web-0".into())
    );
    assert_eq!(
        parse_involved_object(&json!({ "involvedObject": { "kind": "Node", "name": "n1" } })),
        ("Node".into(), "n1".into())
    );
    assert_eq!(
        parse_involved_object(&json!({ "involvedObject": { "name": "orphan" } })),
        ("".into(), "orphan".into())
    );
    assert_eq!(
        parse_involved_object(&json!({ "object": "noslash" })),
        ("".into(), "".into())
    );
    assert_eq!(parse_involved_object(&json!({})), ("".into(), "".into()));
}

#[test]
fn an_event_summary_lists_age_type_reason_object_namespace_and_message() {
    let ev = json!({
        "age": "3m", "type": "Warning", "reason": "BackOff",
        "object": "Pod/web-0", "namespace": "prod", "message": "restarting"
    });
    assert_eq!(
        format_event_summary(&ev),
        "[3m] [Warning] BackOff Pod/web-0 (prod): restarting"
    );
    let typed = json!({ "type_": "Normal", "reason": "Pulled" });
    assert_eq!(format_event_summary(&typed), "[] [Normal] Pulled  (): ");
}

#[test]
fn a_ready_ratio_parses_leniently() {
    assert_eq!(parse_ready_ratio("2/3"), (2, 3));
    assert_eq!(parse_ready_ratio(" 1 / 1 "), (1, 1));
    assert_eq!(parse_ready_ratio("4"), (4, 0));
    assert_eq!(parse_ready_ratio("garbage"), (0, 0));
}

// ---------------------------------------------------------------------------
// Stream events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pods_watch_for_the_current_channel_keeps_previous_metrics_and_fills_the_cache() {
    let (mut app, _rx) = common::app().await;
    let mut table = table_with(ResourceKind::Pods, vec![pod("web-0", "default")]);
    table.raw_items[0]["cpu"] = json!("5m");
    table.raw_items[0]["memory"] = json!("64Mi");
    app.active_view = ActiveView::Table(table);
    let channel = "watch:test-cluster:default:pods".to_string();
    app.current_watch_channel = Some(channel.clone());
    app.is_connected = false;
    app.cluster_unreachable = true;

    app.handle_stream_event(
        channel,
        json!([pod("web-0", "default"), pod("web-1", "default")]),
    );

    assert!(app.is_connected);
    assert!(!app.cluster_unreachable);
    let cached = app
        .resource_cache
        .get(&("test-cluster".into(), "default".into(), "pods".into()))
        .expect("watch payload is cached");
    assert_eq!(cached.len(), 2);
    let ActiveView::Table(t) = &app.active_view else {
        panic!("still a table")
    };
    assert_eq!(t.raw_items.len(), 2);
    assert_eq!(t.raw_items[0]["cpu"], json!("5m"));
    assert_eq!(t.raw_items[0]["memory"], json!("64Mi"));
    assert!(t.raw_items[1].get("cpu").is_none());
}

#[tokio::test]
async fn a_watch_for_another_channel_only_updates_the_cache() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(ResourceKind::Deployments, vec![]));
    app.current_watch_channel = Some("watch:test-cluster:default:deployments".into());

    app.handle_stream_event(
        "watch:test-cluster:default:services".into(),
        json!([{ "name": "svc-a" }]),
    );

    let ActiveView::Table(t) = &app.active_view else {
        panic!("table")
    };
    assert!(t.raw_items.is_empty());
    assert!(app.resource_cache.contains_key(&(
        "test-cluster".into(),
        "default".into(),
        "services".into()
    )));
}

#[tokio::test]
async fn a_non_pod_watch_for_the_current_channel_replaces_the_rows() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(ResourceKind::Services, vec![]));
    let channel = "watch:test-cluster:default:services".to_string();
    app.current_watch_channel = Some(channel.clone());

    app.handle_stream_event(channel, json!([{ "name": "svc-a" }, { "name": "svc-b" }]));

    let ActiveView::Table(t) = &app.active_view else {
        panic!("table")
    };
    assert_eq!(t.raw_items.len(), 2);
    assert_eq!(t.filtered_indices.len(), 2);
}

#[tokio::test]
async fn a_workloads_table_is_rebuilt_when_a_constituent_kind_arrives() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(ResourceKind::Workloads, vec![]));

    app.handle_stream_event(
        "watch:test-cluster:default:deployments".into(),
        json!([{ "name": "web", "namespace": "default", "ready": "1/1" }]),
    );

    let ActiveView::Table(t) = &app.active_view else {
        panic!("table")
    };
    assert_eq!(t.kind, ResourceKind::Workloads);
    assert!(
        t.raw_items.iter().any(|i| i["name"] == "web"),
        "workloads table picks up the deployment: {:?}",
        t.raw_items
    );
}

#[tokio::test]
async fn a_namespaces_watch_replaces_the_sorted_namespace_list() {
    let (mut app, _rx) = common::app().await;
    app.handle_stream_event(
        "watch:test-cluster::namespaces".into(),
        json!([{ "name": "zeta" }, { "name": "alpha" }, { "other": 1 }]),
    );
    assert_eq!(
        app.namespaces,
        vec!["alpha".to_string(), "zeta".to_string()]
    );

    // An empty list keeps the previous namespaces.
    app.handle_stream_event("watch:test-cluster::namespaces".into(), json!([]));
    assert_eq!(app.namespaces.len(), 2);
}

#[tokio::test]
async fn log_lines_and_status_markers_for_the_open_log_stream_are_appended() {
    let (mut app, _rx) = common::app().await;
    let logs = LogsViewState::new(
        "web-0".into(),
        "default".into(),
        None,
        "logs:web-0:1".into(),
    );
    app.active_view = ActiveView::Logs(logs);

    app.handle_stream_event("logs:web-0:1".into(), json!({ "line": "hello world" }));
    app.handle_stream_event("logs:web-0:1".into(), json!({ "status": "closed" }));
    app.handle_stream_event("logs:other:2".into(), json!({ "line": "ignored" }));

    let ActiveView::Logs(l) = &app.active_view else {
        panic!("logs")
    };
    assert_eq!(l.lines, vec!["hello world", "--- log status: closed ---"]);
}

// ---------------------------------------------------------------------------
// Rendering: every view, wide and narrow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_table_kind_renders_its_own_key_hints() {
    let (mut app, _rx) = common::app().await;
    app.pod_count = 7;
    let cases: Vec<(ResourceKind, &str)> = vec![
        (ResourceKind::Workloads, "Segment"),
        (ResourceKind::Pods, "PortForward"),
        (ResourceKind::Deployments, "Restart"),
        (ResourceKind::StatefulSets, "Scale"),
        (ResourceKind::DaemonSets, "Scale"),
        (ResourceKind::Services, "Endpoints"),
        (ResourceKind::Ingresses, "Edit"),
        (ResourceKind::Namespaces, "Pods"),
        (ResourceKind::Events, "Triage"),
        (ResourceKind::Nodes, "Inspect"),
        (ResourceKind::ConfigMaps, "Help"),
    ];
    for (kind, hint) in cases {
        let title = kind.display_name().to_string();
        app.active_view = ActiveView::Table(table_with(kind, vec![pod("web-0", "default")]));
        let screen = wide(&mut app);
        assert!(
            screen.contains(&title),
            "{title} title on wide screen:\n{screen}"
        );
        assert!(
            screen.contains(hint),
            "{title} shows hint {hint}:\n{screen}"
        );
        let screen = narrow(&mut app);
        assert!(
            screen.contains(&title),
            "{title} title on narrow screen:\n{screen}"
        );
    }
}

#[tokio::test]
async fn an_unreachable_cluster_with_no_rows_renders_the_cracked_lens() {
    let (mut app, _rx) = common::app().await;
    app.cluster_unreachable = true;
    app.active_view = ActiveView::Table(table_with(ResourceKind::Pods, vec![]));

    let screen = wide(&mut app);
    assert!(screen.contains("Cluster Unreachable"), "{screen}");
    assert!(screen.contains("Retry"), "{screen}");
    assert!(screen.contains("SwitchCtx"), "{screen}");

    // Rows in hand win over the unreachable flag.
    app.active_view = ActiveView::Table(table_with(ResourceKind::Pods, vec![pod("a", "b")]));
    let screen = wide(&mut app);
    assert!(!screen.contains("Cluster Unreachable"), "{screen}");
    assert!(screen.contains("Shell"), "{screen}");
}

#[tokio::test]
async fn text_views_render_their_titles_search_counts_and_hints() {
    let (mut app, _rx) = common::app().await;

    let mut yaml = YamlViewState::new(
        "web-0".into(),
        "Pod".into(),
        Some("default".into()),
        "apiVersion: v1\nkind: Pod\nmetadata:\n  name: web-0\n".into(),
    );
    yaml.set_search_query("Pod");
    app.active_view = ActiveView::Yaml(yaml);
    let screen = wide(&mut app);
    assert!(screen.contains("YAML Manifest"), "{screen}");
    assert!(screen.contains("Next/Prev"), "{screen}");
    assert!(screen.contains("Edit"), "{screen}");
    // A narrow terminal truncates the header's view name, but the body keeps
    // its own title.
    let screen = narrow(&mut app);
    assert!(screen.contains("YAML: Pod/web-0"), "{screen}");

    let mut desc = DescribeViewState::new(
        "web-0".into(),
        "Pod".into(),
        Some("default".into()),
        "Name: web-0\nNamespace: default\nStatus: Running\n".into(),
    );
    desc.set_search_query("Running");
    app.active_view = ActiveView::Describe(desc);
    let screen = wide(&mut app);
    assert!(screen.contains("Describe: Pod/web-0"), "{screen}");
    assert!(screen.contains("Next/Prev"), "{screen}");
    assert!(narrow(&mut app).contains("Describe: Pod/web-0"));

    let mut logs = LogsViewState::new(
        "web-0".into(),
        "default".into(),
        Some("app".into()),
        "ch".into(),
    );
    logs.push_line("first line".into());
    logs.push_line("second ERROR line".into());
    logs.set_search_query("ERROR");
    app.active_view = ActiveView::Logs(logs);
    // An active search narrows the body to the matching lines.
    let screen = wide(&mut app);
    assert!(screen.contains("Logs: web-0 (default/app)"), "{screen}");
    assert!(screen.contains("[2/2 lines]"), "{screen}");
    assert!(screen.contains("1/1 matches"), "{screen}");
    assert!(screen.contains("second ERROR line"), "{screen}");
    assert!(!screen.contains("first line"), "{screen}");
    assert!(screen.contains("Timestamps"), "{screen}");
    assert!(screen.contains("Follow"), "{screen}");
    assert!(narrow(&mut app).contains("second ERROR line"));
}

#[tokio::test]
async fn list_views_render_their_rows_and_hints() {
    let (mut app, _rx) = common::app().await;

    app.active_view = ActiveView::PortForwards(port_forwards());
    let screen = wide(&mut app);
    assert!(screen.contains("Active Port Forwards [1]"), "{screen}");
    assert!(screen.contains("web-0"), "{screen}");
    assert!(screen.contains("CopyURL"), "{screen}");
    assert!(screen.contains("Stop"), "{screen}");
    assert!(narrow(&mut app).contains("Active Port Forwards [1]"));

    app.active_view = ActiveView::Helm(helm_releases());
    let screen = wide(&mut app);
    assert!(screen.contains("Helm 3 Releases [1]"), "{screen}");
    assert!(screen.contains("nginx"), "{screen}");
    assert!(screen.contains("Values"), "{screen}");
    assert!(screen.contains("Manifest"), "{screen}");
    assert!(narrow(&mut app).contains("Helm 3 Releases [1]"));

    app.active_view = ActiveView::Toolbox(toolbox());
    let screen = wide(&mut app);
    assert!(screen.contains("SRElens Toolbox"), "{screen}");
    assert!(screen.contains("kubectl"), "{screen}");
    assert!(screen.contains("CopyPath"), "{screen}");
    assert!(narrow(&mut app).contains("Toolbox"));
}

#[tokio::test]
async fn the_overview_renders_cluster_health_with_gauges_and_hints() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Overview(OverviewViewState::with_data(overview_data()));

    let screen = wide(&mut app);
    assert!(
        screen.contains("Cluster Health & Resource Overview"),
        "{screen}"
    );
    assert!(screen.contains("Workload Health Distribution"), "{screen}");
    assert!(screen.contains("Summarise"), "{screen}");
    assert!(screen.contains("Refresh"), "{screen}");
    let screen = narrow(&mut app);
    assert!(screen.contains("Overview"), "{screen}");
}

#[tokio::test]
async fn the_assistant_settings_and_tree_views_render_with_their_hints() {
    isolate_ai_settings();
    let (mut app, _rx) = common::app().await;

    app.assistant_state
        .add_assistant_message("Cluster looks healthy.".into());
    app.active_view = ActiveView::Assistant;
    let screen = wide(&mut app);
    assert!(screen.contains("AI Assistant"), "{screen}");
    assert!(screen.contains("Cluster looks healthy."), "{screen}");
    assert!(screen.contains("Cmd"), "{screen}");
    assert!(
        !screen.contains("Filter"),
        "assistant hides the filter hint:\n{screen}"
    );
    assert!(narrow(&mut app).contains("Cluster looks healthy."));

    app.switch_view_to_kind(ResourceKind::Settings).await;
    let screen = wide(&mut app);
    assert!(screen.contains("AI Settings"), "{screen}");
    assert!(screen.contains("Anthropic"), "{screen}");
    assert!(narrow(&mut app).contains("SRElens AI & Assistant Settings"));

    app.active_view = ActiveView::Tree(tree_view());
    let screen = wide(&mut app);
    assert!(
        screen.contains("Resource Relationship Tree: Deployment/web"),
        "{screen}"
    );
    assert!(screen.contains("web-abc-1"), "{screen}");
    assert!(screen.contains("Jump"), "{screen}");
    assert!(screen.contains("Actions"), "{screen}");
    assert!(narrow(&mut app).contains("Relationship Tree"));

    let mut failed = TreeViewState::new("Pod".into(), "gone".into(), None);
    failed.set_error("not found".into());
    app.active_view = ActiveView::Tree(failed);
    let screen = wide(&mut app);
    assert!(screen.contains("not found"), "{screen}");
}

#[tokio::test]
async fn the_node_inspector_renders_loading_error_and_detail_states() {
    let (mut app, _rx) = common::app().await;

    app.active_view = ActiveView::NodeInspector(NodeInspectorState::new("gpu-1".into()));
    let screen = wide(&mut app);
    assert!(screen.contains("Node Inspector: gpu-1"), "{screen}");
    assert!(screen.contains("Cordon"), "{screen}");
    assert!(screen.contains("PodDesc"), "{screen}");

    let mut failed = NodeInspectorState::new("gpu-1".into());
    failed.set_error("node not found".into());
    app.active_view = ActiveView::NodeInspector(failed);
    let screen = wide(&mut app);
    assert!(screen.contains("Node Inspector Error: gpu-1"), "{screen}");

    app.active_view = ActiveView::NodeInspector(inspector_with_details("gpu-1", true));
    let screen = wide(&mut app);
    assert!(
        screen.contains("Uncordon"),
        "cordoned node offers uncordon:\n{screen}"
    );
    assert!(screen.contains("api-0"), "{screen}");
    assert!(screen.contains("Health Conditions & Taints"), "{screen}");
    // The detail layout titles the panel with the node name, not "Inspector".
    let screen = narrow(&mut app);
    assert!(screen.contains("Node: gpu-1"), "{screen}");
    assert!(screen.contains("CORDONED"), "{screen}");
    assert!(screen.contains("Scheduled Pods (2 Total)"), "{screen}");
}

#[tokio::test]
async fn every_modal_variant_renders_over_the_view() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Pods,
        vec![pod("web-0", "default")],
    ));

    app.modal = Some(Modal::Confirm {
        title: "Delete Pod".into(),
        message: "Really delete web-0?".into(),
        action_name: "delete:Pod:default:web-0".into(),
        is_destructive: true,
    });
    let screen = wide(&mut app);
    assert!(screen.contains("Delete Pod"), "{screen}");
    assert!(screen.contains("Really delete web-0?"), "{screen}");

    app.modal = Some(Modal::Scale {
        workload_name: "web".into(),
        current_replicas: 2,
        input: "5".into(),
    });
    assert!(wide(&mut app).contains("Scale Workload: web"));

    app.modal = Some(Modal::PortForward {
        pod_name: "web-0".into(),
        namespace: "default".into(),
        container_port: 80,
        local_port_input: "8080".into(),
    });
    assert!(wide(&mut app).contains("Start Port Forward: web-0 (default)"));

    app.modal = Some(Modal::ContainerPicker {
        pod_name: "web-0".into(),
        namespace: Some("default".into()),
        containers: vec!["app".into(), "sidecar".into()],
        selected_idx: 1,
        action: ContainerAction::Shell,
    });
    let screen = wide(&mut app);
    assert!(screen.contains("sidecar"), "{screen}");

    app.open_context_picker();
    assert!(wide(&mut app).contains("Switch Kubernetes Context"));

    app.modal = Some(Modal::NamespacePicker {
        namespaces: vec!["default".into(), "kube-system".into()],
        current_namespace: "default".into(),
        selected_idx: 0,
        filter: "kube".into(),
    });
    let screen = wide(&mut app);
    assert!(screen.contains("Switch Namespace"), "{screen}");
    assert!(screen.contains("kube-system"), "{screen}");

    app.open_action_palette("Pod".into(), "web-0".into(), Some("default".into()));
    let screen = wide(&mut app);
    assert!(screen.contains("Actions: Pod/web-0 (default)"), "{screen}");
    assert!(screen.contains("View Live Logs"), "{screen}");

    let samples = vec![sample(1_000, 100, 200), sample(2_000, 150, 250)];
    app.modal = Some(Modal::MetricsTimeline(MetricsPanelState::new(
        "Pod".into(),
        "web-0".into(),
        Some("default".into()),
        samples,
    )));
    assert!(wide(&mut app).contains("Live Metrics Timeline"));

    let events = vec![
        json!({ "reason": "BackOff", "type": "Warning" }),
        json!({ "reason": "Pulled", "type": "Normal" }),
    ];
    app.modal = Some(Modal::ReasonRail {
        tallies: srelens_tui::views::reason_rail::tally_event_reasons(&events),
        selected_idx: 0,
        active_filter: Some("BackOff".into()),
    });
    let screen = wide(&mut app);
    assert!(screen.contains("BackOff"), "{screen}");

    app.modal = None;
    app.show_help = true;
    assert!(wide(&mut app).contains("Keybindings Cheat Sheet"));
}

#[tokio::test]
async fn the_status_bar_reflects_input_mode_toast_and_filter() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Pods,
        vec![pod("web-0", "default"), pod("db-0", "default")],
    ));

    app.input_mode = InputMode::Command;
    app.command_buffer = "dep".into();
    let screen = wide(&mut app);
    assert!(
        screen.contains("deployments"),
        "command suggestions are rendered:\n{screen}"
    );

    app.input_mode = InputMode::Filter;
    app.filter_buffer = "web".into();
    app.apply_current_filter();
    let screen = wide(&mut app);
    assert!(screen.contains("web"), "{screen}");

    app.input_mode = InputMode::Normal;
    app.set_toast("Copied something".into(), ratatui::style::Style::default());
    let screen = wide(&mut app);
    assert!(screen.contains("Copied something"), "{screen}");
    assert!(
        screen.contains("[1/2]"),
        "filter match count is shown:\n{screen}"
    );
}

#[tokio::test]
async fn rendering_with_a_screen_selection_captures_the_highlighted_text() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Pods,
        vec![pod("web-0", "default")],
    ));

    let screen = wide(&mut app);
    let row = screen
        .lines()
        .position(|l| l.contains("web-0"))
        .expect("pod row is rendered") as u16;
    let col = cell_col(screen.lines().nth(row as usize).unwrap(), "web-0");

    app.screen_selection = Some(((col, row), (col + 4, row)));
    wide(&mut app);
    assert_eq!(app.screen_selection_text.borrow().as_str(), "web-0");
}

// ---------------------------------------------------------------------------
// Mouse handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn clicking_and_scrolling_a_table_moves_the_selection() {
    let (mut app, _rx) = common::app().await;
    let items: Vec<Value> = (0..6)
        .map(|i| pod(&format!("pod-{i}"), "default"))
        .collect();
    app.active_view = ActiveView::Table(table_with(ResourceKind::Pods, items));
    wide(&mut app);
    let vp = match &app.active_view {
        ActiveView::Table(t) => t.last_viewport_rect.get(),
        _ => unreachable!(),
    };

    app.handle_mouse(common::click(vp.x + 2, vp.y + 2 + 1))
        .await;
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 1);

    app.handle_mouse(common::mouse(
        MouseEventKind::ScrollDown,
        vp.x + 2,
        vp.y + 3,
    ))
    .await;
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 4);

    app.handle_mouse(common::mouse(MouseEventKind::ScrollUp, vp.x + 2, vp.y + 3))
        .await;
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 1);

    // Clicking the header rows leaves the selection alone.
    app.handle_mouse(common::click(vp.x + 2, vp.y)).await;
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 1);
}

#[tokio::test]
async fn dragging_in_the_yaml_view_copies_the_selected_lines() {
    let (mut app, _rx) = common::app().await;
    let content: String = (0..12).map(|i| format!("line-{i}\n")).collect();
    app.active_view = ActiveView::Yaml(YamlViewState::new(
        "web-0".into(),
        "Pod".into(),
        None,
        content,
    ));
    wide(&mut app);
    let vp = match &app.active_view {
        ActiveView::Yaml(y) => y.last_viewport_rect.get(),
        _ => unreachable!(),
    };

    app.handle_mouse(common::click(vp.x + 1, vp.y)).await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Drag(MouseButton::Left),
        vp.x + 1,
        vp.y + 1,
    ))
    .await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Up(MouseButton::Left),
        vp.x + 1,
        vp.y + 1,
    ))
    .await;
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.selected_text().as_deref(), Some("line-0\nline-1"));
    assert!(
        app.screen_selection.is_none(),
        "yaml keeps its own selection"
    );

    app.handle_mouse(common::mouse(MouseEventKind::ScrollDown, vp.x + 1, vp.y))
        .await;
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.scroll_offset, 3);
    app.handle_mouse(common::mouse(MouseEventKind::ScrollUp, vp.x + 1, vp.y))
        .await;
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.scroll_offset, 0);

    // A click outside the viewport clears the selection.
    app.handle_mouse(common::click(0, 0)).await;
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert!(y.selection.is_none());
}

#[tokio::test]
async fn mouse_in_the_assistant_toggles_tool_chips_and_selects_text() {
    let (mut app, _rx) = common::app().await;
    app.assistant_state
        .add_assistant_message("alpha beta gamma".into());
    app.active_view = ActiveView::Assistant;
    wide(&mut app);
    let vp = app.assistant_state.last_viewport_rect.get();
    let line = app
        .assistant_state
        .plain_lines
        .borrow()
        .iter()
        .position(|l| l.contains("alpha beta gamma"))
        .expect("message line is rendered") as u16;

    // A click on a tool-chip line toggles expansion instead of selecting.
    app.assistant_state
        .tool_chip_lines
        .borrow_mut()
        .push(line as usize);
    let before = app.assistant_state.expand_tools;
    app.handle_mouse(common::click(vp.x + 1, vp.y + line)).await;
    assert_ne!(app.assistant_state.expand_tools, before);
    app.assistant_state.tool_chip_lines.borrow_mut().clear();

    app.handle_mouse(common::click(vp.x + 1, vp.y + line)).await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Drag(MouseButton::Left),
        vp.x + 6,
        vp.y + line,
    ))
    .await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Up(MouseButton::Left),
        vp.x + 6,
        vp.y + line,
    ))
    .await;
    assert_eq!(
        app.assistant_state.get_selected_text().as_deref(),
        Some("alpha")
    );
    assert!(
        app.screen_selection.is_none(),
        "assistant keeps its own selection"
    );

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");

    // Re-select, then click outside the viewport to clear.
    app.handle_mouse(common::click(vp.x + 1, vp.y + line)).await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Up(MouseButton::Left),
        vp.x + 4,
        vp.y + line,
    ))
    .await;
    assert!(app.assistant_state.selection.is_some());
    app.handle_mouse(common::click(0, 0)).await;
    assert!(app.assistant_state.selection.is_none());
}

#[tokio::test]
async fn mouse_in_the_node_inspector_pod_table_selects_and_scrolls_pods() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::NodeInspector(inspector_with_details("gpu-1", false));
    wide(&mut app);
    let vp = match &app.active_view {
        ActiveView::NodeInspector(ni) => ni.last_pods_table_rect.get(),
        _ => unreachable!(),
    };
    assert!(vp.height > 0, "pods table was laid out");

    app.handle_mouse(common::click(vp.x + 1, vp.y + 1 + 1))
        .await;
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert_eq!(ni.selected_pod_idx, 1);

    app.handle_mouse(common::mouse(MouseEventKind::ScrollUp, vp.x + 1, vp.y + 1))
        .await;
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert_eq!(ni.selected_pod_idx, 0);

    app.handle_mouse(common::mouse(
        MouseEventKind::ScrollDown,
        vp.x + 1,
        vp.y + 1,
    ))
    .await;
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert_eq!(ni.selected_pod_idx, 1);
}

#[tokio::test]
async fn scroll_wheel_moves_tree_logs_and_describe_views() {
    let (mut app, _rx) = common::app().await;

    app.active_view = ActiveView::Tree(tree_view());
    app.handle_mouse(common::mouse(MouseEventKind::ScrollDown, 10, 10))
        .await;
    let ActiveView::Tree(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 1);
    app.handle_mouse(common::mouse(MouseEventKind::ScrollUp, 10, 10))
        .await;
    let ActiveView::Tree(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 0);

    let mut logs = LogsViewState::new("web".into(), "default".into(), None, "ch".into());
    for i in 0..20 {
        logs.push_line(format!("l{i}"));
    }
    logs.follow = false;
    logs.scroll_offset = 0;
    app.active_view = ActiveView::Logs(logs);
    app.handle_mouse(common::mouse(MouseEventKind::ScrollDown, 10, 10))
        .await;
    let ActiveView::Logs(l) = &app.active_view else {
        panic!()
    };
    assert_eq!(l.scroll_offset, 3);
    app.handle_mouse(common::mouse(MouseEventKind::ScrollUp, 10, 10))
        .await;
    let ActiveView::Logs(l) = &app.active_view else {
        panic!()
    };
    assert_eq!(l.scroll_offset, 0);

    let content: String = (0..20).map(|i| format!("d{i}\n")).collect();
    app.active_view = ActiveView::Describe(DescribeViewState::new(
        "web".into(),
        "Pod".into(),
        None,
        content,
    ));
    app.handle_mouse(common::mouse(MouseEventKind::ScrollDown, 10, 10))
        .await;
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert_eq!(d.scroll_offset, 3);
    app.handle_mouse(common::mouse(MouseEventKind::ScrollUp, 10, 10))
        .await;
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert_eq!(d.scroll_offset, 0);
}

#[tokio::test]
async fn a_drag_over_visible_text_copies_it_and_an_empty_drag_is_dropped() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Pods,
        vec![pod("web-0", "default")],
    ));
    let screen = wide(&mut app);
    let row = screen.lines().position(|l| l.contains("web-0")).unwrap() as u16;
    let col = cell_col(screen.lines().nth(row as usize).unwrap(), "web-0");

    app.handle_mouse(common::click(col, row)).await;
    assert!(app.screen_selecting);
    app.handle_mouse(common::mouse(
        MouseEventKind::Drag(MouseButton::Left),
        col + 4,
        row,
    ))
    .await;
    wide(&mut app);
    app.handle_mouse(common::mouse(
        MouseEventKind::Up(MouseButton::Left),
        col + 4,
        row,
    ))
    .await;
    assert!(!app.screen_selecting);
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");
    assert!(
        app.screen_selection.is_some(),
        "highlight stays until the next key"
    );
    assert_eq!(app.screen_selection_text.borrow().as_str(), "web-0");

    // A key press dismisses the highlight.
    app.handle_key_event(common::ch('j')).await;
    assert!(app.screen_selection.is_none());

    // Press and release on the same cell selects nothing.
    app.toast = None;
    app.handle_mouse(common::click(col, row)).await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Up(MouseButton::Left),
        col,
        row,
    ))
    .await;
    assert!(app.screen_selection.is_none());
    assert!(app.toast.is_none());

    // A drag over blank cells is discarded without a toast.
    app.handle_mouse(common::click(5, 40)).await;
    app.handle_mouse(common::mouse(
        MouseEventKind::Drag(MouseButton::Left),
        30,
        40,
    ))
    .await;
    wide(&mut app);
    app.handle_mouse(common::mouse(MouseEventKind::Up(MouseButton::Left), 30, 40))
        .await;
    assert!(app.screen_selection.is_none());
    assert!(app.toast.is_none());

    // A drag without a prior press is ignored.
    app.handle_mouse(common::mouse(
        MouseEventKind::Drag(MouseButton::Left),
        30,
        41,
    ))
    .await;
    assert!(app.screen_selection.is_none());
}

#[tokio::test]
async fn clicking_a_context_chip_opens_the_picker_or_switches_context() {
    let (mut app, _rx) = common::app().await;
    *app.context_chip_rects.borrow_mut() = vec![
        (Rect::new(0, 1, 10, 1), ":ctx".into()),
        (Rect::new(12, 1, 10, 1), "staging".into()),
        (Rect::new(24, 1, 10, 1), "test-cluster".into()),
    ];

    app.handle_mouse(common::click(2, 1)).await;
    assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));
    app.modal = None;

    app.handle_mouse(common::click(26, 1)).await;
    assert_eq!(
        app.active_context, "test-cluster",
        "clicking the current chip is a no-op"
    );
    assert!(app.toast.is_none());

    app.handle_mouse(common::click(14, 1)).await;
    assert_eq!(app.active_context, "staging");
    assert_eq!(toast(&app), "Switched to context 'staging'");
}

// ---------------------------------------------------------------------------
// Per-view key handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn yaml_view_keys_scroll_copy_edit_and_share() {
    let (mut app, _rx) = common::app().await;
    let content: String = (0..40).map(|i| format!("k{i}: v\n")).collect();
    app.active_view = ActiveView::Yaml(YamlViewState::new(
        "web-0".into(),
        "Pod".into(),
        Some("default".into()),
        content,
    ));

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::key(KeyCode::Down)).await;
    app.handle_key_event(common::ctrl('d')).await;
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.scroll_offset, 12);
    app.handle_key_event(common::ctrl('u')).await;
    app.handle_key_event(common::ch('k')).await;
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.scroll_offset, 1);
    app.handle_key_event(common::ch('G')).await;
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert!(y.scroll_offset > 1);
    app.handle_key_event(common::ch('g')).await;
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.scroll_offset, 0);

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied YAML to clipboard");

    if let ActiveView::Yaml(y) = &mut app.active_view {
        y.start_selection(0);
        y.update_selection(2);
    }
    app.handle_key_event(common::ch('y')).await;
    assert_eq!(toast(&app), "✓ Copied selection to clipboard");

    app.handle_key_event(common::ctrl('y')).await;
    assert_eq!(
        toast(&app),
        "Copied deep link: srelens://resource/test-cluster/default/Pod/web-0"
    );

    app.handle_key_event(common::ch('e')).await;
    assert!(matches!(
        app.requires_terminal_suspend,
        Some(SuspendAction::EditYaml)
    ));
}

#[tokio::test]
async fn describe_view_keys_scroll_copy_and_share() {
    let (mut app, _rx) = common::app().await;
    let content: String = (0..40).map(|i| format!("Line {i}\n")).collect();
    app.active_view = ActiveView::Describe(DescribeViewState::new(
        "web".into(),
        "Deployment".into(),
        Some("prod".into()),
        content,
    ));

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::ctrl('d')).await;
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert_eq!(d.scroll_offset, 11);
    app.handle_key_event(common::ctrl('u')).await;
    app.handle_key_event(common::key(KeyCode::Up)).await;
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert_eq!(d.scroll_offset, 0);
    app.handle_key_event(common::key(KeyCode::End)).await;
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert!(d.scroll_offset > 0);
    app.handle_key_event(common::key(KeyCode::Home)).await;
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert_eq!(d.scroll_offset, 0);

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied describe output to clipboard");
    app.handle_key_event(common::ctrl('y')).await;
    assert_eq!(
        toast(&app),
        "Copied deep link: srelens://resource/test-cluster/prod/Deployment/web"
    );
}

#[tokio::test]
async fn logs_view_keys_toggle_flags_scroll_save_copy_and_share() {
    let (mut app, _rx) = common::app().await;
    let mut logs = LogsViewState::new(
        "web-0".into(),
        "default".into(),
        Some("app".into()),
        "ch".into(),
    );
    for i in 0..30 {
        logs.push_line(format!("log {i}"));
    }
    app.active_view = ActiveView::Logs(logs);

    let flags = |app: &App| match &app.active_view {
        ActiveView::Logs(l) => (l.follow, l.timestamps, l.previous, l.wrap),
        _ => unreachable!(),
    };
    let initial = flags(&app);
    app.handle_key_event(common::ch('f')).await;
    app.handle_key_event(common::ch('t')).await;
    app.handle_key_event(common::ch('p')).await;
    app.handle_key_event(common::ch('w')).await;
    let toggled = flags(&app);
    assert_eq!(toggled.0, !initial.0);
    assert_eq!(toggled.1, !initial.1);
    assert_eq!(toggled.2, !initial.2);
    assert_eq!(toggled.3, !initial.3);

    app.handle_key_event(common::ch('g')).await;
    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::key(KeyCode::Down)).await;
    app.handle_key_event(common::ch('k')).await;
    let ActiveView::Logs(l) = &app.active_view else {
        panic!()
    };
    assert_eq!(l.scroll_offset, 1);
    // `G` jumps to the last line; it scrolls, it does not re-arm following.
    app.handle_key_event(common::ch('G')).await;
    let ActiveView::Logs(l) = &app.active_view else {
        panic!()
    };
    assert_eq!(l.scroll_offset, l.lines.len() - 1);
    assert!(!l.follow);

    app.handle_key_event(common::ch('s')).await;
    assert!(toast(&app).starts_with("Logs saved to "), "{}", toast(&app));

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied 30 log lines to clipboard");
    app.handle_key_event(common::ctrl('y')).await;
    assert_eq!(
        toast(&app),
        "Copied deep link: srelens://resource/test-cluster/default/Pod/web-0"
    );
}

#[tokio::test]
async fn port_forward_view_keys_copy_share_and_stop_a_forward() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::PortForwards(port_forwards());

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::ch('k')).await;
    let ActiveView::PortForwards(pf) = &app.active_view else {
        panic!()
    };
    assert_eq!(pf.selected_idx, 0);

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied forward URL: http://127.0.0.1:8080");
    app.handle_key_event(common::ctrl('y')).await;
    assert_eq!(
        toast(&app),
        "Copied deep link: srelens://resource/test-cluster/default/Pod/web-0"
    );

    app.handle_key_event(common::ch('d')).await;
    match &app.modal {
        Some(Modal::Confirm {
            title,
            action_name,
            is_destructive,
            ..
        }) => {
            assert_eq!(title, "Stop Port Forward [127.0.0.1:8080]");
            assert_eq!(action_name, "stop-pf:pf-1");
            assert!(!is_destructive);
        }
        other => panic!("expected confirm modal, got {:?}", other.is_some()),
    }
    app.handle_key_event(common::ch('y')).await;
    assert!(app.modal.is_none());
    assert_eq!(toast(&app), "Port forward stopped");

    // With nothing selected the copy keys do nothing.
    app.active_view = ActiveView::PortForwards(PortForwardViewState::new());
    app.toast = None;
    app.handle_key_event(common::ch('c')).await;
    app.handle_key_event(common::ch('d')).await;
    assert!(app.toast.is_none());
    assert!(app.modal.is_none());
}

#[tokio::test]
async fn helm_view_keys_navigate_and_copy_release_links() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Helm(helm_releases());

    app.handle_key_event(common::key(KeyCode::Down)).await;
    app.handle_key_event(common::key(KeyCode::Up)).await;
    let ActiveView::Helm(h) = &app.active_view else {
        panic!()
    };
    assert_eq!(h.selected_idx, 0);

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(
        toast(&app),
        "Copied deep link: srelens://resource/test-cluster/default/HelmRelease/nginx"
    );
    app.toast = None;
    app.handle_key_event(common::ctrl('y')).await;
    assert_eq!(
        toast(&app),
        "Copied deep link: srelens://resource/test-cluster/default/HelmRelease/nginx"
    );

    app.active_view = ActiveView::Helm(HelmViewState::new());
    app.toast = None;
    app.handle_key_event(common::ch('c')).await;
    assert!(app.toast.is_none());
}

#[tokio::test]
async fn toolbox_keys_copy_the_tool_path_or_name() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Toolbox(toolbox());

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied tool info: /usr/local/bin/kubectl");

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::ch('y')).await;
    assert_eq!(toast(&app), "Copied tool info: helm");

    app.handle_key_event(common::ch('k')).await;
    let ActiveView::Toolbox(tb) = &app.active_view else {
        panic!()
    };
    assert_eq!(tb.selected_idx, 0);
}

#[tokio::test]
async fn overview_keys_refresh_copy_share_and_summarise() {
    let (mut app, _rx) = common::app().await;
    keyless_assistant(&mut app);
    app.active_view = ActiveView::Overview(OverviewViewState::with_data(overview_data()));

    app.handle_key_event(common::ch('r')).await;
    assert_eq!(toast(&app), "Refreshing cluster overview...");
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Copied cluster overview summary to clipboard");
    app.handle_key_event(common::ctrl('y')).await;
    assert_eq!(
        toast(&app),
        "Copied deep link: srelens://cluster/test-cluster"
    );

    app.handle_key_event(common::ch('s')).await;
    assert!(matches!(app.active_view, ActiveView::Assistant));
    assert!(matches!(
        app.nav_stack.last(),
        Some(ActiveView::Overview(_))
    ));
    assert_eq!(toast(&app), "Generating AI cluster health summary...");
    assert!(app
        .assistant_state
        .messages
        .iter()
        .any(|m| m.role == "user" && m.content == "Summarise the health of this cluster"));
    assert!(last_message(&app).contains("No API key configured for Google Gemini"));

    // A busy assistant refuses a second summary.
    app.active_view = ActiveView::Overview(OverviewViewState::with_data(overview_data()));
    app.assistant_state.is_busy = true;
    app.handle_key_event(common::ctrl('a')).await;
    assert!(matches!(app.active_view, ActiveView::Overview(_)));
    assert!(toast(&app).contains("Assistant is busy"));
}

#[tokio::test]
async fn assistant_editing_keys_shape_the_input_buffer() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Assistant;

    common::type_str(&mut app, "hello world").await;
    assert_eq!(app.assistant_state.input, "hello world");
    app.handle_key_event(common::key(KeyCode::Backspace)).await;
    assert_eq!(app.assistant_state.input, "hello worl");
    app.handle_key_event(common::ctrl('w')).await;
    assert_eq!(app.assistant_state.input, "hello ");
    // Ctrl+u is bound to scrolling here; Alt+u is what clears the composer.
    app.handle_key_event(common::ctrl('u')).await;
    assert_eq!(app.assistant_state.input, "hello ");
    app.handle_key_event(alt('u')).await;
    assert_eq!(app.assistant_state.input, "");

    // Ctrl+v appends the clipboard when there is one; either way the buffer
    // stays a single line.
    app.handle_key_event(common::ctrl('v')).await;
    assert!(!app.assistant_state.input.contains('\n'));
    app.assistant_state.input.clear();

    // 'c' with no assistant answer at all is plain typing.
    app.assistant_state.messages.clear();
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(app.assistant_state.input, "c");
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(app.assistant_state.input, "cc");

    // 'c' on an empty input copies the last assistant answer.
    app.assistant_state.input.clear();
    app.assistant_state
        .add_assistant_message("the answer".into());
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied assistant answer to clipboard");
    assert_eq!(app.assistant_state.input, "");

    // ... but with text in the buffer it is still typing.
    common::type_str(&mut app, "ab").await;
    app.handle_key_event(common::ch('c')).await;
    assert_eq!(app.assistant_state.input, "abc");

    // Ctrl+t toggles the tool chip expansion, Ctrl+l clears the conversation.
    let expanded = app.assistant_state.expand_tools;
    app.handle_key_event(common::ctrl('t')).await;
    assert_ne!(app.assistant_state.expand_tools, expanded);
    app.handle_key_event(common::ctrl('l')).await;
    assert_only_greeting(&app);
    assert_eq!(toast(&app), "✓ Conversation cleared");
}

#[tokio::test]
async fn assistant_history_and_slash_suggestions_drive_the_arrow_keys() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Assistant;
    app.assistant_state.prompt_history = vec!["first".into(), "second".into()];
    app.assistant_state.input = "draft".into();

    app.handle_key_event(common::key(KeyCode::Up)).await;
    assert_eq!(app.assistant_state.input, "second");
    app.handle_key_event(common::key(KeyCode::Up)).await;
    assert_eq!(app.assistant_state.input, "first");
    app.handle_key_event(common::key(KeyCode::Down)).await;
    assert_eq!(app.assistant_state.input, "second");
    app.handle_key_event(common::key(KeyCode::Down)).await;
    assert_eq!(app.assistant_state.input, "draft");

    app.assistant_state.input.clear();
    common::type_str(&mut app, "/").await;
    let n = app.assistant_state.slash_suggestions.len();
    assert!(n > 1, "slash prefix lists suggestions");
    app.handle_key_event(common::key(KeyCode::Down)).await;
    assert_eq!(app.assistant_state.slash_suggestion_idx, 1);
    app.handle_key_event(common::key(KeyCode::Up)).await;
    assert_eq!(app.assistant_state.slash_suggestion_idx, 0);
    app.handle_key_event(common::key(KeyCode::Up)).await;
    assert_eq!(app.assistant_state.slash_suggestion_idx, n - 1);

    // Tab applies the highlighted suggestion.
    app.handle_key_event(common::key(KeyCode::Tab)).await;
    assert!(app.assistant_state.input.starts_with('/'));
    assert!(app.assistant_state.input.ends_with(' '));
    assert!(app.assistant_state.slash_suggestions.is_empty());

    // Enter on a bare prefix with suggestions completes instead of submitting.
    app.assistant_state.input.clear();
    let before = app.assistant_state.messages.len();
    common::type_str(&mut app, "/cra").await;
    assert!(!app.assistant_state.slash_suggestions.is_empty());
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.input, "/crashloop ");
    assert_eq!(
        app.assistant_state.messages.len(),
        before,
        "nothing was submitted"
    );
}

#[tokio::test]
async fn assistant_scroll_keys_move_the_viewport() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Assistant;
    for i in 0..80 {
        app.assistant_state
            .add_assistant_message(format!("message number {i}"));
    }
    narrow(&mut app);
    let max = app.assistant_state.last_max_scroll.get();
    assert!(max > 20, "enough content to scroll: {max}");

    app.handle_key_event(common::key(KeyCode::PageUp)).await;
    assert!(!app.assistant_state.auto_scroll);
    assert_eq!(app.assistant_state.scroll_offset, max - 10);
    app.handle_key_event(common::ctrl('k')).await;
    assert_eq!(app.assistant_state.scroll_offset, max - 12);
    app.handle_key_event(common::ctrl('u')).await;
    assert_eq!(app.assistant_state.scroll_offset, max - 22);
    app.handle_key_event(common::ctrl('j')).await;
    assert_eq!(app.assistant_state.scroll_offset, max - 20);
    app.handle_key_event(common::ctrl('d')).await;
    assert_eq!(app.assistant_state.scroll_offset, max - 10);
    app.handle_key_event(common::key(KeyCode::PageDown)).await;
    assert!(app.assistant_state.auto_scroll);
    app.handle_key_event(common::key(KeyCode::Home)).await;
    assert_eq!(app.assistant_state.scroll_offset, 0);
    app.handle_key_event(common::key(KeyCode::End)).await;
    assert!(app.assistant_state.auto_scroll);
}

#[tokio::test]
async fn submitting_a_query_without_an_api_key_answers_with_setup_guidance() {
    let (mut app, _rx) = common::app().await;
    keyless_assistant(&mut app);
    app.active_view = ActiveView::Assistant;

    // Empty input submits nothing: the seeded greeting is all there is.
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_only_greeting(&app);

    compose(&mut app, "why is web-0 crashing?").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.input, "");
    assert_eq!(app.assistant_state.messages[1].role, "user");
    assert_eq!(
        app.assistant_state.messages[1].content,
        "why is web-0 crashing?"
    );
    assert!(last_message(&app).contains("No API key configured for Google Gemini"));
    assert!(last_message(&app).contains("GEMINI_API_KEY"));
    assert!(!app.assistant_state.is_busy);

    // A busy assistant rejects a second query with a toast.
    app.assistant_state.is_busy = true;
    compose(&mut app, "again").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert!(toast(&app).contains("Assistant is busy"));
    assert_eq!(app.assistant_state.input, "again");
}

#[tokio::test]
async fn a_configured_provider_starts_a_native_agent_turn() {
    let (mut app, _rx) = common::app().await;
    let mut settings = AiSettings::default();
    settings.default_provider = AiProvider::OpenAiCompatible;
    // A closed loopback port: the spawned turn fails to connect instead of
    // talking to anything real.
    settings
        .base_urls
        .insert("openai-compatible".into(), "http://127.0.0.1:9/v1".into());
    app.ai_settings = settings;
    app.assistant_state.caveman_level = None;
    app.active_view = ActiveView::Assistant;

    compose(&mut app, "list pods").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;

    assert!(app.assistant_state.is_busy, "a turn is in flight");
    assert_eq!(
        app.assistant_state.messages.len(),
        2,
        "greeting plus the query"
    );
    let last = app.assistant_state.messages.last().unwrap();
    assert_eq!(last.role, "user");
    assert_eq!(last.content, "list pods");
    assert_eq!(
        app.assistant_state.prompt_history,
        vec!["list pods".to_string()]
    );
}

#[tokio::test]
async fn caveman_phrases_and_slash_commands_switch_the_terse_mode() {
    isolate_ai_settings();
    let (mut app, _rx) = common::app().await;
    keyless_assistant(&mut app);
    app.active_view = ActiveView::Assistant;

    compose(&mut app, "caveman mode").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.caveman_level, Some(CavemanLevel::Full));
    assert_eq!(toast(&app), "🦖 Caveman mode: full");
    assert!(last_message(&app).contains("Caveman mode active"));

    compose(&mut app, "stop caveman").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.caveman_level, None);
    assert_eq!(toast(&app), "Caveman mode disabled");

    // "/caveman" with no level reports status when active, activates otherwise.
    compose(&mut app, "/caveman").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.caveman_level, Some(CavemanLevel::Full));
    assert!(last_message(&app).contains("activated"));
    compose(&mut app, "/caveman").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert!(last_message(&app).contains("currently active"));

    compose(&mut app, "/caveman ultra").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.caveman_level, Some(CavemanLevel::Ultra));
    assert_eq!(toast(&app), "🦖 Caveman mode: ultra");

    compose(&mut app, "/caveman off").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.caveman_level, None);

    // A level followed by a question sets the level and submits the question.
    let before = app.assistant_state.messages.len();
    compose(&mut app, "/caveman lite why is web-0 pending").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.caveman_level, Some(CavemanLevel::Lite));
    let user = &app.assistant_state.messages[before];
    assert_eq!(user.role, "user");
    assert_eq!(user.content, "why is web-0 pending");
    assert!(last_message(&app).contains("No API key"));
}

#[tokio::test]
async fn utility_slash_commands_clear_open_settings_and_expand_playbooks() {
    isolate_ai_settings();
    let (mut app, _rx) = common::app().await;
    keyless_assistant(&mut app);
    app.active_view = ActiveView::Assistant;
    app.assistant_state.add_assistant_message("old".into());

    compose(&mut app, "/clear").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_only_greeting(&app);
    assert_eq!(toast(&app), "✓ Conversation cleared");

    compose(&mut app, "/crashloop web-0").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.assistant_state.messages[1].role, "user");
    assert_eq!(app.assistant_state.messages[1].content, "/crashloop web-0");
    assert!(last_message(&app).contains("No API key"));

    compose(&mut app, "/settings").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Settings(_)));
    assert!(matches!(app.nav_stack.last(), Some(ActiveView::Assistant)));

    // Ctrl+s from the assistant also opens settings.
    app.active_view = ActiveView::Assistant;
    app.handle_key_event(common::ctrl('s')).await;
    assert!(matches!(app.active_view, ActiveView::Settings(_)));
}

#[tokio::test]
async fn settings_keys_navigate_toggle_edit_and_save() {
    isolate_ai_settings();
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Settings).await;
    // The view opens on whichever provider the stored settings name; start
    // from a known row so the moves below are checkable.
    if let ActiveView::Settings(s) = &mut app.active_view {
        s.selected_provider_idx = 0;
    }

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::key(KeyCode::Down)).await;
    app.handle_key_event(common::ch('k')).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert_eq!(s.selected_provider_idx, 1);

    app.handle_key_event(common::ch('l')).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    let field_after_next = s.selected_field;
    app.handle_key_event(common::ch('h')).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert_ne!(s.selected_field, field_after_next);
    app.handle_key_event(common::key(KeyCode::Tab)).await;
    app.handle_key_event(common::key(KeyCode::BackTab)).await;

    app.handle_key_event(common::ch(' ')).await;
    assert!(
        toast(&app).starts_with("✓ Active provider set to "),
        "{}",
        toast(&app)
    );
    assert_eq!(app.ai_settings.default_provider, AiProvider::OpenAi);

    app.handle_key_event(common::ch('s')).await;
    assert!(
        toast(&app).starts_with("Saved AI settings to "),
        "{}",
        toast(&app)
    );

    // Editing a field: the provider toggle row has nothing to edit, so move
    // onto the API key first, then type, word-delete, backspace, cancel and
    // commit.
    app.handle_key_event(common::ch('e')).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert!(!s.is_editing, "the provider toggle row is not editable");
    app.handle_key_event(common::key(KeyCode::Tab)).await;
    app.handle_key_event(common::ch('e')).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert!(s.is_editing);
    common::type_str(&mut app, "sk-test key").await;
    app.handle_key_event(common::ctrl('w')).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert_eq!(s.edit_buffer, "sk-test ");
    app.handle_key_event(common::key(KeyCode::Backspace)).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert_eq!(s.edit_buffer, "sk-test");
    app.handle_key_event(common::ctrl('v')).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert!(!s.edit_buffer.contains('\n'));
    app.handle_key_event(common::key(KeyCode::Esc)).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert!(!s.is_editing);

    app.handle_key_event(common::key(KeyCode::Enter)).await;
    common::type_str(&mut app, "value").await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert!(!s.is_editing);
    assert_eq!(toast(&app), "✓ Setting updated and saved");

    // 'q' returns to the previous view; with no history it opens Pods.
    app.handle_key_event(common::ch('q')).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));
    app.active_view =
        ActiveView::Settings(srelens_tui::views::settings_view::SettingsViewState::new());
    app.nav_stack.clear();
    app.handle_key_event(common::key(KeyCode::Esc)).await;
    assert!(matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Pods));
}

#[tokio::test]
async fn tree_keys_move_copy_and_open_logs_or_actions() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Tree(tree_view());

    app.handle_key_event(common::ch('G')).await;
    let ActiveView::Tree(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 2);
    app.handle_key_event(common::ch('k')).await;
    app.handle_key_event(common::ch('g')).await;
    app.handle_key_event(common::ch('j')).await;
    let ActiveView::Tree(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.selected_idx, 1);
    assert_eq!(t.selected_node().unwrap().name, "web-abc");

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied 'web-abc' to clipboard");
    app.handle_key_event(common::shift(KeyCode::Char('C')))
        .await;
    assert_eq!(
        toast(&app),
        "✓ Copied resource relationship tree to clipboard"
    );

    app.handle_key_event(common::ch('l')).await;
    assert_eq!(
        toast(&app),
        "Logs only available for Pods (selected ReplicaSet)"
    );

    app.handle_key_event(common::ch('x')).await;
    match &app.modal {
        Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            namespace,
            ..
        }) => {
            assert_eq!(resource_kind, "ReplicaSet");
            assert_eq!(resource_name, "web-abc");
            assert_eq!(namespace.as_deref(), Some("default"));
        }
        _ => panic!("expected action palette"),
    }
    app.modal = None;

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::ch('l')).await;
    match &app.active_view {
        ActiveView::Logs(l) => {
            assert_eq!(l.pod_name, "web-abc-1");
            assert_eq!(l.namespace, "default");
        }
        _ => panic!("pod logs open from the tree"),
    }
}

#[tokio::test]
async fn node_inspector_keys_navigate_pods_and_offer_node_actions() {
    let (mut app, _rx) = common::app().await;
    app.nav_stack
        .push(ActiveView::Table(table_with(ResourceKind::Nodes, vec![])));
    app.active_view = ActiveView::NodeInspector(inspector_with_details("gpu-1", false));

    app.handle_key_event(common::ch('j')).await;
    app.handle_key_event(common::ch('k')).await;
    app.handle_key_event(common::ch('G')).await;
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert_eq!(ni.selected_pod_idx, 1);
    app.handle_key_event(common::ch('g')).await;
    app.handle_key_event(common::ctrl('d')).await;
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert_eq!(ni.selected_pod_idx, 1);
    app.handle_key_event(common::ctrl('u')).await;
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert_eq!(ni.selected_pod_idx, 0);

    app.handle_key_event(common::ch('s')).await;
    assert_eq!(
        toast(&app),
        "Node debug command: kubectl debug node/gpu-1 -it --image=busybox"
    );

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Cordoning node 'gpu-1'...");

    app.handle_key_event(common::ch('x')).await;
    match &app.modal {
        Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            namespace,
            ..
        }) => {
            assert_eq!(resource_kind, "Pod");
            assert_eq!(resource_name, "api-0");
            assert_eq!(namespace.as_deref(), Some("default"));
        }
        _ => panic!("expected pod action palette"),
    }
    app.modal = None;

    // Refreshing re-opens the inspector for the same node.
    app.handle_key_event(common::ch('r')).await;
    match &app.active_view {
        ActiveView::NodeInspector(ni) => {
            assert_eq!(ni.node_name, "gpu-1");
            assert!(ni.is_loading);
        }
        _ => panic!("still the inspector"),
    }

    // Escape pops back to the previous view.
    app.handle_key_event(common::key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::NodeInspector(_)));
    app.handle_key_event(common::ch('q')).await;
    assert!(matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Nodes));

    // With no history, leaving opens the Nodes table.
    app.nav_stack.clear();
    app.active_view = ActiveView::NodeInspector(NodeInspectorState::new("gpu-1".into()));
    app.handle_key_event(common::ch('q')).await;
    assert!(matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Nodes));
}

#[tokio::test]
async fn node_inspector_without_pods_targets_the_node_and_toggles_uncordon() {
    let (mut app, _rx) = common::app().await;
    let mut details = node_details("gpu-1", true);
    details.pods.clear();
    let mut ni = NodeInspectorState::new("gpu-1".into());
    ni.set_details(details);
    app.active_view = ActiveView::NodeInspector(ni);

    app.handle_key_event(common::ch('c')).await;
    assert_eq!(toast(&app), "Uncordoning node 'gpu-1'...");

    app.handle_key_event(common::ch('x')).await;
    match &app.modal {
        Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            namespace,
            ..
        }) => {
            assert_eq!(resource_kind, "Node");
            assert_eq!(resource_name, "gpu-1");
            assert!(namespace.is_none());
        }
        _ => panic!("expected node action palette"),
    }
    app.modal = None;

    // Enter and 'l' need a highlighted pod; without one they are no-ops.
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    app.handle_key_event(common::ch('l')).await;
    assert!(matches!(app.active_view, ActiveView::NodeInspector(_)));
}

#[tokio::test]
async fn node_inspector_enter_and_l_jump_to_the_highlighted_pod() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::NodeInspector(inspector_with_details("gpu-1", false));
    app.handle_key_event(common::ch('j')).await;

    app.handle_key_event(common::ch('l')).await;
    match &app.active_view {
        ActiveView::Logs(l) => {
            assert_eq!(l.pod_name, "exporter");
            assert_eq!(l.namespace, "monitoring");
        }
        _ => panic!("logs open for the highlighted pod"),
    }
    app.handle_key_event(common::key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::NodeInspector(_)));

    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(app.active_namespace, "monitoring");
    assert_eq!(app.filter_buffer, "exporter");
    assert!(matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Pods));
    assert_eq!(toast(&app), "Jumped to Pod 'exporter'");
}

// ---------------------------------------------------------------------------
// Filters, paste and colon commands
// ---------------------------------------------------------------------------

#[tokio::test]
async fn filters_apply_and_clear_in_every_text_view() {
    let (mut app, _rx) = common::app().await;

    app.active_view = ActiveView::Describe(DescribeViewState::new(
        "w".into(),
        "Pod".into(),
        None,
        "alpha\nbeta\nalpha again\n".into(),
    ));
    app.filter_buffer = "alpha".into();
    app.apply_current_filter();
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert_eq!(d.search_matches.len(), 2);
    app.clear_current_filter();
    let ActiveView::Describe(d) = &app.active_view else {
        panic!()
    };
    assert!(d.search_matches.is_empty());
    assert!(app.filter_buffer.is_empty());

    app.active_view = ActiveView::Yaml(YamlViewState::new(
        "w".into(),
        "Pod".into(),
        None,
        "a: 1\nb: 2\na: 3\n".into(),
    ));
    app.filter_buffer = "a:".into();
    app.apply_current_filter();
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.search_matches.len(), 2);
    app.clear_current_filter();
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert!(y.search_query.is_empty());

    let mut logs = LogsViewState::new("w".into(), "default".into(), None, "ch".into());
    logs.push_line("ERROR one".into());
    logs.push_line("info".into());
    app.active_view = ActiveView::Logs(logs);
    app.filter_buffer = "ERROR".into();
    app.apply_current_filter();
    let ActiveView::Logs(l) = &app.active_view else {
        panic!()
    };
    assert_eq!(l.search_matches.len(), 1);
    app.clear_current_filter();
    let ActiveView::Logs(l) = &app.active_view else {
        panic!()
    };
    assert!(l.search_matches.is_empty());

    // Views without a filter are untouched.
    app.active_view = ActiveView::Assistant;
    app.filter_buffer = "x".into();
    app.apply_current_filter();
    app.clear_current_filter();
    assert!(app.filter_buffer.is_empty());
}

#[tokio::test]
async fn pasted_text_lands_in_the_active_input() {
    let (mut app, _rx) = common::app().await;

    app.input_mode = InputMode::Command;
    app.handle_paste("po\r\nds".into());
    assert_eq!(app.command_buffer, "po ds");

    app.input_mode = InputMode::Filter;
    app.active_view = ActiveView::Yaml(YamlViewState::new(
        "w".into(),
        "Pod".into(),
        None,
        "web\nx\n".into(),
    ));
    app.handle_paste("we\nb".into());
    assert_eq!(app.filter_buffer, "web");
    let ActiveView::Yaml(y) = &app.active_view else {
        panic!()
    };
    assert_eq!(y.search_matches.len(), 1);

    app.input_mode = InputMode::Normal;
    app.active_view = ActiveView::Assistant;
    app.handle_paste("multi\nline".into());
    assert_eq!(app.assistant_state.input, "multi line");

    let mut settings = srelens_tui::views::settings_view::SettingsViewState::new();
    settings.is_editing = true;
    app.active_view = ActiveView::Settings(settings);
    app.handle_paste("sk-\nabc".into());
    let ActiveView::Settings(s) = &app.active_view else {
        panic!()
    };
    assert_eq!(s.edit_buffer, "sk-abc");

    app.active_view = ActiveView::Table(ResourceTableState::new(ResourceKind::Pods));
    app.modal = Some(Modal::Scale {
        workload_name: "web".into(),
        current_replicas: 1,
        input: String::new(),
    });
    app.handle_paste("1\n2".into());
    assert!(matches!(&app.modal, Some(Modal::Scale { input, .. }) if input == "12"));

    app.modal = Some(Modal::PortForward {
        pod_name: "web".into(),
        namespace: "default".into(),
        container_port: 80,
        local_port_input: String::new(),
    });
    app.handle_paste("90\r\n90".into());
    assert!(
        matches!(&app.modal, Some(Modal::PortForward { local_port_input, .. }) if local_port_input == "9090")
    );

    // Other modals ignore pastes.
    app.modal = Some(Modal::Confirm {
        title: "t".into(),
        message: "m".into(),
        action_name: "a".into(),
        is_destructive: false,
    });
    app.handle_paste("ignored".into());
    assert!(matches!(&app.modal, Some(Modal::Confirm { message, .. }) if message == "m"));
}

#[tokio::test]
async fn colon_commands_open_tree_actions_and_metrics_for_the_selected_row() {
    let (mut app, _rx) = common::app().await;

    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Nodes,
        vec![json!({ "name": "gpu-1", "status": "Ready" })],
    ));
    app.node_metrics_history.insert(
        "gpu-1".into(),
        VecDeque::from(vec![sample(1, 100, 512), sample(2, 200, 600)]),
    );
    app.execute_colon_command("metrics").await;
    match &app.modal {
        Some(Modal::MetricsTimeline(panel)) => {
            assert_eq!(panel.target_name, "gpu-1");
            assert_eq!(panel.target_kind, "Nodes");
            assert!(panel.namespace.is_none());
            assert_eq!(panel.samples.len(), 2);
        }
        _ => panic!("expected metrics timeline"),
    }
    app.modal = None;

    app.execute_colon_command("tree").await;
    match &app.active_view {
        ActiveView::Tree(t) => {
            assert_eq!(t.root_kind, "Nodes");
            assert_eq!(t.root_name, "gpu-1");
            assert!(t.is_loading);
        }
        _ => panic!("expected tree view"),
    }
    app.active_view = app.nav_stack.pop().unwrap();

    app.execute_colon_command("actions").await;
    assert!(
        matches!(&app.modal, Some(Modal::ActionPalette { resource_name, .. }) if resource_name == "gpu-1")
    );
    app.modal = None;

    app.active_view = ActiveView::Table(table_with(ResourceKind::Pods, vec![pod("web-0", "prod")]));
    app.pod_metrics_history
        .insert("web-0".into(), VecDeque::from(vec![sample(1, 5, 64)]));
    app.execute_colon_command("metric").await;
    match &app.modal {
        Some(Modal::MetricsTimeline(panel)) => {
            assert_eq!(panel.namespace.as_deref(), Some("prod"));
            assert_eq!(panel.samples.len(), 1);
        }
        _ => panic!("expected metrics timeline"),
    }
    app.modal = None;

    // Metrics only exist for pods and nodes; other tables get a hint.
    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Services,
        vec![json!({ "name": "svc" })],
    ));
    app.execute_colon_command("metrics").await;
    assert_eq!(
        toast(&app),
        "Select a Pod or Node to view its live metrics timeline"
    );

    // Without a selected row the tree and actions commands explain themselves.
    app.active_view = ActiveView::Table(table_with(ResourceKind::Pods, vec![]));
    app.execute_colon_command("lineage").await;
    assert_eq!(
        toast(&app),
        "Select a resource in table to view its relationship tree"
    );
    app.execute_colon_command("act").await;
    assert_eq!(
        toast(&app),
        "Select a resource in table to open its actions palette"
    );
}

#[tokio::test]
async fn colon_commands_open_the_reason_rail_clear_ai_and_fall_back_to_fuzzy_matches() {
    let (mut app, _rx) = common::app().await;

    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Events,
        vec![json!({ "name": "e1", "reason": "BackOff", "type": "Warning" })],
    ));
    app.execute_colon_command("reasons").await;
    match &app.modal {
        Some(Modal::ReasonRail { tallies, .. }) => assert_eq!(tallies[0].reason, "BackOff"),
        _ => panic!("expected reason rail"),
    }
    app.modal = None;

    app.active_view = ActiveView::Table(table_with(ResourceKind::Pods, vec![]));
    app.execute_colon_command("reason").await;
    assert!(toast(&app).starts_with(":reasons is only available"));

    app.assistant_state.add_assistant_message("x".into());
    app.execute_colon_command("clear-ai").await;
    assert_only_greeting(&app);
    assert_eq!(toast(&app), "✓ Conversation cleared");

    // "clear" only means the conversation while the assistant is showing.
    app.active_view = ActiveView::Assistant;
    app.assistant_state.add_assistant_message("y".into());
    assert_eq!(app.assistant_state.messages.len(), 2);
    app.execute_colon_command("clear").await;
    assert_only_greeting(&app);

    // An alias prefix scores high enough to run without an exact match.
    app.execute_colon_command("sv").await;
    assert!(matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Services));

    app.execute_colon_command("zzzz-nothing").await;
    assert_eq!(
        toast(&app),
        "Unknown command: 'zzzz-nothing' (type :help or ?)"
    );
}

#[tokio::test]
async fn view_targets_open_pickers_help_quit_and_deep_links() {
    let (mut app, _rx) = common::app().await;
    app.namespaces = vec!["default".into(), "kube-system".into()];
    app.active_namespace = "kube-system".into();

    app.execute_view_target(CommandTarget::Namespaces).await;
    match &app.modal {
        Some(Modal::NamespacePicker {
            selected_idx,
            current_namespace,
            ..
        }) => {
            assert_eq!(*selected_idx, 1);
            assert_eq!(current_namespace, "kube-system");
        }
        _ => panic!("expected namespace picker"),
    }
    app.modal = None;

    app.execute_view_target(CommandTarget::Contexts).await;
    assert!(matches!(app.modal, Some(Modal::ContextPicker { .. })));
    app.modal = None;

    app.execute_view_target(CommandTarget::Help).await;
    assert!(app.show_help);
    app.execute_view_target(CommandTarget::OpenUrl("x".into()))
        .await;
    app.execute_view_target(CommandTarget::Quit).await;
    assert!(!app.is_running);
    app.is_running = true;

    app.execute_command_target(CommandTarget::OpenUrl(String::new()))
        .await;
    assert_eq!(toast(&app), "Usage: :open <srelens://... or kind/name>");
    app.execute_command_target(CommandTarget::OpenUrl("srelens://nope/x".into()))
        .await;
    assert!(toast(&app).starts_with("Invalid URL: "), "{}", toast(&app));
    app.execute_command_target(CommandTarget::OpenUrl(
        "srelens://resource/test-cluster/default/widget/w1".into(),
    ))
    .await;
    assert_eq!(
        toast(&app),
        "Navigation error: Unknown resource kind 'widget'"
    );

    app.execute_command_target(CommandTarget::OpenUrl(
        "srelens://view/_/kube-system/nodes".into(),
    ))
    .await;
    assert!(matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Nodes));
}

#[tokio::test]
async fn deep_links_switch_namespace_select_rows_or_filter_for_missing_rows() {
    let (mut app, _rx) = common::app().await;
    app.resource_cache.insert(
        ("test-cluster".into(), "prod".into(), "pods".into()),
        vec![pod("web-0", "prod"), pod("web-1", "prod")],
    );

    let link = DeepLink::Resource {
        context: "test-cluster".into(),
        namespace: Some("prod".into()),
        kind: "pods".into(),
        name: "web-1".into(),
    };
    app.navigate_deep_link(&link).await.unwrap();
    assert_eq!(app.active_namespace, "prod");
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.kind, ResourceKind::Pods);
    assert_eq!(t.selected_idx, 1);
    assert_eq!(toast(&app), "Navigated to pods 'web-1'");

    let missing = DeepLink::Resource {
        context: String::new(),
        namespace: Some("prod".into()),
        kind: "pod".into(),
        name: "ghost".into(),
    };
    app.navigate_deep_link(&missing).await.unwrap();
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert!(
        t.filtered_indices.is_empty(),
        "filtered to the missing name"
    );

    let view = DeepLink::View {
        context: Some("test-cluster".into()),
        namespace: Some("default".into()),
        target: CommandTarget::Resource(ResourceKind::Deployments),
    };
    app.navigate_deep_link(&view).await.unwrap();
    assert_eq!(app.active_namespace, "default");
    assert!(
        matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Deployments)
    );

    let cluster = DeepLink::Cluster {
        context: "test-cluster".into(),
    };
    app.navigate_deep_link(&cluster).await.unwrap();
    assert_eq!(toast(&app), "Switched to cluster 'test-cluster'");

    let unknown = DeepLink::Resource {
        context: String::new(),
        namespace: None,
        kind: "widget".into(),
        name: "w".into(),
    };
    assert_eq!(
        app.navigate_deep_link(&unknown).await,
        Err("Unknown resource kind 'widget'".to_string())
    );
}

#[tokio::test]
async fn switching_to_a_crd_uses_cached_instances_and_discovered_columns() {
    let (mut app, _rx) = common::app().await;
    let mut discovered = CrdMeta {
        crd_name: "widgets.example.io".into(),
        group: "example.io".into(),
        version: "v1".into(),
        kind: "Widget".into(),
        plural: "widgets".into(),
        singular: "widget".into(),
        namespaced: true,
        short_names: vec!["wd".into()],
        printer_columns: vec![PrinterColumn {
            name: "Size".into(),
            json_path: ".spec.size".into(),
            col_type: "integer".into(),
            priority: 0,
            description: None,
        }],
    };
    app.crds.push(discovered.clone());
    app.resource_cache.insert(
        ("test-cluster".into(), "default".into(), "Widget".into()),
        vec![json!({ "name": "w1", "spec": { "size": 3 } })],
    );

    discovered.printer_columns.clear();
    app.switch_view_to_crd(discovered.clone()).await;
    match &app.active_view {
        ActiveView::Table(t) => {
            assert!(!t.is_loading);
            assert_eq!(t.raw_items.len(), 1);
            match &t.kind {
                ResourceKind::CustomResource(crd) => assert_eq!(crd.printer_columns.len(), 1),
                other => panic!("expected custom resource kind, got {other:?}"),
            }
        }
        _ => panic!("expected table"),
    }

    // Without a cache entry the table starts loading.
    app.resource_cache.clear();
    app.switch_view_to_crd(discovered).await;
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert!(t.is_loading);
    assert!(t.raw_items.is_empty());

    // The colon command resolves the short name to the same view.
    app.execute_colon_command("wd").await;
    assert!(
        matches!(&app.active_view, ActiveView::Table(t) if matches!(&t.kind, ResourceKind::CustomResource(c) if c.kind == "Widget"))
    );
}

// ---------------------------------------------------------------------------
// Containers, logs, shells
// ---------------------------------------------------------------------------

fn cached_pod_spec(name: &str) -> Value {
    json!({
        "name": name,
        "namespace": "default",
        "spec": {
            "containers": [{ "name": "app" }, { "name": "sidecar" }],
            "initContainers": [{ "name": "init-db" }],
            "ephemeralContainers": [{ "name": "debugger" }]
        }
    })
}

#[tokio::test]
async fn a_multi_container_pod_prompts_for_the_container_before_logs_or_shell() {
    let (mut app, _rx) = common::app().await;
    app.resource_cache.insert(
        ("test-cluster".into(), "default".into(), "Pods".into()),
        vec![cached_pod_spec("web-0")],
    );

    let containers = app.get_pod_containers("web-0", None).await;
    assert_eq!(containers, vec!["app", "sidecar", "init-db", "debugger"]);

    app.prompt_pod_logs("web-0".into(), None).await;
    match &app.modal {
        Some(Modal::ContainerPicker {
            containers,
            action,
            pod_name,
            ..
        }) => {
            assert_eq!(containers.len(), 4);
            assert_eq!(*action, ContainerAction::Logs);
            assert_eq!(pod_name, "web-0");
        }
        _ => panic!("expected container picker"),
    }
    app.modal = None;

    app.prompt_pod_shell("web-0".into(), Some("default".into()))
        .await;
    assert!(matches!(
        &app.modal,
        Some(Modal::ContainerPicker {
            action: ContainerAction::Shell,
            ..
        })
    ));
    assert!(app.requires_terminal_suspend.is_none());
}

#[tokio::test]
async fn a_single_or_unknown_container_pod_goes_straight_to_logs_or_shell() {
    let (mut app, _rx) = common::app().await;
    app.resource_cache.insert(
        ("test-cluster".into(), "default".into(), "Pods".into()),
        vec![json!({ "metadata": { "name": "solo" }, "spec": { "containers": [{ "name": "only" }] } })],
    );

    app.prompt_pod_shell("solo".into(), None).await;
    match &app.requires_terminal_suspend {
        Some(SuspendAction::PodShell { pod, container }) => {
            assert_eq!(pod, "solo");
            assert_eq!(container.as_deref(), Some("only"));
        }
        _ => panic!("expected pod shell suspend"),
    }
    app.requires_terminal_suspend = None;

    // A pod the cache does not know falls back to the (failing) API and opens
    // with no container.
    app.prompt_pod_shell("ghost".into(), None).await;
    assert!(matches!(
        &app.requires_terminal_suspend,
        Some(SuspendAction::PodShell {
            container: None,
            ..
        })
    ));

    app.prompt_pod_logs("solo".into(), None).await;
    match &app.active_view {
        ActiveView::Logs(l) => {
            assert_eq!(l.pod_name, "solo");
            assert_eq!(l.container.as_deref(), Some("only"));
            assert_eq!(
                l.lines[0],
                "Streaming logs for pod default/solo (container: only)..."
            );
        }
        _ => panic!("expected logs view"),
    }
    assert!(app
        .active_log_channel
        .as_deref()
        .unwrap()
        .starts_with("logs:solo:"));

    // Opening another stream replaces the active log channel.
    let previous = app.active_log_channel.clone();
    app.open_logs_view("other".into(), Some("kube-system".into()), None)
        .await;
    assert_ne!(app.active_log_channel, previous);
    let ActiveView::Logs(l) = &app.active_view else {
        panic!()
    };
    assert_eq!(l.namespace, "kube-system");
    assert_eq!(
        l.lines[0],
        "Streaming logs for pod kube-system/other (container: default)..."
    );
}

// ---------------------------------------------------------------------------
// Lineage and node inspector round trips
// ---------------------------------------------------------------------------

#[tokio::test]
async fn opening_a_resource_tree_without_a_cluster_reports_the_error_through_the_event_loop() {
    let (mut app, mut rx) = common::app().await;
    app.open_resource_tree("Deployment".into(), "web".into(), Some("default".into()));
    assert!(matches!(&app.active_view, ActiveView::Tree(t) if t.is_loading));

    let ev = wait_for(&mut rx, |e| matches!(e, AppEvent::LineageResult { .. })).await;
    let AppEvent::LineageResult { kind, name, result } = ev else {
        unreachable!()
    };
    assert_eq!((kind.as_str(), name.as_str()), ("Deployment", "web"));
    let err = result.expect_err("no cluster behind the test");
    assert!(err.starts_with("Failed to connect to cluster: "), "{err}");

    // A result for a different resource is ignored.
    app.handle_lineage_result("Pod", "other", Err("nope".into()));
    assert!(matches!(&app.active_view, ActiveView::Tree(t) if t.is_loading && t.error.is_none()));

    app.handle_lineage_result("deployment", "web", Err(err.clone()));
    let ActiveView::Tree(t) = &app.active_view else {
        panic!()
    };
    assert!(!t.is_loading);
    assert_eq!(t.error.as_deref(), Some(err.as_str()));

    app.handle_lineage_result("Deployment", "web", Ok(lineage_tree()));
    let ActiveView::Tree(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.nodes.len(), 3);
    assert!(t.error.is_none());
}

#[tokio::test]
async fn opening_the_node_inspector_seeds_history_and_reports_the_connection_error() {
    let (mut app, mut rx) = common::app().await;
    app.node_metrics_history.insert(
        "gpu-1".into(),
        VecDeque::from(vec![sample(1, 100, 512), sample(2, 300, 700)]),
    );
    app.open_node_inspector("gpu-1".into());
    match &app.active_view {
        ActiveView::NodeInspector(ni) => {
            assert_eq!(ni.cpu_history, vec![100, 300]);
            assert_eq!(ni.mem_history, vec![512, 700]);
            assert!(ni.is_loading);
        }
        _ => panic!("expected inspector"),
    }

    let ev = wait_for(&mut rx, |e| {
        matches!(e, AppEvent::NodeInspectorResult { .. })
    })
    .await;
    let AppEvent::NodeInspectorResult { node_name, result } = ev else {
        unreachable!()
    };
    assert_eq!(node_name, "gpu-1");
    let err = result.expect_err("no cluster behind the test");

    app.handle_node_inspector_result("other-node", Err("ignored".into()));
    assert!(matches!(&app.active_view, ActiveView::NodeInspector(ni) if ni.error.is_none()));
    app.handle_node_inspector_result("gpu-1", Err(err.clone()));
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert_eq!(ni.error.as_deref(), Some(err.as_str()));
    assert!(!ni.is_loading);

    app.handle_node_inspector_result("gpu-1", Ok(node_details("gpu-1", false)));
    let ActiveView::NodeInspector(ni) = &app.active_view else {
        panic!()
    };
    assert!(ni.error.is_none());
    assert_eq!(ni.details.as_ref().unwrap().pods.len(), 2);
}

// ---------------------------------------------------------------------------
// Action palette execution and confirmations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ai_palette_actions_prefill_the_assistant_prompt() {
    let (mut app, _rx) = common::app().await;
    let cases = [
        (
            QuickActionId::AskAi,
            "Investigate Pod 'web-0' in namespace 'default'",
        ),
        (QuickActionId::PlaybookCrashLoop, "/crashloop web-0"),
        (QuickActionId::PlaybookPending, "/pending web-0"),
        (QuickActionId::PlaybookOom, "/oom web-0"),
        (QuickActionId::PlaybookRollout, "/rollout web-0"),
        (QuickActionId::PlaybookEndpoints, "/endpoints web-0"),
        (QuickActionId::PlaybookNodePressure, "/nodepressure web-0"),
    ];
    for (id, expected) in cases {
        app.active_view = ActiveView::Table(ResourceTableState::new(ResourceKind::Pods));
        app.modal = Some(palette("Pod", "web-0", Some("default"), id));
        app.execute_action_palette().await;
        assert!(app.modal.is_none());
        assert!(matches!(app.active_view, ActiveView::Assistant), "{id:?}");
        assert!(matches!(app.nav_stack.last(), Some(ActiveView::Table(_))));
        assert!(
            app.assistant_state.input.starts_with(expected),
            "{id:?}: {}",
            app.assistant_state.input
        );
    }
}

#[tokio::test]
async fn navigation_palette_actions_open_the_matching_view_or_modal() {
    let (mut app, _rx) = common::app().await;

    app.modal = Some(palette(
        "Deployment",
        "web",
        Some("default"),
        QuickActionId::RelationshipTree,
    ));
    app.execute_action_palette().await;
    assert!(matches!(&app.active_view, ActiveView::Tree(t) if t.root_name == "web"));

    app.modal = Some(palette(
        "Pod",
        "web-0",
        Some("default"),
        QuickActionId::ViewLogs,
    ));
    app.execute_action_palette().await;
    assert!(matches!(&app.active_view, ActiveView::Logs(l) if l.pod_name == "web-0"));

    app.modal = Some(palette("Pod", "web-0", None, QuickActionId::OpenShell));
    app.execute_action_palette().await;
    assert!(
        matches!(&app.requires_terminal_suspend, Some(SuspendAction::PodShell { pod, .. }) if pod == "web-0")
    );

    app.modal = Some(palette("Pod", "web-0", None, QuickActionId::PortForward));
    app.execute_action_palette().await;
    match &app.modal {
        Some(Modal::PortForward {
            pod_name,
            namespace,
            container_port,
            local_port_input,
        }) => {
            assert_eq!(pod_name, "web-0");
            assert_eq!(namespace, "default");
            assert_eq!(*container_port, 8080);
            assert_eq!(local_port_input, "8080");
        }
        _ => panic!("expected port forward modal"),
    }

    app.modal = Some(palette(
        "Deployment",
        "web",
        Some("prod"),
        QuickActionId::RolloutRestart,
    ));
    app.execute_action_palette().await;
    match &app.modal {
        Some(Modal::Confirm {
            action_name,
            is_destructive,
            ..
        }) => {
            assert_eq!(action_name, "restart:Deployment:prod:web");
            assert!(!is_destructive);
        }
        _ => panic!("expected confirm modal"),
    }

    app.modal = Some(palette("Deployment", "web", None, QuickActionId::Scale));
    app.execute_action_palette().await;
    assert!(
        matches!(&app.modal, Some(Modal::Scale { workload_name, input, .. }) if workload_name == "web" && input == "1")
    );

    app.modal = Some(palette(
        "Deployment",
        "web",
        None,
        QuickActionId::JumpToPods,
    ));
    app.execute_action_palette().await;
    assert_eq!(app.filter_buffer, "web");
    assert!(matches!(&app.active_view, ActiveView::Table(t) if t.kind == ResourceKind::Pods));

    app.modal = Some(palette("Node", "gpu-1", None, QuickActionId::InspectNode));
    app.execute_action_palette().await;
    assert!(matches!(&app.active_view, ActiveView::NodeInspector(ni) if ni.node_name == "gpu-1"));

    app.modal = Some(palette("Node", "gpu-1", None, QuickActionId::CordonNode));
    app.execute_action_palette().await;
    assert_eq!(toast(&app), "Cordoned node gpu-1");
    app.modal = Some(palette("Node", "gpu-1", None, QuickActionId::DrainNode));
    app.execute_action_palette().await;
    assert_eq!(toast(&app), "Draining node gpu-1");

    app.modal = Some(palette("Service", "api", None, QuickActionId::Delete));
    app.execute_action_palette().await;
    match &app.modal {
        Some(Modal::Confirm {
            title,
            action_name,
            is_destructive,
            ..
        }) => {
            assert_eq!(title, "Delete Service");
            assert_eq!(action_name, "delete:Service:default:api");
            assert!(is_destructive);
        }
        _ => panic!("expected destructive confirm"),
    }
}

#[tokio::test]
async fn the_palette_filter_narrows_what_enter_runs() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(ResourceTableState::new(ResourceKind::Nodes));
    let make = |filter: &str, selected_idx: usize| Modal::ActionPalette {
        resource_kind: "Node".into(),
        resource_name: "gpu-1".into(),
        namespace: None,
        actions: vec![
            action(QuickActionId::CordonNode, "Cordon node"),
            action(QuickActionId::DrainNode, "Drain node"),
        ],
        selected_idx,
        filter: filter.into(),
    };

    app.modal = Some(make("drain", 0));
    app.execute_action_palette().await;
    assert_eq!(toast(&app), "Draining node gpu-1");

    app.toast = None;
    app.modal = Some(make("nothing-matches", 0));
    app.execute_action_palette().await;
    assert!(app.toast.is_none());
    assert!(app.modal.is_none());

    app.modal = Some(make("", 5));
    app.execute_action_palette().await;
    assert!(app.toast.is_none());

    // A non-palette modal is left alone.
    app.show_help = true;
    app.modal = None;
    app.execute_action_palette().await;
    assert!(app.show_help);
}

#[tokio::test]
async fn confirmed_actions_without_a_cluster_report_a_connection_error() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Pods,
        vec![pod("web-0", "default")],
    ));

    let confirm = |name: &str| Modal::Confirm {
        title: "t".into(),
        message: "m".into(),
        action_name: name.into(),
        is_destructive: true,
    };

    app.modal = Some(confirm("delete:Pod:default:web-0"));
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app).starts_with("Connection error: "),
        "{}",
        toast(&app)
    );
    let ActiveView::Table(t) = &app.active_view else {
        panic!()
    };
    assert_eq!(t.raw_items.len(), 1, "nothing was removed locally");

    app.modal = Some(confirm("delete:legacy"));
    app.handle_key_event(common::ch('Y')).await;
    assert_eq!(toast(&app), "Resource deleted successfully");

    app.modal = Some(confirm("restart:Deployment:default:web"));
    app.handle_key_event(common::ch('y')).await;
    assert!(
        toast(&app).starts_with("Connection error: "),
        "{}",
        toast(&app)
    );

    app.modal = Some(confirm("restart:legacy"));
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(toast(&app), "Rollout restart triggered");

    app.modal = Some(confirm("stop-pf:abc"));
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert_eq!(toast(&app), "Port forward stopped");

    // Unknown actions and cancellations do nothing.
    app.toast = None;
    app.modal = Some(confirm("noop:x"));
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert!(app.toast.is_none());
    app.modal = Some(confirm("delete:Pod:default:web-0"));
    app.handle_key_event(common::ch('n')).await;
    assert!(app.modal.is_none());
    assert!(app.toast.is_none());
}

#[tokio::test]
async fn scale_and_port_forward_modals_run_on_enter() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Table(table_with(
        ResourceKind::Deployments,
        vec![json!({ "name": "web", "namespace": "prod" })],
    ));

    app.modal = Some(Modal::Scale {
        workload_name: "web".into(),
        current_replicas: 1,
        input: String::new(),
    });
    app.handle_key_event(common::ch('3')).await;
    app.handle_key_event(common::ch('x')).await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app).starts_with("Connection error: "),
        "{}",
        toast(&app)
    );

    // Outside a table the scale command still resolves to a deployment.
    app.active_view = ActiveView::Assistant;
    app.toast = None;
    app.execute_scale_workload("web".into(), 2).await;
    assert!(
        toast(&app).starts_with("Connection error: "),
        "{}",
        toast(&app)
    );

    app.modal = Some(Modal::PortForward {
        pod_name: "web-0".into(),
        namespace: "default".into(),
        container_port: 8080,
        local_port_input: "90".into(),
    });
    app.handle_key_event(common::ch('9')).await;
    app.handle_key_event(common::ch('0')).await;
    app.handle_key_event(common::key(KeyCode::Backspace)).await;
    app.handle_key_event(common::key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert_eq!(
        toast(&app),
        "Port forward started on 127.0.0.1:909 -> web-0:8080"
    );
}

// ---------------------------------------------------------------------------
// Overview data plumbing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_overview_starts_from_cached_data_and_live_updates_replace_it() {
    let (mut app, _rx) = common::app().await;
    app.cluster_overview_data = Some(overview_data());
    app.cluster_version = "v1.31.0".into();
    app.is_connected = true;
    app.node_count = 9;
    app.pod_count = 99;

    app.switch_view_to_kind(ResourceKind::Overview).await;
    let ActiveView::Overview(ov) = &app.active_view else {
        panic!()
    };
    assert_eq!(
        ov.data.node_count, 3,
        "cached counts win over header counts"
    );
    assert_eq!(ov.data.total_pods, 40);
    assert_eq!(ov.data.k8s_version, "v1.31.0");
    assert!(ov.data.is_reachable);

    let mut live = overview_data();
    live.ready_nodes = 2;
    live.failed_pods = 5;
    app.handle_cluster_overview_update(&serde_json::to_string(&live).unwrap());
    let ActiveView::Overview(ov) = &app.active_view else {
        panic!()
    };
    assert_eq!(ov.data.ready_nodes, 2);
    assert_eq!(ov.data.failed_pods, 5);
    assert_eq!(app.cluster_overview_data.as_ref().unwrap().failed_pods, 5);

    let screen = wide(&mut app);
    assert!(screen.contains("2/3"), "ready ratio is drawn:\n{screen}");
}

#[tokio::test]
async fn the_overview_falls_back_to_header_counts_without_cached_data() {
    let (mut app, _rx) = common::app().await;
    app.cluster_overview_data = None;
    app.node_count = 4;
    app.pod_count = 12;
    app.switch_view_to_kind(ResourceKind::Overview).await;
    let ActiveView::Overview(ov) = &app.active_view else {
        panic!()
    };
    assert_eq!(ov.data.node_count, 4);
    assert_eq!(ov.data.total_pods, 12);
    assert_eq!(ov.data.context_name, "test-cluster");
    assert!(!ov.data.is_reachable);
}
