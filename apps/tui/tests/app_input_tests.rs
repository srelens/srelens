//! Integration tests for `App`'s input handling: construction, key handling,
//! command and filter modes, modals, navigation and the background-update
//! handlers that feed state back into the app. Nothing here talks to a
//! cluster; every cluster-bound path ends in the error/toast state the app
//! shows when the context cannot be resolved.

mod common;

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use srelens_kube::contexts::ContextDto;
use srelens_tui::app::{ActiveView, App, SuspendAction};
use srelens_tui::commands::{command_suggestions_with_crds, CrdMeta, PrinterColumn, ResourceKind};
use srelens_tui::event::AppEvent;
use srelens_tui::ui::{ContainerAction, InputMode, Modal};
use srelens_tui::views::metrics_panel_view::MetricsTimeRange;
use srelens_tui::views::overview_view::ClusterOverviewData;
use srelens_tui::views::resource_table::WorkloadSegment;
use srelens_tui::views::{
    DescribeViewState, LogsViewState, NodeInspectorState, ResourceTableState, YamlViewState,
};

use common::{ch, ctrl, key, shift, type_str};

// ---------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------

async fn press(app: &mut App, k: KeyEvent) {
    app.handle_key_event(k).await;
}

fn table(app: &App) -> &ResourceTableState {
    match &app.active_view {
        ActiveView::Table(t) => t,
        _ => panic!("expected a table view"),
    }
}

fn table_mut(app: &mut App) -> &mut ResourceTableState {
    match &mut app.active_view {
        ActiveView::Table(t) => t,
        _ => panic!("expected a table view"),
    }
}

fn set_table(app: &mut App, kind: ResourceKind, items: Vec<Value>) {
    let mut t = ResourceTableState::new(kind);
    t.set_items(items, "");
    app.active_view = ActiveView::Table(t);
}

/// Set the active table *and* prime the informer cache for its kind. Popping
/// back to a table re-runs `restart_active_watch`, which clears the rows
/// unless the cache can re-prime them, so any test that navigates away and
/// comes back needs the cache seeded too.
fn seed_table(app: &mut App, kind: ResourceKind, items: Vec<Value>) {
    if let Some(watch_kind) = kind.watch_kind() {
        app.resource_cache.insert(
            (
                app.active_context.clone(),
                app.active_namespace.clone(),
                watch_kind.to_string(),
            ),
            items.clone(),
        );
    }
    set_table(app, kind, items);
}

fn toast(app: &App) -> String {
    app.toast
        .as_ref()
        .map(|(m, _, _)| m.clone())
        .unwrap_or_default()
}

fn ago(secs: u64) -> Instant {
    Instant::now()
        .checked_sub(Duration::from_secs(secs))
        .expect("monotonic clock older than a few seconds")
}

fn ctx(name: &str, cluster: &str, namespace: &str) -> ContextDto {
    ContextDto {
        name: name.to_string(),
        stable_id: format!("file/{}", name),
        cluster: cluster.to_string(),
        server: format!("https://{}.example.invalid", cluster),
        namespace: namespace.to_string(),
        is_current: false,
        is_local: false,
        provider: None,
        // Deliberately free of the substrings the picker tests filter on:
        // the picker matches the source file too, so "kubeconfig" (which
        // contains "be") would match every context.
        source_file: "cluster-config.yaml".to_string(),
        auth_kind: "token".to_string(),
    }
}

fn pods(names: &[&str]) -> Vec<Value> {
    names
        .iter()
        .map(|n| json!({ "name": n, "namespace": "default", "status": "Running" }))
        .collect()
}

fn widget_crd() -> CrdMeta {
    CrdMeta {
        crd_name: "widgets.example.com".to_string(),
        group: "example.com".to_string(),
        version: "v1".to_string(),
        kind: "Widget".to_string(),
        plural: "widgets".to_string(),
        singular: "widget".to_string(),
        namespaced: true,
        short_names: vec!["wd".to_string()],
        printer_columns: vec![PrinterColumn {
            name: "Size".to_string(),
            json_path: ".spec.size".to_string(),
            col_type: "integer".to_string(),
            priority: 0,
            description: None,
        }],
    }
}

/// Wait (bounded) for the background task that emits `title` to report in.
async fn await_action_result(
    rx: &mut UnboundedReceiver<AppEvent>,
    title: &str,
) -> Result<String, String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await {
                Some(AppEvent::ActionResult { title: t, result }) if t == title => return result,
                Some(_) => continue,
                None => panic!("event channel closed"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no '{}' event within 10s", title))
}

// ---------------------------------------------------------------------------
// App::new
// ---------------------------------------------------------------------------

#[tokio::test]
async fn new_with_an_explicit_namespace_opens_the_pods_table_and_starts_its_watch() {
    let (app, _rx) = common::app().await;
    assert_eq!(app.active_context, "test-cluster");
    assert_eq!(app.active_namespace, "default");
    assert_eq!(app.last_active_namespace, "default");
    assert_eq!(table(&app).kind, ResourceKind::Pods);
    assert!(
        table(&app).is_loading,
        "no cache yet, so the table is loading"
    );
    assert!(app.nav_stack.is_empty());
    assert!(app.is_running);
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.cluster_version, "Connecting...");
    assert_eq!(
        app.cluster_name, "kubernetes",
        "unknown context falls back to a generic cluster name"
    );
    assert_eq!(
        app.current_watch_channel.as_deref(),
        Some("watch:test-cluster:default:pods")
    );
    assert!(app
        .active_watch_channels
        .contains("watch:test-cluster:default:pods"));
    assert_eq!(
        app.active_watch_pool,
        vec!["watch:test-cluster:default:pods".to_string()]
    );
}

#[tokio::test]
async fn new_without_a_namespace_opens_namespaces_and_all_namespaces_clears_the_namespace() {
    let (tx, _rx) = unbounded_channel();
    let app = App::new(None, None, true, None, vec![], tx)
        .await
        .expect("app");
    assert_eq!(
        app.active_context, "default",
        "no kubeconfig means the fallback context"
    );
    assert_eq!(app.active_namespace, "");
    assert_eq!(app.last_active_namespace, "default");
    assert_eq!(table(&app).kind, ResourceKind::Namespaces);
    assert_eq!(
        app.current_watch_channel.as_deref(),
        Some("watch:default::namespaces")
    );

    let (tx, _rx) = unbounded_channel();
    let app = App::new(
        Some("c".into()),
        None,
        false,
        Some(ResourceKind::Nodes),
        vec![],
        tx,
    )
    .await
    .expect("app");
    assert_eq!(app.active_namespace, "");
    assert_eq!(
        table(&app).kind,
        ResourceKind::Nodes,
        "an explicit initial resource wins"
    );
}

#[tokio::test]
async fn new_reads_contexts_from_kubeconfig_and_prefers_the_current_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config");
    std::fs::write(
        &path,
        r#"apiVersion: v1
kind: Config
current-context: staging
clusters:
- name: staging-cluster
  cluster: {server: "https://127.0.0.1:1"}
- name: prod-cluster
  cluster: {server: "https://127.0.0.1:2"}
contexts:
- name: staging
  context: {cluster: staging-cluster, user: u, namespace: team-a}
- name: prod
  context: {cluster: prod-cluster, user: u}
users:
- name: u
  user: {token: abc}
"#,
    )
    .expect("write kubeconfig");

    let (tx, _rx) = unbounded_channel();
    let app = App::new(None, None, false, None, vec![path.clone()], tx)
        .await
        .expect("app");
    assert_eq!(app.contexts.len(), 2);
    assert_eq!(app.active_context, "staging");
    assert_eq!(app.cluster_name, "staging-cluster");
    assert_eq!(app.server_url, "https://127.0.0.1:1");
    let staging = app
        .contexts
        .iter()
        .find(|c| c.name == "staging")
        .expect("staging");
    assert!(staging.is_current);
    assert_eq!(staging.namespace, "team-a");
    assert_eq!(staging.source_file, path.to_string_lossy());
    assert_eq!(staging.auth_kind, "token");
    assert_eq!(app.kubeconfig_paths, vec![path]);

    let (tx, _rx) = unbounded_channel();
    let app = App::new(
        Some("prod".into()),
        None,
        false,
        None,
        vec![dir.path().join("config")],
        tx,
    )
    .await
    .expect("app");
    assert_eq!(app.cluster_name, "prod-cluster");
    assert_eq!(app.server_url, "https://127.0.0.1:2");
}

#[tokio::test]
async fn startup_background_fetches_report_failures_that_the_handlers_apply() {
    let (mut app, mut rx) = common::app().await;

    let info = await_action_result(&mut rx, "cluster_info_failed").await;
    let err = info.expect_err("no cluster means the info fetch fails");
    assert!(!err.is_empty());
    app.handle_cluster_info_failure(&err);
    assert!(!app.is_connected);
    assert!(
        !app.cluster_unreachable,
        "still inside the connection grace period"
    );

    let overview = await_action_result(&mut rx, "cluster_overview_updated")
        .await
        .expect("overview payload");
    app.handle_cluster_overview_update(&overview);
    let data = app
        .cluster_overview_data
        .as_ref()
        .expect("overview data stored");
    assert_eq!(data.context_name, "test-cluster");
    assert_eq!(data.cluster_name, "kubernetes");
    assert!(!data.is_reachable);
    assert_eq!(data.node_count, 0);
    assert!(!app.is_connected);
}

#[tokio::test]
async fn any_keypress_dismisses_the_drag_to_copy_selection() {
    let (mut app, _rx) = common::app().await;
    app.screen_selection = Some(((0, 0), (5, 5)));
    app.screen_selecting = true;
    press(&mut app, key(KeyCode::Null)).await;
    assert!(app.screen_selection.is_none());
    assert!(!app.screen_selecting);
}

// ---------------------------------------------------------------------------
// Tick and metric updates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tick_expires_toasts_and_marks_the_cluster_unreachable_after_the_grace_period() {
    let (mut app, _rx) = common::app().await;
    app.toast = Some(("old".to_string(), ago(4), Style::default()));
    app.handle_tick();
    assert!(app.toast.is_none(), "toasts older than 3s are dropped");
    assert!(!app.cluster_unreachable);
    assert!(table(&app).is_loading);

    app.connection_attempt_start = ago(9);
    app.handle_tick();
    assert!(app.cluster_unreachable);
    assert!(!app.is_connected);
    assert!(
        !table(&app).is_loading,
        "an empty table stops spinning once the cluster is unreachable"
    );

    let screen = common::render_app(&mut app, 100, 30);
    assert!(screen.contains("test-cluster"), "screen: {}", screen);
}

#[tokio::test]
async fn tick_schedules_metric_refreshes_for_pod_and_node_views_and_modals() {
    let (mut app, _rx) = common::app().await;
    app.handle_tick();
    assert_eq!(
        app.pod_metrics_tick_counter, 1,
        "pods table wants pod metrics"
    );
    assert_eq!(app.node_metrics_tick_counter, 0);

    set_table(&mut app, ResourceKind::Nodes, vec![]);
    app.handle_tick();
    assert_eq!(app.pod_metrics_tick_counter, 1);
    assert_eq!(
        app.node_metrics_tick_counter, 1,
        "nodes table wants node metrics"
    );

    app.active_view = ActiveView::NodeInspector(NodeInspectorState::new("node-1".into()));
    app.handle_tick();
    assert_eq!(app.node_metrics_tick_counter, 2);

    app.active_view = ActiveView::Assistant;
    app.modal = Some(Modal::MetricsTimeline(
        srelens_tui::views::MetricsPanelState::new(
            "Pod".into(),
            "pod-a".into(),
            Some("default".into()),
            vec![],
        ),
    ));
    app.handle_tick();
    assert_eq!(
        app.pod_metrics_tick_counter, 2,
        "a pod timeline modal wants pod metrics"
    );

    app.modal = Some(Modal::MetricsTimeline(
        srelens_tui::views::MetricsPanelState::new("Node".into(), "node-1".into(), None, vec![]),
    ));
    app.handle_tick();
    assert_eq!(app.node_metrics_tick_counter, 3);

    app.modal = None;
    app.handle_tick();
    assert_eq!(
        app.pod_metrics_tick_counter, 2,
        "the assistant view needs neither"
    );
    assert_eq!(app.node_metrics_tick_counter, 3);
}

#[tokio::test]
async fn tick_schedules_helm_refreshes_and_keys_trigger_manual_refresh() {
    let (mut app, _rx) = common::app().await;

    let mut helm_state = srelens_tui::views::helm_view::HelmViewState::new();
    let release_1 = srelens_tui::views::helm_view::HelmReleaseItem {
        name: "nginx".into(),
        namespace: "default".into(),
        revision: 1,
        status: "deployed".into(),
        chart: "nginx-1.0.0".into(),
        chart_version: "1.0.0".into(),
        app_version: "1.25".into(),
        updated: "2026-01-01".into(),
    };
    helm_state.set_releases(vec![release_1.clone()]);
    app.active_view = ActiveView::Helm(helm_state);

    // 1. Tick increments helm_tick_counter and triggers fetch on tick 1
    app.handle_tick();
    assert_eq!(app.helm_tick_counter, 1);
    assert!(app.helm_refreshing, "helm_refreshing is set while fetch is in-flight");

    // Existing releases remain visible during background refresh (zero flicker)
    if let ActiveView::Helm(h) = &app.active_view {
        assert!(!h.is_loading, "is_loading should stay false when releases are already loaded");
        assert_eq!(h.releases.len(), 1);
    } else {
        panic!("expected Helm view");
    }

    // 2. handle_helm_releases_result finishes the refresh and preserves selection
    let summary_1 = srelens_kube::helm::HelmReleaseSummary {
        name: "nginx".into(),
        namespace: "default".into(),
        revision: 1,
        status: "deployed".into(),
        chart: "nginx-1.0.0".into(),
        chart_version: "1.0.0".into(),
        app_version: "1.25".into(),
        updated: "2026-01-01".into(),
    };
    let summary_2 = srelens_kube::helm::HelmReleaseSummary {
        name: "redis".into(),
        namespace: "default".into(),
        revision: 1,
        status: "deployed".into(),
        chart: "redis-1.0.0".into(),
        chart_version: "1.0.0".into(),
        app_version: "7.0".into(),
        updated: "2026-01-01".into(),
    };
    app.handle_helm_releases_result("test-cluster", "default", Ok(vec![summary_1, summary_2]));
    assert!(!app.helm_refreshing);
    if let ActiveView::Helm(h) = &app.active_view {
        assert_eq!(h.releases.len(), 2);
    } else {
        panic!("expected Helm view");
    }

    // 3. Advancing ticks: at tick 36 (35 ticks later), refresh fires again
    for _ in 0..34 {
        app.handle_tick();
    }
    assert_eq!(app.helm_tick_counter, 35);
    app.handle_tick();
    assert_eq!(app.helm_tick_counter, 36);
    assert!(app.helm_refreshing, "tick 36 triggers another refresh");

    // 4. Leaving Helm view resets the tick counter, while the in-flight guard is safely preserved until result handling
    app.active_view = ActiveView::Assistant;
    app.handle_tick();
    assert_eq!(app.helm_tick_counter, 0);
    assert!(app.helm_refreshing, "fetch initiated on tick 36 is still in-flight");

    app.handle_helm_releases_result("test-cluster", "default", Ok(vec![]));
    assert!(!app.helm_refreshing, "handling result clears in-flight guard");

    // 5. Manual refresh keys: 'R' (Shift+R) and Ctrl+r trigger immediate refresh with toast
    app.active_view = ActiveView::Helm(srelens_tui::views::helm_view::HelmViewState::new());
    app.handle_key_event(common::ch('R')).await;
    assert!(app.helm_refreshing);
    assert!(app.toast.as_ref().map(|(msg, _, _)| msg.contains("Refreshing Helm releases")).unwrap_or(false));

    // While refresh is in-flight, subsequent 'R' does not spawn duplicate or reset guard
    app.toast = None;
    app.handle_key_event(common::ch('R')).await;
    assert!(app.helm_refreshing);
    assert!(app.toast.as_ref().map(|(msg, _, _)| msg.contains("already in progress")).unwrap_or(false));

    app.handle_helm_releases_result("test-cluster", "default", Ok(vec![]));
    assert!(!app.helm_refreshing);
    app.toast = None;
    app.handle_key_event(common::ctrl('r')).await;
    assert!(app.helm_refreshing);
    assert!(app.toast.as_ref().map(|(msg, _, _)| msg.contains("Refreshing Helm releases")).unwrap_or(false));

    // Lowercase 'r' on empty releases shows warn toast or rollback
    app.toast = None;
    app.handle_key_event(common::ch('r')).await;
    assert!(app.modal.is_none());
}

#[tokio::test]
async fn helm_refresh_in_flight_survives_namespace_switch_and_refetches_new_target() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Helm(srelens_tui::views::helm_view::HelmViewState::new());
    app.active_context = "test-cluster".into();
    app.active_namespace = "default".into();

    // 1. Initial refresh starts for "default"
    app.refresh_helm_releases();
    assert!(app.helm_refreshing);

    // 2. User switches namespace to "kube-system" while refresh is in-flight
    app.switch_namespace("kube-system".into()).await;
    assert_eq!(app.active_namespace, "kube-system");
    // In-flight guard prevented duplicate concurrent fetch during switch
    assert!(app.helm_refreshing);

    // 3. Stale result for "default" arrives
    let summary = srelens_kube::helm::HelmReleaseSummary {
        name: "stale-nginx".into(),
        namespace: "default".into(),
        revision: 1,
        status: "deployed".into(),
        chart: "nginx-1.0.0".into(),
        chart_version: "1.0.0".into(),
        app_version: "1.25".into(),
        updated: "2026-01-01".into(),
    };
    app.handle_helm_releases_result("test-cluster", "default", Ok(vec![summary]));

    // Stale result is NOT applied to kube-system, and a fresh refresh is triggered for kube-system
    if let ActiveView::Helm(h) = &app.active_view {
        assert!(h.releases.is_empty(), "stale releases for old namespace should not be applied");
    } else {
        panic!("expected Helm view");
    }
    assert!(app.helm_refreshing, "new fetch for kube-system was immediately triggered");

    // 4. Fresh result for "kube-system" arrives
    let ks_summary = srelens_kube::helm::HelmReleaseSummary {
        name: "cilium".into(),
        namespace: "kube-system".into(),
        revision: 1,
        status: "deployed".into(),
        chart: "cilium-1.14.0".into(),
        chart_version: "1.14.0".into(),
        app_version: "1.14.0".into(),
        updated: "2026-01-01".into(),
    };
    app.handle_helm_releases_result("test-cluster", "kube-system", Ok(vec![ks_summary]));
    assert!(!app.helm_refreshing);
    if let ActiveView::Helm(h) = &app.active_view {
        assert_eq!(h.releases.len(), 1);
        assert_eq!(h.releases[0].name, "cilium");
    } else {
        panic!("expected Helm view");
    }
}

#[tokio::test]
async fn pod_metrics_update_fills_the_table_the_cache_and_an_open_timeline() {
    let (mut app, _rx) = common::app().await;
    set_table(&mut app, ResourceKind::Pods, pods(&["pod-a", "pod-b"]));
    app.resource_cache.insert(
        ("test-cluster".into(), "default".into(), "pods".into()),
        pods(&["pod-a"]),
    );
    app.modal = Some(Modal::MetricsTimeline(
        srelens_tui::views::MetricsPanelState::new(
            "Pod".into(),
            "pod-a".into(),
            Some("default".into()),
            vec![],
        ),
    ));

    app.handle_pod_metrics_update("not json");
    app.handle_pod_metrics_update("[]");
    assert!(
        app.pod_metrics_history.is_empty(),
        "bad and empty payloads are ignored"
    );

    let payload = json!([
        { "name": "pod-a", "namespace": "default", "cpuMillicores": 250, "memoryMiB": 2048 },
        { "name": "pod-b", "namespace": "default", "cpuMillicores": 5, "memoryMiB": 64 },
    ])
    .to_string();
    app.handle_pod_metrics_update(&payload);

    let t = table(&app);
    assert_eq!(t.raw_items[0]["cpu"], "250m");
    assert_eq!(t.raw_items[0]["memory"], "2.0Gi");
    assert_eq!(t.raw_items[1]["cpu"], "5m");
    assert_eq!(t.raw_items[1]["memory"], "64Mi");
    let cached = &app.resource_cache[&("test-cluster".into(), "default".into(), "pods".into())];
    assert_eq!(
        cached[0]["memory"], "2.0Gi",
        "the informer cache keeps the metrics too"
    );
    assert_eq!(app.pod_metrics_history["pod-a"].len(), 1);
    match &app.modal {
        Some(Modal::MetricsTimeline(panel)) => assert_eq!(panel.samples.len(), 1),
        _ => panic!("timeline modal should still be open"),
    }
}

#[tokio::test]
async fn pod_metrics_update_only_touches_pod_rows_of_the_workloads_table() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![
            json!({ "name": "web", "namespace": "default", "kind": "Deployment" }),
            json!({ "name": "web-1", "namespace": "default", "kind": "Pod" }),
        ],
    );
    let payload = json!([
        { "name": "web", "namespace": "default", "cpuMillicores": 1, "memoryMiB": 1 },
        { "name": "web-1", "namespace": "default", "cpuMillicores": 100, "memoryMiB": 1536 },
    ])
    .to_string();
    app.handle_pod_metrics_update(&payload);
    let t = table(&app);
    assert!(
        t.raw_items[0].get("cpu").is_none(),
        "controller rows carry no usage"
    );
    assert_eq!(t.raw_items[1]["cpu"], "100m");
    assert_eq!(t.raw_items[1]["memory"], "1.5Gi");
}

#[tokio::test]
async fn pod_metrics_history_is_capped_at_720_samples() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::Assistant;
    let payload =
        json!([{ "name": "p", "namespace": "default", "cpuMillicores": 1, "memoryMiB": 1 }])
            .to_string();
    for _ in 0..725 {
        app.handle_pod_metrics_update(&payload);
    }
    assert_eq!(app.pod_metrics_history["p"].len(), 720);
}

#[tokio::test]
async fn node_metrics_update_feeds_the_node_inspector_and_an_open_timeline() {
    let (mut app, _rx) = common::app().await;
    app.active_view = ActiveView::NodeInspector(NodeInspectorState::new("node-1".into()));

    app.handle_node_metrics_update("nope");
    app.handle_node_metrics_update("[]");
    assert!(app.node_metrics_history.is_empty());

    let payload = json!([
        { "name": "node-1", "cpuMillicores": 1500, "memoryMiB": 4096 },
        { "name": "node-2", "cpuMillicores": -5, "memoryMiB": -1 },
    ])
    .to_string();
    app.handle_node_metrics_update(&payload);
    assert_eq!(app.node_metrics_history["node-1"].len(), 1);
    assert_eq!(
        app.node_metrics_history["node-2"][0].cpu_millicores, 0,
        "negative usage clamps to zero"
    );
    match &app.active_view {
        ActiveView::NodeInspector(ni) => {
            assert_eq!(ni.cpu_history, vec![1500]);
            assert_eq!(ni.mem_history, vec![4096]);
        }
        _ => panic!("expected the node inspector"),
    }

    app.active_view = ActiveView::Assistant;
    app.modal = Some(Modal::MetricsTimeline(
        srelens_tui::views::MetricsPanelState::new("Node".into(), "node-1".into(), None, vec![]),
    ));
    app.handle_node_metrics_update(&payload);
    match &app.modal {
        Some(Modal::MetricsTimeline(panel)) => assert_eq!(panel.samples.len(), 2),
        _ => panic!("timeline modal should still be open"),
    }
}

// ---------------------------------------------------------------------------
// Cluster info, overview and CRD handlers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cluster_info_update_marks_the_cluster_connected_and_fills_the_overview() {
    let (mut app, _rx) = common::app().await;
    app.handle_cluster_info_update("garbage");
    assert!(!app.is_connected);
    app.handle_cluster_info_update("unknown|0|0");
    assert!(!app.is_connected, "an unknown version is not a connection");

    app.switch_view_to_kind(ResourceKind::Overview).await;
    app.handle_cluster_info_update("v1.30.2|3|42");
    assert!(app.is_connected);
    assert_eq!(app.cluster_version, "v1.30.2");
    assert_eq!((app.node_count, app.pod_count), (3, 42));
    match &app.active_view {
        ActiveView::Overview(ov) => {
            assert_eq!(ov.data.k8s_version, "v1.30.2");
            assert_eq!(ov.data.node_count, 3);
            assert_eq!(ov.data.total_pods, 42);
            assert!(ov.data.is_reachable);
            assert_eq!(ov.data.context_name, "test-cluster");
        }
        _ => panic!("expected the overview"),
    }
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("v1.30.2"), "screen: {}", screen);
}

#[tokio::test]
async fn cluster_info_failure_only_marks_unreachable_after_the_grace_period() {
    let (mut app, _rx) = common::app().await;
    app.is_connected = true;
    app.handle_cluster_info_failure("boom");
    assert!(!app.is_connected);
    assert!(!app.cluster_unreachable);
    assert!(table(&app).is_loading);

    app.connection_attempt_start = ago(9);
    app.handle_cluster_info_failure("boom");
    assert!(app.cluster_unreachable);
    assert!(!table(&app).is_loading);
}

#[tokio::test]
async fn cluster_overview_update_ignores_other_contexts_and_applies_its_own() {
    let (mut app, _rx) = common::app().await;
    let mut other = ClusterOverviewData::default();
    other.context_name = "somewhere-else".into();
    other.k8s_version = "v9".into();
    other.is_reachable = true;
    app.handle_cluster_overview_update(&serde_json::to_string(&other).unwrap());
    assert!(app.cluster_overview_data.is_none());
    assert_eq!(app.cluster_version, "Connecting...");

    app.switch_view_to_kind(ResourceKind::Overview).await;
    app.cluster_unreachable = true;
    let mut mine = ClusterOverviewData::default();
    mine.context_name = "test-cluster".into();
    mine.k8s_version = "v1.31.0".into();
    mine.is_reachable = true;
    mine.node_count = 2;
    mine.total_pods = 7;
    app.handle_cluster_overview_update(&serde_json::to_string(&mine).unwrap());
    assert_eq!(app.cluster_version, "v1.31.0");
    assert_eq!((app.node_count, app.pod_count), (2, 7));
    assert!(app.is_connected);
    assert!(!app.cluster_unreachable);
    match &app.active_view {
        ActiveView::Overview(ov) => assert_eq!(ov.data.total_pods, 7),
        _ => panic!("expected the overview"),
    }
    app.handle_cluster_overview_update("{ not json");
    assert_eq!(app.cluster_version, "v1.31.0", "bad payloads are ignored");
}

#[tokio::test]
async fn crd_updates_populate_the_registry_and_instances_fill_the_crd_table() {
    let (mut app, _rx) = common::app().await;
    app.handle_crds_update("[nonsense");
    assert!(app.crds.is_empty());
    app.handle_crds_update(&serde_json::to_string(&vec![widget_crd()]).unwrap());
    assert_eq!(app.crds.len(), 1);

    let mut bare = widget_crd();
    bare.printer_columns.clear();
    app.switch_view_to_crd(bare).await;
    assert_eq!(app.nav_stack.len(), 1);
    assert!(table(&app).is_loading);
    match &table(&app).kind {
        ResourceKind::CustomResource(crd) => {
            assert_eq!(crd.printer_columns.len(), 1, "columns come from discovery")
        }
        other => panic!("expected a CRD table, got {}", other),
    }

    app.handle_crd_instances_update("crd_instances:Widget", "oops");
    assert!(table(&app).raw_items.is_empty());
    app.handle_crd_instances_update(
        "crd_instances:Widget",
        &json!([{ "name": "w-1", "namespace": "default", "spec": { "size": 3 } }]).to_string(),
    );
    assert_eq!(table(&app).raw_items.len(), 1);
    assert!(!table(&app).is_loading);
    assert!(app.resource_cache.contains_key(&(
        "test-cluster".into(),
        "default".into(),
        "Widget".into()
    )));

    // A table whose CRD was built without columns picks them up from the registry on first data.
    if let ActiveView::Table(t) = &mut app.active_view {
        if let ResourceKind::CustomResource(crd) = &mut t.kind {
            crd.printer_columns.clear();
        }
    }
    app.handle_crd_instances_update(
        "crd_instances:widgets",
        &json!([{ "name": "w-2" }]).to_string(),
    );
    match &table(&app).kind {
        ResourceKind::CustomResource(crd) => assert_eq!(crd.printer_columns[0].name, "Size"),
        other => panic!("expected a CRD table, got {}", other),
    }
    assert_eq!(table(&app).raw_items.len(), 1);
    let screen = common::render_app(&mut app, 120, 30);
    assert!(screen.contains("w-2"), "screen: {}", screen);
}

// ---------------------------------------------------------------------------
// Context and namespace switching
// ---------------------------------------------------------------------------

#[tokio::test]
async fn switch_namespace_updates_state_toast_and_watch_channel() {
    let (mut app, _rx) = common::app().await;
    app.switch_namespace("kube-system".into()).await;
    assert_eq!(app.active_namespace, "kube-system");
    assert_eq!(app.last_active_namespace, "kube-system");
    assert_eq!(toast(&app), "Switched to namespace [kube-system]");
    assert_eq!(
        app.current_watch_channel.as_deref(),
        Some("watch:test-cluster:kube-system:pods")
    );
    let screen = common::render_app(&mut app, 120, 30);
    assert!(
        screen.contains("Switched to namespace [kube-system]"),
        "screen: {}",
        screen
    );

    app.switch_namespace(String::new()).await;
    assert_eq!(app.active_namespace, "");
    assert_eq!(
        app.last_active_namespace, "kube-system",
        "'all' does not overwrite the remembered namespace"
    );
    assert_eq!(toast(&app), "Switched to namespace [all]");
}

#[tokio::test]
async fn switch_context_swaps_assistant_state_and_resets_cluster_info() {
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![
        ctx("test-cluster", "c-test", ""),
        ctx("staging", "c-staging", "team-a"),
    ];
    app.assistant_state.input = "draft question".into();
    app.cluster_version = "v1.29.0".into();
    app.is_connected = true;
    app.resource_cache.insert(
        ("test-cluster".into(), "default".into(), "pods".into()),
        pods(&["p"]),
    );

    app.switch_context("test-cluster".into()).await;
    assert!(
        app.toast.is_none(),
        "switching to the current context is a no-op"
    );

    app.switch_context("staging".into()).await;
    assert_eq!(app.active_context, "staging");
    assert_eq!(app.cluster_name, "c-staging");
    assert_eq!(app.server_url, "https://c-staging.example.invalid");
    assert_eq!(
        app.active_namespace, "team-a",
        "the context's default namespace applies"
    );
    assert_eq!(
        app.assistant_state.input, "",
        "a fresh assistant for the new context"
    );
    assert_eq!(app.assistant_state.context_name, "staging");
    assert_eq!(app.assistant_states["test-cluster"].input, "draft question");
    assert_eq!(app.cluster_version, "Connecting...");
    assert!(!app.is_connected);
    assert!(app.resource_cache.is_empty());
    assert_eq!(toast(&app), "Switched to context 'staging'");
    assert_eq!(
        app.current_watch_channel.as_deref(),
        Some("watch:staging:team-a:pods")
    );

    app.switch_context("test-cluster".into()).await;
    assert_eq!(
        app.assistant_state.input, "draft question",
        "the saved assistant comes back"
    );
    assert!(!app.assistant_states.contains_key("test-cluster"));
    assert_eq!(
        app.active_namespace, "",
        "a context without a default namespace shows all"
    );
}

#[tokio::test]
async fn switch_context_while_on_the_overview_resets_the_panel() {
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("prod", "c-prod", "")];
    app.switch_view_to_kind(ResourceKind::Overview).await;
    app.switch_context("prod".into()).await;
    match &app.active_view {
        ActiveView::Overview(ov) => {
            assert_eq!(ov.data.context_name, "prod");
            assert_eq!(ov.data.cluster_name, "c-prod");
            assert_eq!(ov.data.k8s_version, "Connecting...");
            assert!(ov.data.is_reachable);
        }
        _ => panic!("expected the overview"),
    }
}

#[tokio::test]
async fn function_keys_switch_to_the_nth_context() {
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster", "a", ""), ctx("second", "b", "")];
    press(&mut app, key(KeyCode::F(2))).await;
    assert_eq!(app.active_context, "second");
    press(&mut app, key(KeyCode::F(9))).await;
    assert_eq!(
        app.active_context, "second",
        "a hotkey without a context does nothing"
    );
    press(&mut app, key(KeyCode::F(1))).await;
    assert_eq!(app.active_context, "test-cluster");
}

#[tokio::test]
async fn ctrl_a_toggles_between_all_namespaces_and_the_last_one() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ctrl('a')).await;
    assert_eq!(app.active_namespace, "");
    assert_eq!(app.last_active_namespace, "default");
    press(&mut app, ctrl('a')).await;
    assert_eq!(app.active_namespace, "default");

    app.active_namespace.clear();
    app.last_active_namespace.clear();
    press(&mut app, ctrl('a')).await;
    assert_eq!(
        app.active_namespace, "default",
        "nothing remembered falls back to default"
    );
}

#[tokio::test]
async fn ctrl_x_opens_the_context_picker_whose_keys_navigate_filter_and_select() {
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![
        ctx("alpha", "c-alpha", ""),
        ctx("beta", "c-beta", ""),
        ctx("gamma", "c-gamma", ""),
    ];

    press(&mut app, ctrl('x')).await;
    let sel = |app: &App| match &app.modal {
        Some(Modal::ContextPicker {
            selected_idx,
            filter,
            ..
        }) => (*selected_idx, filter.clone()),
        other => panic!("expected the context picker, got {:?}", other),
    };
    assert_eq!(sel(&app), (0, String::new()));
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(sel(&app).0, 1);
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(sel(&app).0, 0, "down wraps");
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(sel(&app).0, 2, "up wraps");
    press(&mut app, ctrl('j')).await;
    assert_eq!(sel(&app).0, 0);
    press(&mut app, ctrl('k')).await;
    assert_eq!(sel(&app).0, 2);
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("gamma"), "screen: {}", screen);

    type_str(&mut app, "bet").await;
    assert_eq!(sel(&app), (0, "bet".to_string()));
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(sel(&app).1, "be");
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(sel(&app).0, 0, "only one match, so it wraps to itself");
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert_eq!(app.active_context, "beta");

    press(&mut app, ctrl('x')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());

    press(&mut app, ctrl('x')).await;
    type_str(&mut app, "zzz").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(
        app.active_context, "beta",
        "enter with no match keeps the context"
    );

    press(&mut app, ch(':')).await;
    type_str(&mut app, "ctx").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(
        matches!(app.modal, Some(Modal::ContextPicker { .. })),
        ":ctx opens the same picker"
    );
}

#[tokio::test]
async fn namespace_picker_filters_navigates_and_switches() {
    let (mut app, _rx) = common::app().await;
    app.namespaces = vec!["default".into(), "kube-system".into(), "monitoring".into()];
    let sel = |app: &App| match &app.modal {
        Some(Modal::NamespacePicker {
            selected_idx,
            filter,
            ..
        }) => (*selected_idx, filter.clone()),
        other => panic!("expected the namespace picker, got {:?}", other),
    };

    press(&mut app, ch(':')).await;
    type_str(&mut app, "ns").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(sel(&app), (0, String::new()));
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(sel(&app).0, 1);
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(sel(&app).0, 0);
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(sel(&app).0, 2);
    press(&mut app, ctrl('j')).await;
    assert_eq!(sel(&app).0, 0);
    press(&mut app, ctrl('k')).await;
    assert_eq!(sel(&app).0, 2);
    press(&mut app, key(KeyCode::Home)).await;
    assert_eq!(sel(&app).0, 0, "Home moves to the start");
    press(&mut app, ctrl('g')).await;
    assert_eq!(sel(&app).0, 2, "Ctrl+g moves to the end");
    press(&mut app, key(KeyCode::Home)).await;
    assert_eq!(sel(&app).0, 0);
    press(&mut app, key(KeyCode::End)).await;
    assert_eq!(sel(&app).0, 2, "End moves to the end");
    press(&mut app, key(KeyCode::Null)).await;
    assert_eq!(sel(&app).0, 2, "unknown keys leave the picker alone");

    type_str(&mut app, "kube sys").await;
    assert_eq!(sel(&app), (0, "kube sys".to_string()));
    press(&mut app, ctrl('w')).await;
    assert_eq!(sel(&app).1, "kube ");
    press(&mut app, ctrl('u')).await;
    assert_eq!(sel(&app).1, "");
    type_str(&mut app, "monx").await;
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(sel(&app).1, "mon");
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("monitoring"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert_eq!(app.active_namespace, "monitoring");

    press(&mut app, ch(':')).await;
    type_str(&mut app, "namespaces").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(sel(&app).0, 2, "the picker starts on the active namespace");
    press(&mut app, ch('0')).await;
    assert!(app.modal.is_none());
    assert_eq!(app.active_namespace, "", "'0' means all namespaces");

    press(&mut app, ch(':')).await;
    type_str(&mut app, "ns").await;
    press(&mut app, key(KeyCode::Enter)).await;
    type_str(&mut app, "nothing-here").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.active_namespace, "", "no match keeps the namespace");

    press(&mut app, ch(':')).await;
    type_str(&mut app, "ns").await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());
}

// ---------------------------------------------------------------------------
// Command mode
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pressing_colon_enters_command_mode_and_esc_leaves_it() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ch(':')).await;
    assert_eq!(app.input_mode, InputMode::Command);
    type_str(&mut app, "pods").await;
    assert_eq!(app.command_buffer, "pods");
    let screen = common::render_app(&mut app, 120, 30);
    assert!(screen.contains(":pods"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.command_buffer, "");
    assert_eq!(app.command_suggestion_idx, 0);
}

#[tokio::test]
async fn command_mode_tab_and_arrow_keys_cycle_the_suggestions() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ch(':')).await;
    let suggestions = command_suggestions_with_crds("s", &app.crds);
    let len = suggestions.len();
    assert!(len > 1, "':s' should offer several commands");

    // Tab completes to suggestion and advances index
    app.command_buffer = "s".to_string();
    app.command_suggestion_idx = 0;
    press(&mut app, key(KeyCode::Tab)).await;
    assert_eq!(app.command_buffer, suggestions[0].0.name);
    assert_eq!(
        app.command_suggestion_idx, 1,
        "the cursor advances to the next candidate"
    );

    // Down arrow and Ctrl-N advance selection index in popup
    for k in [key(KeyCode::Down), ctrl('n')] {
        app.command_buffer = "s".to_string();
        app.command_suggestion_idx = 0;
        press(&mut app, k).await;
        assert_eq!(
            app.command_suggestion_idx, 1,
            "the cursor advances to the next candidate"
        );
    }
    // Up arrow and Ctrl-P step back selection index
    for k in [key(KeyCode::BackTab), key(KeyCode::Up), ctrl('p')] {
        app.command_buffer = "s".to_string();
        app.command_suggestion_idx = 0;
        press(&mut app, k).await;
        assert_eq!(
            app.command_suggestion_idx,
            len - 1,
            "stepping back from the first wraps to the last"
        );
    }

    app.command_buffer = "qqqqqq".into();
    app.command_suggestion_idx = 3;
    press(&mut app, key(KeyCode::Tab)).await;
    assert_eq!(
        app.command_buffer, "qqqqqq",
        "no suggestions, nothing changes"
    );
    assert_eq!(app.command_suggestion_idx, 3);
}

#[tokio::test]
async fn command_mode_editing_keys_delete_words_clear_and_backspace_out() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ch(':')).await;
    type_str(&mut app, "get pods").await;
    press(&mut app, ctrl('w')).await;
    assert_eq!(app.command_buffer, "get ");
    type_str(&mut app, "svc").await;
    press(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
    )
    .await;
    assert_eq!(app.command_buffer, "get ");
    press(
        &mut app,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
    )
    .await;
    assert_eq!(app.command_buffer, "");
    assert_eq!(
        app.input_mode,
        InputMode::Command,
        "deleting the last word keeps the prompt open"
    );
    press(&mut app, ctrl('w')).await;
    assert_eq!(
        app.input_mode,
        InputMode::Normal,
        "a word delete on an empty prompt closes it"
    );

    press(&mut app, ch(':')).await;
    type_str(&mut app, "deploy").await;
    press(&mut app, ctrl('u')).await;
    assert_eq!(app.command_buffer, "");
    assert_eq!(app.input_mode, InputMode::Command);
    type_str(&mut app, "po").await;
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(app.command_buffer, "p");
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(
        app.input_mode,
        InputMode::Normal,
        "backspacing the last character leaves command mode"
    );

    press(&mut app, ch(':')).await;
    type_str(&mut app, "po").await;
    press(
        &mut app,
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::ALT),
    )
    .await;
    assert_eq!(app.command_buffer, "po", "alt-chords are not typed");
    press(&mut app, ctrl('v')).await;
    assert!(
        app.command_buffer.starts_with("po"),
        "paste appends (or is a no-op headless)"
    );
    assert!(!app.command_buffer.contains('\n'));
    press(&mut app, key(KeyCode::Null)).await;
    assert_eq!(
        app.input_mode,
        InputMode::Command,
        "unknown keys are ignored"
    );
}

#[tokio::test]
async fn command_enter_runs_the_command_and_unknown_commands_toast() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ch(':')).await;
    type_str(&mut app, "zzzz").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.command_buffer, "");
    assert!(
        toast(&app).starts_with("Unknown command: 'zzzz'"),
        "toast: {}",
        toast(&app)
    );

    press(&mut app, ch(':')).await;
    type_str(&mut app, "help").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.show_help);
    press(&mut app, ch('?')).await;
    assert!(!app.show_help);

    press(&mut app, ch(':')).await;
    type_str(&mut app, "nodes").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).kind, ResourceKind::Nodes);
    assert_eq!(app.nav_stack.len(), 1);
    assert_eq!(
        app.current_watch_channel.as_deref(),
        Some("watch:test-cluster:default:nodes")
    );

    press(&mut app, ch(':')).await;
    type_str(&mut app, "deplo").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(
        table(&app).kind,
        ResourceKind::Deployments,
        "prefixes resolve"
    );

    press(&mut app, ch(':')).await;
    type_str(&mut app, "q").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(!app.is_running);
}

#[tokio::test]
async fn colon_commands_that_need_a_selection_warn_when_the_table_is_empty() {
    let (mut app, _rx) = common::app().await;
    for (cmd, expected) in [
        (
            "tree",
            "Select a resource in table to view its relationship tree",
        ),
        (
            "actions",
            "Select a resource in table to open its actions palette",
        ),
        (
            "metrics",
            "Select a Pod or Node to view its live metrics timeline",
        ),
        (
            "reasons",
            ":reasons is only available when viewing Events (:events)",
        ),
    ] {
        press(&mut app, ch(':')).await;
        type_str(&mut app, cmd).await;
        press(&mut app, key(KeyCode::Enter)).await;
        assert_eq!(toast(&app), expected, "for :{}", cmd);
        assert!(app.modal.is_none());
    }
}

#[tokio::test]
async fn colon_commands_open_tree_palette_metrics_and_reasons_for_the_selection() {
    let (mut app, _rx) = common::app().await;
    set_table(&mut app, ResourceKind::Pods, pods(&["pod-a"]));

    press(&mut app, ch(':')).await;
    type_str(&mut app, "act").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.modal {
        Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            ..
        }) => {
            assert_eq!(resource_kind, "Pods");
            assert_eq!(resource_name, "pod-a");
        }
        other => panic!("expected the action palette, got {:?}", other),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch(':')).await;
    type_str(&mut app, "metrics").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.modal {
        Some(Modal::MetricsTimeline(panel)) => {
            assert_eq!(panel.target_name, "pod-a");
            assert_eq!(panel.namespace.as_deref(), Some("default"));
        }
        other => panic!("expected the metrics timeline, got {:?}", other),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch(':')).await;
    type_str(&mut app, "tree").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.active_view {
        ActiveView::Tree(t) => {
            assert_eq!(t.root_kind, "Pods");
            assert_eq!(t.root_name, "pod-a");
            assert_eq!(t.namespace.as_deref(), Some("default"));
        }
        _ => panic!("expected the tree view"),
    }
    assert_eq!(app.nav_stack.len(), 1);

    set_table(
        &mut app,
        ResourceKind::Events,
        vec![
            json!({ "name": "e1", "reason": "BackOff", "type": "Warning", "object": "Pod/web-1" }),
        ],
    );
    press(&mut app, ch(':')).await;
    type_str(&mut app, "reasons").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.modal {
        Some(Modal::ReasonRail { tallies, .. }) => assert_eq!(tallies[0].reason, "BackOff"),
        other => panic!("expected the reason rail, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Filter mode
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slash_filter_narrows_the_table_and_esc_clears_it() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Pods,
        pods(&["api-1", "api-2", "db-1"]),
    );
    press(&mut app, ch('/')).await;
    assert_eq!(app.input_mode, InputMode::Filter);
    type_str(&mut app, "api").await;
    assert_eq!(table(&app).filtered_indices.len(), 2);
    let screen = common::render_app(&mut app, 120, 30);
    assert!(screen.contains("api"), "screen: {}", screen);
    assert!(
        !screen.contains("db-1"),
        "filtered rows disappear: {}",
        screen
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.filter_buffer, "api", "enter keeps the filter applied");
    assert_eq!(table(&app).filtered_indices.len(), 2);

    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(
        app.filter_buffer, "",
        "esc in normal mode clears an active filter first"
    );
    assert_eq!(table(&app).filtered_indices.len(), 3);

    press(&mut app, ch('/')).await;
    type_str(&mut app, "db").await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.filter_buffer, "");
    assert_eq!(table(&app).filtered_indices.len(), 3);
}

#[tokio::test]
async fn filter_editing_keys_delete_words_clear_backspace_and_paste() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Pods,
        pods(&["foo bar", "foo", "baz"]),
    );
    press(&mut app, ch('/')).await;
    type_str(&mut app, "foo bar").await;
    assert_eq!(table(&app).filtered_indices.len(), 1);
    press(&mut app, ctrl('w')).await;
    assert_eq!(app.filter_buffer, "foo ");
    assert_eq!(
        table(&app).filtered_indices.len(),
        2,
        "the filter is trimmed before matching"
    );
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(app.filter_buffer, "foo");
    press(&mut app, ctrl('u')).await;
    assert_eq!(app.filter_buffer, "");
    assert_eq!(table(&app).filtered_indices.len(), 3);
    press(&mut app, ctrl('v')).await;
    assert!(!app.filter_buffer.contains('\n'));
    press(
        &mut app,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
    )
    .await;
    press(&mut app, key(KeyCode::Null)).await;
    assert_eq!(
        app.input_mode,
        InputMode::Filter,
        "chords and unknown keys are ignored"
    );
}

#[tokio::test]
async fn slash_in_text_views_seeds_the_filter_with_the_current_search() {
    let (mut app, _rx) = common::app().await;

    let mut yaml = YamlViewState::new(
        "p".into(),
        "Pod".into(),
        None,
        "kind: Pod\nname: x\n".into(),
    );
    yaml.set_search_query("kind");
    app.active_view = ActiveView::Yaml(yaml);
    press(&mut app, ch('/')).await;
    assert_eq!(app.filter_buffer, "kind");
    type_str(&mut app, "s").await;
    match &app.active_view {
        ActiveView::Yaml(y) => assert_eq!(y.search_query, "kinds"),
        _ => panic!("expected yaml"),
    }
    press(&mut app, key(KeyCode::Esc)).await;
    match &app.active_view {
        ActiveView::Yaml(y) => assert_eq!(y.search_query, ""),
        _ => panic!("expected yaml"),
    }

    let mut desc = DescribeViewState::new(
        "p".into(),
        "Pod".into(),
        None,
        "Name: p\nStatus: Running\n".into(),
    );
    desc.set_search_query("status");
    app.active_view = ActiveView::Describe(desc);
    press(&mut app, ch('/')).await;
    assert_eq!(app.filter_buffer, "status");
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.input_mode, InputMode::Normal);
    match &app.active_view {
        ActiveView::Describe(d) => assert_eq!(d.search_matches.len(), 1),
        _ => panic!("expected describe"),
    }

    let mut logs = LogsViewState::new("p".into(), "default".into(), None, "logs:x".into());
    logs.push_line("error one".into());
    logs.set_search_query("error");
    app.active_view = ActiveView::Logs(logs);
    press(&mut app, ch('/')).await;
    assert_eq!(app.filter_buffer, "error");
    press(&mut app, ctrl('u')).await;
    match &app.active_view {
        ActiveView::Logs(l) => assert!(l.search_matches.is_empty()),
        _ => panic!("expected logs"),
    }
}

#[tokio::test]
async fn n_and_shift_n_step_through_search_matches_in_text_views() {
    let (mut app, _rx) = common::app().await;
    let text = "x1\ny\nx2\nx3\n";
    let idx = |app: &App| match &app.active_view {
        ActiveView::Yaml(v) => v.current_match_idx,
        ActiveView::Describe(v) => v.current_match_idx,
        ActiveView::Logs(v) => v.current_match_idx,
        _ => panic!("expected a text view"),
    };

    let mut yaml = YamlViewState::new("p".into(), "Pod".into(), None, text.into());
    yaml.set_search_query("x");
    app.active_view = ActiveView::Yaml(yaml);
    assert_eq!(idx(&app), Some(0));
    press(&mut app, ch('n')).await;
    assert_eq!(idx(&app), Some(1));
    press(&mut app, ch('N')).await;
    assert_eq!(idx(&app), Some(0));
    press(&mut app, ch('N')).await;
    assert_eq!(idx(&app), Some(2), "previous wraps to the last match");

    let mut desc = DescribeViewState::new("p".into(), "Pod".into(), None, text.into());
    desc.set_search_query("x");
    app.active_view = ActiveView::Describe(desc);
    press(&mut app, ch('n')).await;
    assert_eq!(idx(&app), Some(1));
    press(&mut app, ch('N')).await;
    assert_eq!(idx(&app), Some(0));

    let mut logs = LogsViewState::new("p".into(), "default".into(), None, "logs:x".into());
    for l in text.lines() {
        logs.push_line(l.into());
    }
    logs.set_search_query("x");
    app.active_view = ActiveView::Logs(logs);
    press(&mut app, ch('n')).await;
    assert_eq!(idx(&app), Some(1));
    press(&mut app, ch('N')).await;
    assert_eq!(idx(&app), Some(0));
}

#[tokio::test]
async fn esc_in_text_views_clears_selection_and_search_before_popping_the_view() {
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Nodes).await;
    let base_depth = app.nav_stack.len();

    let mut yaml = YamlViewState::new("p".into(), "Pod".into(), None, "a\nb\nc\n".into());
    yaml.start_selection(0);
    yaml.update_selection(1);
    yaml.set_search_query("b");
    let prev = std::mem::replace(&mut app.active_view, ActiveView::Yaml(yaml));
    app.nav_stack.push(prev);
    app.filter_buffer = "b".into();

    press(&mut app, key(KeyCode::Esc)).await;
    match &app.active_view {
        ActiveView::Yaml(y) => {
            assert!(y.selection.is_none(), "first esc drops the selection");
            assert_eq!(y.search_query, "b");
        }
        _ => panic!("expected yaml"),
    }
    press(&mut app, key(KeyCode::Esc)).await;
    match &app.active_view {
        ActiveView::Yaml(y) => assert_eq!(y.search_query, "", "second esc drops the search"),
        _ => panic!("expected yaml"),
    }
    assert_eq!(app.filter_buffer, "");
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(
        matches!(app.active_view, ActiveView::Table(_)),
        "third esc pops the view"
    );
    assert_eq!(app.nav_stack.len(), base_depth);

    let mut desc = DescribeViewState::new("p".into(), "Pod".into(), None, "Name: p\n".into());
    desc.set_search_query("name");
    let prev = std::mem::replace(&mut app.active_view, ActiveView::Describe(desc));
    app.nav_stack.push(prev);
    press(&mut app, key(KeyCode::Esc)).await;
    match &app.active_view {
        ActiveView::Describe(d) => assert_eq!(d.search_query, ""),
        _ => panic!("expected describe"),
    }
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));

    let mut logs = LogsViewState::new("p".into(), "default".into(), None, "logs:x".into());
    logs.push_line("hello".into());
    logs.set_search_query("hello");
    let prev = std::mem::replace(&mut app.active_view, ActiveView::Logs(logs));
    app.nav_stack.push(prev);
    app.active_log_channel = Some("logs:x".into());
    press(&mut app, key(KeyCode::Esc)).await;
    match &app.active_view {
        ActiveView::Logs(l) => assert_eq!(l.search_query, ""),
        _ => panic!("expected logs"),
    }
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));
    assert!(
        app.active_log_channel.is_none(),
        "leaving the logs view stops the stream"
    );
}

// ---------------------------------------------------------------------------
// Help, quit, toasts, assistant drawer, settings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn question_mark_opens_help_and_esc_q_or_question_mark_close_it() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ch('?')).await;
    assert!(app.show_help);
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("Cheat Sheet"), "screen: {}", screen);
    press(&mut app, ch('j')).await;
    assert!(app.show_help, "other keys are swallowed while help is open");
    press(&mut app, ch('q')).await;
    assert!(!app.show_help);
    press(&mut app, ch('?')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(!app.show_help);
    press(&mut app, ch('?')).await;
    press(&mut app, ch('?')).await;
    assert!(!app.show_help);
}

#[tokio::test]
async fn ctrl_c_stops_the_app_even_inside_a_modal_or_help() {
    let (mut app, _rx) = common::app().await;
    app.show_help = true;
    app.modal = Some(Modal::Confirm {
        title: "t".into(),
        message: "m".into(),
        action_name: "noop".into(),
        is_destructive: false,
    });
    press(&mut app, ctrl('c')).await;
    assert!(!app.is_running);
    assert!(app.show_help, "nothing else is touched");
}

#[tokio::test]
async fn a_toast_older_than_four_seconds_is_dropped_on_the_next_keypress() {
    let (mut app, _rx) = common::app().await;
    app.toast = Some(("stale".into(), ago(5), Style::default()));
    press(&mut app, ch('?')).await;
    assert!(app.toast.is_none());
    app.toast = Some(("fresh".into(), Instant::now(), Style::default()));
    press(&mut app, ch('?')).await;
    assert_eq!(toast(&app), "fresh");
}

#[tokio::test]
async fn tab_toggles_the_assistant_drawer_and_its_esc_clears_suggestions_and_selection() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, key(KeyCode::Tab)).await;
    assert!(matches!(app.active_view, ActiveView::Assistant));
    assert_eq!(app.nav_stack.len(), 1);
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("AI Assistant"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Tab)).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));
    assert!(app.nav_stack.is_empty());

    press(&mut app, key(KeyCode::Tab)).await;
    app.assistant_state.input = "/cr".into();
    app.assistant_state.update_slash_suggestions();
    assert!(!app.assistant_state.slash_suggestions.is_empty());
    press(&mut app, ch(':')).await;
    assert_eq!(
        app.input_mode,
        InputMode::Normal,
        "':' with a draft goes to the composer, not the prompt"
    );
    assert_eq!(
        app.assistant_state.input, "/cr:",
        "the character lands in the composer"
    );
    press(&mut app, ch('?')).await;
    assert!(
        !app.show_help,
        "'?' with a draft goes to the composer, not help"
    );
    assert_eq!(app.assistant_state.input, "/cr:?");

    app.assistant_state.input = "/cr".into();
    app.assistant_state.update_slash_suggestions();
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(
        app.assistant_state.slash_suggestions.is_empty(),
        "esc first drops the suggestions"
    );
    assert!(matches!(app.active_view, ActiveView::Assistant));

    app.assistant_state.update_slash_suggestions();
    press(&mut app, key(KeyCode::Tab)).await;
    assert!(app.assistant_state.input.starts_with('/'));
    assert!(
        app.assistant_state.input.ends_with(' '),
        "tab completes the slash command"
    );
    assert!(matches!(app.active_view, ActiveView::Assistant));

    app.assistant_state.start_selection(0, 0);
    app.assistant_state.update_selection(0, 4);
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(
        app.assistant_state.selection.is_none(),
        "esc drops the selection"
    );
    assert!(matches!(app.active_view, ActiveView::Assistant));

    app.assistant_state.input.clear();
    press(&mut app, ch('/')).await;
    assert_eq!(
        app.input_mode,
        InputMode::Normal,
        "'/' in the assistant is a slash command, not a filter"
    );
    assert_eq!(app.assistant_state.input, "/");
    app.assistant_state.input.clear();
    app.assistant_state.update_slash_suggestions();
    press(&mut app, ch(':')).await;
    assert_eq!(
        app.input_mode,
        InputMode::Command,
        "an empty composer lets ':' open the prompt"
    );
}

#[tokio::test]
async fn tab_on_workloads_cycles_segments_and_on_wide_events_focuses_the_reason_rail() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![
            json!({ "name": "web", "namespace": "default", "kind": "Deployment" }),
            json!({ "name": "web-1", "namespace": "default", "kind": "Pod" }),
        ],
    );
    press(&mut app, key(KeyCode::Tab)).await;
    assert_eq!(table(&app).workload_segment, WorkloadSegment::Deployment);
    assert_eq!(table(&app).filtered_indices.len(), 1);
    assert_eq!(toast(&app), "Segment: Deployment");
    assert!(matches!(app.active_view, ActiveView::Table(_)));

    set_table(
        &mut app,
        ResourceKind::Events,
        vec![json!({ "name": "e", "reason": "Pulled", "object": "Pod/p" })],
    );
    table_mut(&mut app).last_area_width.set(120);
    press(&mut app, key(KeyCode::Tab)).await;
    assert!(table(&app).reason_rail_focused);
    press(&mut app, key(KeyCode::Tab)).await;
    assert!(
        !table(&app).reason_rail_focused,
        "tab in the rail hands focus back"
    );

    table_mut(&mut app).last_area_width.set(80);
    press(&mut app, key(KeyCode::Tab)).await;
    assert!(
        matches!(app.active_view, ActiveView::Assistant),
        "a narrow events table opens the assistant"
    );
}

#[tokio::test]
async fn settings_view_lets_colon_open_the_prompt_unless_a_field_is_being_edited() {
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Settings).await;
    press(&mut app, ch(':')).await;
    assert_eq!(app.input_mode, InputMode::Command);
    press(&mut app, key(KeyCode::Esc)).await;

    if let ActiveView::Settings(s) = &mut app.active_view {
        s.is_editing = true;
    }
    press(&mut app, ch(':')).await;
    assert_eq!(
        app.input_mode,
        InputMode::Normal,
        "while editing, ':' is text"
    );
}

// ---------------------------------------------------------------------------
// Table keys that open modals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ctrl_d_asks_to_confirm_deletion_and_n_cancels_while_y_runs_it() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ctrl('d')).await;
    assert!(app.modal.is_none(), "nothing selected, nothing to delete");

    set_table(&mut app, ResourceKind::Pods, pods(&["pod-a"]));
    press(&mut app, ctrl('d')).await;
    match &app.modal {
        Some(Modal::Confirm {
            title,
            action_name,
            is_destructive,
            ..
        }) => {
            assert_eq!(title, "Delete Pod [pod-a]");
            assert_eq!(action_name, "delete:Pod:default:pod-a");
            assert!(is_destructive);
        }
        other => panic!("expected a confirm dialog, got {:?}", other),
    }
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("Delete Pod [pod-a]"), "screen: {}", screen);
    press(&mut app, ch('x')).await;
    assert!(app.modal.is_some(), "unrelated keys keep the dialog");
    press(&mut app, ch('n')).await;
    assert!(app.modal.is_none());
    assert!(app.toast.is_none());

    press(&mut app, ctrl('d')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());

    press(&mut app, ctrl('d')).await;
    press(&mut app, ch('y')).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app).starts_with("Connection error"),
        "toast: {}",
        toast(&app)
    );

    app.active_namespace.clear();
    set_table(
        &mut app,
        ResourceKind::Nodes,
        vec![json!({ "name": "node-1" })],
    );
    press(&mut app, ctrl('d')).await;
    match &app.modal {
        Some(Modal::Confirm { action_name, .. }) => {
            assert_eq!(action_name, "delete:Node:default:node-1")
        }
        other => panic!("expected a confirm dialog, got {:?}", other),
    }
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(toast(&app).starts_with("Connection error"));
}

#[tokio::test]
async fn r_on_a_workload_asks_to_confirm_a_rollout_restart() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Deployments,
        vec![json!({ "name": "web", "namespace": "shop" })],
    );
    press(&mut app, ch('r')).await;
    match &app.modal {
        Some(Modal::Confirm {
            title,
            action_name,
            is_destructive,
            ..
        }) => {
            assert_eq!(title, "Restart Workload [web]");
            assert_eq!(action_name, "restart:Deployment:shop:web");
            assert!(!is_destructive);
        }
        other => panic!("expected a confirm dialog, got {:?}", other),
    }
    press(&mut app, ch('Y')).await;
    assert!(
        toast(&app).starts_with("Connection error"),
        "toast: {}",
        toast(&app)
    );

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web-1", "namespace": "shop", "kind": "Pod" })],
    );
    press(&mut app, ch('r')).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app).starts_with("Rollout restart is only available"),
        "toast: {}",
        toast(&app)
    );

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "agent", "namespace": "shop", "kind": "DaemonSet" })],
    );
    press(&mut app, ch('r')).await;
    match &app.modal {
        Some(Modal::Confirm { action_name, .. }) => {
            assert_eq!(action_name, "restart:DaemonSet:shop:agent")
        }
        other => panic!("expected a confirm dialog, got {:?}", other),
    }
}

#[tokio::test]
async fn r_on_an_empty_or_unreachable_table_retries_the_connection() {
    let (mut app, _rx) = common::app().await;
    app.cluster_unreachable = true;
    app.connection_attempt_start = ago(30);
    table_mut(&mut app).is_loading = false;
    press(&mut app, ch('r')).await;
    assert!(app.modal.is_none());
    assert!(!app.cluster_unreachable);
    assert!(table(&app).is_loading);
    assert!(app.connection_attempt_start.elapsed() < Duration::from_secs(5));
    assert_eq!(toast(&app), "Retrying cluster connection...");
}

#[tokio::test]
async fn ctrl_s_opens_the_scale_dialog_which_digits_backspace_and_enter_drive() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Deployments,
        vec![json!({ "name": "web", "namespace": "default" })],
    );
    let input = |app: &App| match &app.modal {
        Some(Modal::Scale {
            input,
            workload_name,
            ..
        }) => {
            assert_eq!(workload_name, "web");
            input.clone()
        }
        other => panic!("expected the scale dialog, got {:?}", other),
    };
    press(&mut app, ctrl('s')).await;
    assert_eq!(input(&app), "1");
    press(&mut app, ch('5')).await;
    assert_eq!(input(&app), "15");
    press(&mut app, ch('x')).await;
    assert_eq!(input(&app), "15", "letters are not replicas");
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("15"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Backspace)).await;
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(input(&app), "");
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert!(app.toast.is_none(), "an empty count is silently dropped");

    press(&mut app, ctrl('s')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());

    press(&mut app, ctrl('s')).await;
    press(&mut app, ch('3')).await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(
        toast(&app).starts_with("Connection error"),
        "toast: {}",
        toast(&app)
    );

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web-1", "namespace": "default", "kind": "Pod" })],
    );
    press(&mut app, ctrl('s')).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app).starts_with("Scale is only available"),
        "toast: {}",
        toast(&app)
    );
}

#[tokio::test]
async fn f_opens_port_forward_with_the_detected_port_and_enter_starts_it() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Services,
        vec![
            json!({ "name": "web-svc", "namespace": "shop", "spec": { "ports": [{ "port": 443 }] } }),
        ],
    );
    let input = |app: &App| match &app.modal {
        Some(Modal::PortForward {
            local_port_input,
            container_port,
            ..
        }) => (*container_port, local_port_input.clone()),
        other => panic!("expected the port-forward dialog, got {:?}", other),
    };
    let target = |app: &App| match &app.modal {
        Some(Modal::PortForward {
            pod_name,
            namespace,
            ..
        }) => (pod_name.clone(), namespace.clone()),
        other => panic!("expected the port-forward dialog, got {:?}", other),
    };
    press(&mut app, ch('F')).await;
    assert_eq!(target(&app), ("web-svc".to_string(), "shop".to_string()));
    assert_eq!(input(&app), (443, "443".to_string()));
    press(&mut app, ch('1')).await;
    press(&mut app, ch('a')).await;
    assert_eq!(input(&app).1, "4431");
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(input(&app).1, "443");
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("443"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app) == "Port forward started on 127.0.0.1:443 -> web-svc:443"
            || toast(&app).starts_with("Failed to port forward: Permission denied"),
        "toast: {}",
        toast(&app)
    );

    press(&mut app, ch('f')).await;
    press(&mut app, key(KeyCode::Backspace)).await;
    press(&mut app, key(KeyCode::Backspace)).await;
    press(&mut app, key(KeyCode::Backspace)).await;
    app.toast = None;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.toast.is_none(), "an unparsable port does nothing");

    press(&mut app, ch('f')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());

    set_table(&mut app, ResourceKind::Pods, pods(&["plain"]));
    press(&mut app, ch('f')).await;
    assert_eq!(input(&app).0, 8080, "no declared port falls back to 8080");
    press(&mut app, key(KeyCode::Esc)).await;

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web", "namespace": "default", "kind": "Deployment" })],
    );
    press(&mut app, ch('f')).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app).starts_with("Port forward is only available for Pods"),
        "toast: {}",
        toast(&app)
    );
}

#[tokio::test]
async fn container_picker_cycles_and_enter_opens_logs_or_a_shell() {
    let (mut app, _rx) = common::app().await;
    let multi = vec![json!({
        "name": "multi",
        "namespace": "default",
        "spec": { "containers": [{ "name": "app" }, { "name": "sidecar" }], "initContainers": [{ "name": "init" }] }
    })];
    // `get_pod_containers` looks the pod up under a "Pods" cache key; the
    // informer cache the table is re-primed from uses the watch kind "pods".
    app.resource_cache.insert(
        ("test-cluster".into(), "default".into(), "Pods".into()),
        multi.clone(),
    );
    seed_table(&mut app, ResourceKind::Pods, multi);

    let picker = |app: &App| match &app.modal {
        Some(Modal::ContainerPicker {
            containers,
            selected_idx,
            action,
            ..
        }) => {
            assert_eq!(containers, &["app", "sidecar", "init"]);
            (*selected_idx, *action)
        }
        other => panic!("expected the container picker, got {:?}", other),
    };
    press(&mut app, ch('l')).await;
    assert_eq!(picker(&app), (0, ContainerAction::Logs));
    press(&mut app, ch('j')).await;
    assert_eq!(picker(&app).0, 1);
    press(&mut app, key(KeyCode::Down)).await;
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(picker(&app).0, 0, "down wraps");
    press(&mut app, ch('k')).await;
    assert_eq!(picker(&app).0, 2, "up wraps");
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(picker(&app).0, 1);
    press(&mut app, ch('z')).await;
    assert_eq!(picker(&app).0, 1);
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("sidecar"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    match &app.active_view {
        ActiveView::Logs(l) => {
            assert_eq!(l.pod_name, "multi");
            assert_eq!(l.container.as_deref(), Some("sidecar"));
            assert_eq!(l.namespace, "default");
        }
        _ => panic!("expected the logs view"),
    }
    assert!(app
        .active_log_channel
        .as_deref()
        .unwrap()
        .starts_with("logs:multi:"));
    assert_eq!(app.nav_stack.len(), 1);
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));
    assert!(app.active_log_channel.is_none());

    press(&mut app, ch('s')).await;
    assert_eq!(picker(&app), (0, ContainerAction::Shell));
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());
    press(&mut app, ch('s')).await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.requires_terminal_suspend {
        Some(SuspendAction::PodShell { pod, container, .. }) => {
            assert_eq!(pod, "multi");
            assert_eq!(container.as_deref(), Some("app"));
        }
        _ => panic!("expected a pending pod shell"),
    }
}

#[tokio::test]
async fn l_and_s_on_a_single_container_pod_go_straight_to_logs_and_shell() {
    let (mut app, _rx) = common::app().await;
    seed_table(&mut app, ResourceKind::Pods, pods(&["pod-a"]));
    press(&mut app, ch('l')).await;
    assert!(app.modal.is_none());
    match &app.active_view {
        ActiveView::Logs(l) => {
            assert_eq!(l.pod_name, "pod-a");
            assert!(l.container.is_none());
            assert!(
                l.lines[0].contains("Streaming logs for pod default/pod-a"),
                "line: {}",
                l.lines[0]
            );
        }
        _ => panic!("expected the logs view"),
    }
    let channel = app.active_log_channel.clone().expect("log channel");
    app.handle_stream_event(channel, json!({ "line": "hello from the pod" }));
    let screen = common::render_app(&mut app, 120, 30);
    assert!(screen.contains("hello from the pod"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('s')).await;
    match &app.requires_terminal_suspend {
        Some(SuspendAction::PodShell { pod, container, .. }) => {
            assert_eq!(pod, "pod-a");
            assert!(container.is_none());
        }
        _ => panic!("expected a pending pod shell"),
    }
    app.requires_terminal_suspend = None;

    press(&mut app, key(KeyCode::Enter)).await;
    assert!(
        matches!(app.active_view, ActiveView::Logs(_)),
        "enter on a pod opens its logs"
    );
    press(&mut app, key(KeyCode::Esc)).await;

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web", "namespace": "default", "kind": "Deployment" })],
    );
    press(&mut app, ch('l')).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));
    assert!(
        toast(&app).starts_with("Logs only available for Pods")
            || toast(&app).starts_with("No running pods found"),
        "toast: {}",
        toast(&app)
    );
    press(&mut app, ch('s')).await;
    assert!(app.requires_terminal_suspend.is_none());
    assert!(
        toast(&app).starts_with("Shell only available for Pods"),
        "toast: {}",
        toast(&app)
    );

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web-1", "namespace": "default", "kind": "Pod" })],
    );
    press(&mut app, ch('l')).await;
    assert!(matches!(app.active_view, ActiveView::Logs(_)));
}

#[tokio::test]
async fn action_palette_opens_with_x_filters_navigates_and_runs_the_chosen_action() {
    let (mut app, _rx) = common::app().await;
    seed_table(&mut app, ResourceKind::Pods, pods(&["pod-a"]));
    let palette = |app: &App| match &app.modal {
        Some(Modal::ActionPalette {
            actions,
            selected_idx,
            filter,
            resource_kind,
            ..
        }) => {
            assert_eq!(resource_kind, "Pod");
            (actions.len(), *selected_idx, filter.clone())
        }
        other => panic!("expected the action palette, got {:?}", other),
    };

    press(&mut app, ch('x')).await;
    let (count, _, _) = palette(&app);
    assert_eq!(count, 12);
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(palette(&app).1, 1);
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(palette(&app).1, 0);
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(palette(&app).1, count - 1, "up wraps");
    press(&mut app, ctrl('j')).await;
    assert_eq!(palette(&app).1, 0, "down wraps");
    press(&mut app, ctrl('k')).await;
    assert_eq!(palette(&app).1, count - 1);
    press(&mut app, key(KeyCode::Null)).await;
    assert_eq!(palette(&app).1, count - 1);
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("pod-a"), "screen: {}", screen);

    type_str(&mut app, "shells").await;
    assert_eq!(palette(&app).1, 0, "typing resets the selection");
    press(&mut app, key(KeyCode::Backspace)).await;
    assert_eq!(palette(&app).2, "shell");
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert!(matches!(
        app.requires_terminal_suspend,
        Some(SuspendAction::PodShell { .. })
    ));
    app.requires_terminal_suspend = None;

    press(&mut app, ch('x')).await;
    type_str(&mut app, "port").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(
        app.modal,
        Some(Modal::PortForward {
            container_port: 8080,
            ..
        })
    ));
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('x')).await;
    type_str(&mut app, "del").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.modal {
        Some(Modal::Confirm {
            action_name,
            is_destructive,
            ..
        }) => {
            assert_eq!(action_name, "delete:Pod:default:pod-a");
            assert!(is_destructive);
        }
        other => panic!("expected a confirm dialog, got {:?}", other),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('x')).await;
    type_str(&mut app, "no such action").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none(), "enter on an empty list just closes");

    press(&mut app, ch('x')).await;
    type_str(&mut app, "/crashloop").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Assistant));
    assert_eq!(app.assistant_state.input, "/crashloop pod-a");
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(
        table(&app).kind,
        ResourceKind::Pods,
        "esc pops back to the table it was opened from"
    );
    assert_eq!(
        table(&app).raw_items.len(),
        1,
        "the informer cache re-primes the restored table"
    );

    press(&mut app, ch('x')).await;
    // "logs" alone would hit the CrashLoop playbook, whose description
    // mentions container logs and which sorts first.
    type_str(&mut app, "view live logs").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Logs(_)));
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('x')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());
}

#[tokio::test]
async fn action_palette_offers_kind_specific_actions_for_workloads_and_nodes() {
    let (mut app, _rx) = common::app().await;
    seed_table(
        &mut app,
        ResourceKind::Deployments,
        vec![json!({ "name": "web", "namespace": "default" })],
    );
    press(&mut app, ch('x')).await;
    type_str(&mut app, "scale").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.modal, Some(Modal::Scale { .. })));
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('x')).await;
    type_str(&mut app, "restart").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.modal {
        Some(Modal::Confirm { action_name, .. }) => {
            assert_eq!(action_name, "restart:Deployment:default:web")
        }
        other => panic!("expected a confirm dialog, got {:?}", other),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('x')).await;
    type_str(&mut app, "/rollout").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Assistant));
    assert_eq!(app.assistant_state.input, "/rollout web");
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(table(&app).kind, ResourceKind::Deployments);

    press(&mut app, ch('x')).await;
    // Plain "pods" would hit the relationship tree, whose description maps
    // "Workload -> ReplicaSets -> Pods -> Services" and which sorts first.
    type_str(&mut app, "view pods").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).kind, ResourceKind::Pods);
    assert_eq!(app.filter_buffer, "web");
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(
        app.filter_buffer, "",
        "esc clears the filter the jump left behind"
    );

    seed_table(
        &mut app,
        ResourceKind::Nodes,
        vec![json!({ "name": "node-1" })],
    );
    press(&mut app, ch('x')).await;
    type_str(&mut app, "inspect").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.active_view {
        ActiveView::NodeInspector(ni) => assert_eq!(ni.node_name, "node-1"),
        _ => panic!("expected the node inspector"),
    }
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(table(&app).kind, ResourceKind::Nodes);

    press(&mut app, ch('x')).await;
    type_str(&mut app, "tree").await;
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.active_view {
        ActiveView::Tree(t) => assert_eq!(
            (t.root_kind.as_str(), t.root_name.as_str()),
            ("Node", "node-1")
        ),
        _ => panic!("expected the tree view"),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    set_table(
        &mut app,
        ResourceKind::ConfigMaps,
        vec![json!({ "name": "cfg", "namespace": "default" })],
    );
    press(&mut app, ch('x')).await;
    type_str(&mut app, "ask ai").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Assistant));
    assert!(
        app.assistant_state.input.contains("ConfigMap 'cfg'"),
        "input: {}",
        app.assistant_state.input
    );
}

#[tokio::test]
async fn m_opens_the_metrics_timeline_and_keys_change_its_range() {
    let (mut app, _rx) = common::app().await;
    set_table(&mut app, ResourceKind::Pods, pods(&["pod-a"]));
    let panel = |app: &App| match &app.modal {
        Some(Modal::MetricsTimeline(p)) => (p.target_kind.clone(), p.namespace.clone(), p.range),
        other => panic!("expected the metrics timeline, got {:?}", other),
    };
    press(&mut app, ch('m')).await;
    assert_eq!(
        panel(&app),
        (
            "Pod".to_string(),
            Some("default".to_string()),
            MetricsTimeRange::FiveMin
        )
    );
    press(&mut app, key(KeyCode::Tab)).await;
    assert_eq!(panel(&app).2, MetricsTimeRange::TenMin);
    press(&mut app, ch('3')).await;
    assert_eq!(panel(&app).2, MetricsTimeRange::ThirtyMin);
    press(&mut app, ch('4')).await;
    assert_eq!(panel(&app).2, MetricsTimeRange::OneHour);
    press(&mut app, ch('1')).await;
    assert_eq!(panel(&app).2, MetricsTimeRange::FiveMin);
    press(&mut app, ch('2')).await;
    assert_eq!(panel(&app).2, MetricsTimeRange::TenMin);
    press(&mut app, ch('r')).await;
    assert_eq!(toast(&app), "Refreshing metrics...");
    press(&mut app, ch('z')).await;
    assert!(app.modal.is_some());
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("pod-a"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());

    set_table(
        &mut app,
        ResourceKind::Nodes,
        vec![json!({ "name": "node-1" })],
    );
    press(&mut app, ch('m')).await;
    assert_eq!(panel(&app).0, "Node");
    assert!(panel(&app).1.is_none());
    press(&mut app, ch('r')).await;
    assert_eq!(toast(&app), "Refreshing metrics...");
    press(&mut app, key(KeyCode::Esc)).await;

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web", "namespace": "default", "kind": "Deployment" })],
    );
    press(&mut app, ch('m')).await;
    assert!(app.modal.is_none());
    assert!(
        toast(&app).starts_with("Metrics timeline is only available for Pods"),
        "toast: {}",
        toast(&app)
    );

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web-1", "namespace": "default", "kind": "Pod" })],
    );
    press(&mut app, ch('m')).await;
    assert_eq!(panel(&app).0, "Pod");
}

fn events() -> Vec<Value> {
    vec![
        json!({ "name": "e1", "namespace": "default", "reason": "BackOff", "type": "Warning", "object": "Pod/web-1", "message": "restarting", "age": "1m" }),
        json!({ "name": "e2", "namespace": "default", "reason": "BackOff", "type": "Warning", "object": "Pod/web-2", "message": "restarting", "age": "2m" }),
        json!({ "name": "e3", "namespace": "default", "reason": "Pulled", "type": "Normal", "object": "Pod/web-1", "message": "pulled", "age": "3m" }),
    ]
}

#[tokio::test]
async fn shift_r_on_a_narrow_events_table_opens_the_reason_rail_modal() {
    let (mut app, _rx) = common::app().await;
    set_table(&mut app, ResourceKind::Events, events());
    let rail = |app: &App| match &app.modal {
        Some(Modal::ReasonRail {
            tallies,
            selected_idx,
            ..
        }) => (tallies.len(), *selected_idx),
        other => panic!("expected the reason rail, got {:?}", other),
    };
    press(&mut app, ch('R')).await;
    assert_eq!(rail(&app), (2, 0));
    press(&mut app, ch('j')).await;
    assert_eq!(rail(&app).1, 1);
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(rail(&app).1, 1, "no wrap past the end");
    press(&mut app, ch('z')).await;
    assert_eq!(rail(&app).1, 1);
    press(&mut app, ch('k')).await;
    assert_eq!(rail(&app).1, 0);
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(rail(&app).1, 0);
    let screen = common::render_app(&mut app, 100, 40);
    assert!(screen.contains("BackOff"), "screen: {}", screen);
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert_eq!(table(&app).active_reason_filter.as_deref(), Some("BackOff"));
    assert_eq!(table(&app).filtered_indices.len(), 2);
    assert_eq!(toast(&app), "Filtered events by reason: BackOff");

    press(&mut app, ch('R')).await;
    press(&mut app, ch('c')).await;
    assert!(table(&app).active_reason_filter.is_none());
    assert_eq!(table(&app).filtered_indices.len(), 3);
    assert_eq!(toast(&app), "Cleared event reason filter");

    press(&mut app, ch('R')).await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, ch('R')).await;
    press(&mut app, key(KeyCode::Backspace)).await;
    assert!(table(&app).active_reason_filter.is_none());

    press(&mut app, ch('R')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());
}

#[tokio::test]
async fn shift_r_on_a_wide_events_table_focuses_the_rail_whose_keys_pick_a_reason() {
    let (mut app, _rx) = common::app().await;
    set_table(&mut app, ResourceKind::Events, events());
    table_mut(&mut app).last_area_width.set(120);
    press(&mut app, ch('R')).await;
    assert!(table(&app).reason_rail_focused);
    assert!(app.modal.is_none());
    press(&mut app, ch('j')).await;
    assert_eq!(table(&app).selected_reason_idx, 1);
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(table(&app).selected_reason_idx, 1);
    press(&mut app, ch('k')).await;
    assert_eq!(table(&app).selected_reason_idx, 0);
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(table(&app).selected_reason_idx, 0);
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(!table(&app).reason_rail_focused);
    assert_eq!(table(&app).active_reason_filter.as_deref(), Some("BackOff"));
    assert_eq!(toast(&app), "Filtered events by reason: BackOff");

    press(&mut app, ch('R')).await;
    press(&mut app, ch('c')).await;
    assert!(!table(&app).reason_rail_focused);
    assert!(table(&app).active_reason_filter.is_none());
    assert_eq!(toast(&app), "Cleared event reason filter");

    press(&mut app, ch('R')).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(
        table(&app).reason_rail_focused,
        "Esc never reaches the rail: the global back/clear handler claims it first, \
         so the rail's own Esc arm (app.rs:2145) is dead"
    );
    press(&mut app, ch('R')).await;
    assert!(!table(&app).reason_rail_focused, "'R' does leave the rail");

    press(&mut app, ch('R')).await;
    press(&mut app, ch('w')).await;
    assert!(
        table(&app).reason_rail_focused,
        "unhandled keys fall through without leaving the rail"
    );
    assert!(
        toast(&app).starts_with("Warning Triage"),
        "toast: {}",
        toast(&app)
    );
    press(&mut app, ch('R')).await;

    set_table(&mut app, ResourceKind::Events, vec![]);
    table_mut(&mut app).last_area_width.set(120);
    press(&mut app, ch('R')).await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(
        !table(&app).reason_rail_focused,
        "enter with no reasons just unfocuses"
    );
    assert!(table(&app).active_reason_filter.is_none());
}

#[tokio::test]
async fn w_toggles_warning_triage_on_the_events_table() {
    let (mut app, _rx) = common::app().await;
    set_table(&mut app, ResourceKind::Events, events());
    press(&mut app, ch('w')).await;
    assert!(table(&app).warning_triage);
    assert_eq!(table(&app).filtered_indices.len(), 2);
    assert_eq!(toast(&app), "Warning Triage: ON (2 warnings)");
    press(&mut app, ch('w')).await;
    assert!(!table(&app).warning_triage);
    assert_eq!(toast(&app), "Warning Triage: OFF (all 3 events)");

    set_table(&mut app, ResourceKind::Pods, pods(&["p"]));
    press(&mut app, ch('w')).await;
    assert!(
        !table(&app).warning_triage,
        "'w' means nothing outside events"
    );
}

#[tokio::test]
async fn events_table_keys_act_on_the_involved_object() {
    let (mut app, _rx) = common::app().await;
    seed_table(&mut app, ResourceKind::Events, events());

    press(&mut app, ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied event message to clipboard");
    press(&mut app, ctrl('y')).await;
    assert!(
        toast(&app).starts_with("✓ Copied deep link: "),
        "toast: {}",
        toast(&app)
    );
    assert!(toast(&app).contains("web-1"));

    press(&mut app, ch('l')).await;
    match &app.active_view {
        ActiveView::Logs(l) => assert_eq!(l.pod_name, "web-1"),
        _ => panic!("expected the logs view"),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('x')).await;
    match &app.modal {
        Some(Modal::ActionPalette {
            resource_kind,
            resource_name,
            ..
        }) => {
            assert_eq!(
                (resource_kind.as_str(), resource_name.as_str()),
                ("Pod", "web-1")
            );
        }
        other => panic!("expected the action palette, got {:?}", other),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('t')).await;
    match &app.active_view {
        ActiveView::Tree(t) => assert_eq!(
            (t.root_kind.as_str(), t.root_name.as_str()),
            ("Pod", "web-1")
        ),
        _ => panic!("expected the tree view"),
    }
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).kind, ResourceKind::Pods);
    assert_eq!(app.filter_buffer, "web-1");
    assert_eq!(toast(&app), "Jumped to Pod 'web-1'");
    press(&mut app, key(KeyCode::Esc)).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(table(&app).kind, ResourceKind::Events);

    seed_table(
        &mut app,
        ResourceKind::Events,
        vec![
            json!({ "name": "e", "namespace": "default", "reason": "ScalingReplicaSet", "type": "Normal", "involvedObject": { "kind": "Deployment", "name": "api" } }),
            json!({ "name": "e2", "namespace": "default", "reason": "Odd", "type": "Normal", "message": "no object" }),
        ],
    );
    press(&mut app, ch('l')).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));
    assert_eq!(
        toast(&app),
        "Logs only available for Pods (event target is Deployment)"
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).kind, ResourceKind::Deployments);
    assert_eq!(app.filter_buffer, "api");
    assert_eq!(toast(&app), "Jumped to Deployment 'api'");
    press(&mut app, key(KeyCode::Esc)).await;
    press(&mut app, key(KeyCode::Esc)).await;

    press(&mut app, ch('j')).await;
    app.toast = None;
    for k in [ch('x'), ch('t'), ctrl('y'), key(KeyCode::Enter)] {
        press(&mut app, k).await;
        assert!(app.modal.is_none());
        assert!(matches!(app.active_view, ActiveView::Table(_)));
        assert!(
            app.toast.is_none(),
            "an event without an object has nothing to act on"
        );
    }
}

#[tokio::test]
async fn table_navigation_keys_move_page_and_mark_rows() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Pods,
        pods(&["a", "b", "c", "d", "e"]),
    );
    app.set_toast("hello".into(), Style::default());
    press(&mut app, ch('j')).await;
    assert_eq!(table(&app).selected_idx, 1);
    assert!(app.toast.is_none(), "moving dismisses the toast");
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(table(&app).selected_idx, 2);
    press(&mut app, ch('k')).await;
    press(&mut app, key(KeyCode::Up)).await;
    assert_eq!(table(&app).selected_idx, 0);
    press(&mut app, ch('k')).await;
    assert_eq!(table(&app).selected_idx, 0, "no wrap at the top");
    press(&mut app, ch('G')).await;
    assert_eq!(table(&app).selected_idx, 4);
    press(&mut app, ch('g')).await;
    assert_eq!(table(&app).selected_idx, 0);
    press(&mut app, key(KeyCode::End)).await;
    assert_eq!(table(&app).selected_idx, 4);
    press(&mut app, key(KeyCode::Home)).await;
    assert_eq!(table(&app).selected_idx, 0);
    press(&mut app, key(KeyCode::PageDown)).await;
    assert_eq!(table(&app).selected_idx, 4);
    press(&mut app, key(KeyCode::PageUp)).await;
    assert_eq!(table(&app).selected_idx, 0);
    press(&mut app, key(KeyCode::PageDown)).await;
    press(&mut app, ctrl('u')).await;
    assert_eq!(table(&app).selected_idx, 0);

    press(&mut app, ch(' ')).await;
    assert!(table(&app).marked_indices.contains(&0));
    press(&mut app, ch(' ')).await;
    assert!(table(&app).marked_indices.is_empty());
    press(&mut app, key(KeyCode::Null)).await;
    assert_eq!(table(&app).selected_idx, 0);
    let screen = common::render_app(&mut app, 120, 30);
    assert!(screen.contains("e"), "screen: {}", screen);
}

#[tokio::test]
async fn c_copies_the_selection_or_marked_names_and_shift_c_copies_yaml() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ch('c')).await;
    assert!(app.toast.is_none(), "nothing to copy on an empty table");
    press(&mut app, ch('C')).await;
    assert!(app.toast.is_none());
    press(&mut app, ctrl('y')).await;
    assert!(app.toast.is_none());

    set_table(
        &mut app,
        ResourceKind::Pods,
        pods(&["pod-a", "pod-b", "pod-c"]),
    );
    press(&mut app, ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied 'pod-a' to clipboard");
    press(&mut app, ch(' ')).await;
    press(&mut app, ch('j')).await;
    press(&mut app, ch(' ')).await;
    press(&mut app, ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied 2 resource names to clipboard");
    press(&mut app, shift(KeyCode::Char('C'))).await;
    assert_eq!(toast(&app), "✓ Copied resource YAML to clipboard");
    press(&mut app, ctrl('y')).await;
    assert!(
        toast(&app).starts_with("✓ Copied deep link: "),
        "toast: {}",
        toast(&app)
    );
    assert!(toast(&app).contains("pod-b"));
    assert!(toast(&app).contains("test-cluster"));
}

#[tokio::test]
async fn enter_drills_into_namespaces_crds_controllers_and_nodes() {
    let (mut app, _rx) = common::app().await;
    set_table(
        &mut app,
        ResourceKind::Namespaces,
        vec![json!({ "name": "team-a" })],
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.active_namespace, "team-a");
    assert_eq!(table(&app).kind, ResourceKind::Pods);
    assert_eq!(app.nav_stack.len(), 1);

    app.crds = vec![widget_crd()];
    set_table(
        &mut app,
        ResourceKind::CustomResourceDefinitions,
        vec![json!({ "name": "widgets.example.com" })],
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(&table(&app).kind, ResourceKind::CustomResource(c) if c.kind == "Widget"));

    set_table(
        &mut app,
        ResourceKind::CustomResourceDefinitions,
        vec![json!({ "name": "unknown.example.com" })],
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(
        table(&app).kind,
        ResourceKind::CustomResourceDefinitions,
        "an undiscovered CRD stays put"
    );

    set_table(
        &mut app,
        ResourceKind::StatefulSets,
        vec![json!({ "name": "db", "namespace": "team-a" })],
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).kind, ResourceKind::Pods);
    assert_eq!(app.filter_buffer, "db");

    set_table(
        &mut app,
        ResourceKind::Nodes,
        vec![json!({ "name": "node-1" })],
    );
    app.node_metrics_history.insert(
        "node-1".into(),
        std::iter::once(srelens_kube::metrics::MetricSample {
            timestamp_epoch_ms: 1,
            cpu_millicores: 7,
            memory_mib: 9,
        })
        .collect(),
    );
    press(&mut app, key(KeyCode::Enter)).await;
    match &app.active_view {
        ActiveView::NodeInspector(ni) => {
            assert_eq!(ni.node_name, "node-1");
            assert_eq!(
                ni.cpu_history,
                vec![7],
                "existing history seeds the inspector"
            );
        }
        _ => panic!("expected the node inspector"),
    }
    let screen = common::render_app(&mut app, 120, 40);
    assert!(screen.contains("node-1"), "screen: {}", screen);

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web", "namespace": "team-a", "kind": "Deployment" })],
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).kind, ResourceKind::Pods);
    assert_eq!(app.filter_buffer, "web");

    set_table(
        &mut app,
        ResourceKind::Workloads,
        vec![json!({ "name": "web-1", "namespace": "team-a", "kind": "Pod" })],
    );
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(matches!(app.active_view, ActiveView::Logs(_)));

    set_table(&mut app, ResourceKind::Pods, vec![]);
    press(&mut app, key(KeyCode::Enter)).await;
    assert!(
        matches!(app.active_view, ActiveView::Table(_)),
        "enter on an empty table does nothing"
    );
}

#[tokio::test]
async fn esc_pops_the_navigation_stack_after_clearing_filters() {
    let (mut app, _rx) = common::app().await;
    set_table(&mut app, ResourceKind::Pods, pods(&["a", "b"]));
    app.switch_view_to_kind(ResourceKind::Nodes).await;
    assert_eq!(app.nav_stack.len(), 1);
    table_mut(&mut app).set_items(
        vec![
            json!({ "name": "beta-1" }),
            json!({ "name": "beta-2" }),
            json!({ "name": "gamma" }),
        ],
        "",
    );

    press(&mut app, ch('/')).await;
    // Two matches, not one: a single match makes Enter drill into the row.
    type_str(&mut app, "beta").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).filtered_indices.len(), 2);
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(
        table(&app).kind,
        ResourceKind::Nodes,
        "the first esc only clears the filter"
    );
    assert_eq!(table(&app).filtered_indices.len(), 3);

    table_mut(&mut app).active_reason_filter = Some("x".into());
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(table(&app).active_reason_filter.is_none());
    assert_eq!(table(&app).kind, ResourceKind::Nodes);

    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(table(&app).kind, ResourceKind::Pods);
    assert!(app.nav_stack.is_empty());
    assert_eq!(
        app.current_watch_channel.as_deref(),
        Some("watch:test-cluster:default:pods")
    );
    press(&mut app, key(KeyCode::Esc)).await;
    assert_eq!(
        table(&app).kind,
        ResourceKind::Pods,
        "esc on the root view is a no-op"
    );
}

#[tokio::test]
async fn yaml_view_keys_scroll_copy_and_flag_an_edit() {
    let (mut app, _rx) = common::app().await;
    let content: String = (0..40).map(|i| format!("line-{}\n", i)).collect();
    app.active_view = ActiveView::Yaml(YamlViewState::new(
        "p".into(),
        "Pod".into(),
        Some("default".into()),
        content,
    ));
    let offset = |app: &App| match &app.active_view {
        ActiveView::Yaml(y) => y.scroll_offset,
        _ => panic!("expected yaml"),
    };
    press(&mut app, ch('j')).await;
    press(&mut app, key(KeyCode::Down)).await;
    assert_eq!(offset(&app), 2);
    press(&mut app, ch('k')).await;
    assert_eq!(offset(&app), 1);
    press(&mut app, ctrl('d')).await;
    assert_eq!(offset(&app), 11);
    press(&mut app, ctrl('u')).await;
    assert_eq!(offset(&app), 1);
    press(&mut app, ch('G')).await;
    assert!(offset(&app) > 1);
    press(&mut app, ch('g')).await;
    assert_eq!(offset(&app), 0);
    press(&mut app, ch('c')).await;
    assert_eq!(toast(&app), "✓ Copied YAML to clipboard");
    press(&mut app, ch('e')).await;
    assert!(matches!(
        app.requires_terminal_suspend,
        Some(SuspendAction::EditYaml)
    ));
    let screen = common::render_app(&mut app, 120, 30);
    assert!(screen.contains("line-0"), "screen: {}", screen);
}

// ---------------------------------------------------------------------------
// Watch pool bookkeeping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restart_active_watch_primes_from_the_cache_and_keeps_channels_pooled() {
    let (mut app, _rx) = common::app().await;
    app.resource_cache.insert(
        ("test-cluster".into(), "default".into(), "pods".into()),
        pods(&["cached"]),
    );
    app.restart_active_watch().await;
    assert!(!table(&app).is_loading);
    assert_eq!(
        table(&app).selected_resource_name().as_deref(),
        Some("cached")
    );

    app.switch_view_to_kind(ResourceKind::Nodes).await;
    assert!(table(&app).is_loading, "no cache for nodes");
    assert_eq!(app.active_watch_pool.len(), 2);
    assert_eq!(app.active_watch_pool[1], "watch:test-cluster:default:nodes");
    app.switch_view_to_kind(ResourceKind::Pods).await;
    assert_eq!(
        app.active_watch_pool.len(),
        2,
        "revisiting a kind does not duplicate its channel"
    );
    assert_eq!(
        app.active_watch_pool[1], "watch:test-cluster:default:pods",
        "most recently used goes last"
    );

    app.switch_view_to_kind(ResourceKind::Overview).await;
    let before = app.active_watch_pool.clone();
    app.restart_active_watch().await;
    assert_eq!(
        app.active_watch_pool, before,
        "non-table views start no watch"
    );
}

#[tokio::test]
async fn the_watch_pool_evicts_its_oldest_channel_past_twenty() {
    let (mut app, _rx) = common::app().await;
    app.active_watch_pool = (0..20)
        .map(|i| format!("watch:test-cluster:default:fake{}", i))
        .collect();
    app.active_watch_channels = app.active_watch_pool.iter().cloned().collect();
    app.switch_view_to_kind(ResourceKind::Services).await;
    assert_eq!(app.active_watch_pool.len(), 20);
    assert!(!app
        .active_watch_pool
        .contains(&"watch:test-cluster:default:fake0".to_string()));
    assert!(!app
        .active_watch_channels
        .contains("watch:test-cluster:default:fake0"));
    assert_eq!(
        app.active_watch_pool[19],
        "watch:test-cluster:default:services"
    );
}

#[tokio::test]
async fn the_workloads_view_watches_every_constituent_kind_and_rebuilds_from_the_cache() {
    let (mut app, _rx) = common::app().await;
    press(&mut app, ch(':')).await;
    type_str(&mut app, "wl").await;
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(table(&app).kind, ResourceKind::Workloads);
    assert_eq!(
        app.current_watch_channel.as_deref(),
        Some("workloads:test-cluster:default")
    );
    assert!(table(&app).is_loading);
    for kind in [
        "deployments",
        "statefulsets",
        "daemonsets",
        "pods",
        "cronjobs",
    ] {
        let ch = format!("watch:test-cluster:default:{}", kind);
        assert!(app.active_watch_pool.contains(&ch), "missing {}", ch);
    }

    app.handle_stream_event(
        "watch:test-cluster:default:deployments".into(),
        json!([{ "name": "web", "namespace": "default", "ready": "1/2", "age": "5m" }]),
    );
    app.handle_stream_event(
        "watch:test-cluster:default:pods".into(),
        json!([{ "name": "web-1", "namespace": "default", "phase": "Running", "ready": "0/1", "restarts": 3 }]),
    );
    assert!(!table(&app).is_loading);
    assert_eq!(table(&app).raw_items.len(), 2);
    assert_eq!(table(&app).raw_items[0]["status"], "Degraded");
    assert_eq!(table(&app).raw_items[1]["status"], "NotReady");

    app.restart_active_watch().await;
    assert_eq!(
        table(&app).raw_items.len(),
        2,
        "restarting rebuilds from the cache"
    );
    let mut channels: Vec<String> = app.active_watch_channels.iter().cloned().collect();
    channels.sort();
    assert_eq!(
        channels,
        vec![
            "watch:test-cluster:default:cronjobs".to_string(),
            "watch:test-cluster:default:daemonsets".to_string(),
            "watch:test-cluster:default:deployments".to_string(),
            "watch:test-cluster:default:pods".to_string(),
            "watch:test-cluster:default:statefulsets".to_string(),
        ],
        "the workloads view watches exactly its five constituent kinds"
    );
}

#[tokio::test]
async fn test_nodes_table_press_s_triggers_node_shell() {
    let (mut app, _rx) = common::app().await;
    seed_table(
        &mut app,
        ResourceKind::Nodes,
        vec![json!({ "name": "node-a", "status": "Ready", "roles": "master" })],
    );
    press(&mut app, ch('s')).await;
    match &app.requires_terminal_suspend {
        Some(SuspendAction::NodeShell { node }) => {
            assert_eq!(node, "node-a");
        }
        _ => panic!("expected NodeShell suspend action"),
    }
}

#[tokio::test]
async fn test_node_inspector_press_s_on_selected_pod() {
    let (mut app, _rx) = common::app().await;
    let mut ni = NodeInspectorState::new("node-1".into());
    let details = srelens_kube::node_inspector::NodeInspectorDetails {
        name: "node-1".into(),
        status: "Ready".into(),
        pods: vec![srelens_kube::node_inspector::NodePodItem {
            name: "test-pod".into(),
            namespace: "custom-ns".into(),
            phase: "Running".into(),
            ready_containers: "1/1".into(),
            restarts: 0,
            age: "1d".into(),
            cpu_requests_millicores: 100,
            mem_requests_mib: 256,
            gpu_requests: 0,
            gpu_mem_requests_mib: 0,
            pod_ip: "10.244.0.5".into(),
        }],
        ..Default::default()
    };
    ni.set_details(details);
    app.active_view = ActiveView::NodeInspector(ni);

    press(&mut app, ch('s')).await;
    match &app.requires_terminal_suspend {
        Some(SuspendAction::PodShell { pod, namespace, .. }) => {
            assert_eq!(pod, "test-pod");
            assert_eq!(namespace.as_deref(), Some("custom-ns"));
        }
        _ => panic!("expected PodShell suspend action with custom-ns namespace"),
    }
}

#[tokio::test]
async fn test_node_inspector_press_s_when_no_pods_triggers_node_shell() {
    let (mut app, _rx) = common::app().await;
    let mut ni = NodeInspectorState::new("node-empty".into());
    let details = srelens_kube::node_inspector::NodeInspectorDetails {
        name: "node-empty".into(),
        status: "Ready".into(),
        pods: vec![],
        ..Default::default()
    };
    ni.set_details(details);
    app.active_view = ActiveView::NodeInspector(ni);

    press(&mut app, ch('s')).await;
    match &app.requires_terminal_suspend {
        Some(SuspendAction::NodeShell { node }) => {
            assert_eq!(node, "node-empty");
        }
        _ => panic!("expected NodeShell suspend action when no pods scheduled"),
    }
}

#[tokio::test]
async fn test_node_inspector_press_capital_s_triggers_node_shell_even_with_pods() {
    let (mut app, _rx) = common::app().await;
    let mut ni = NodeInspectorState::new("node-2".into());
    let details = srelens_kube::node_inspector::NodeInspectorDetails {
        name: "node-2".into(),
        status: "Ready".into(),
        pods: vec![srelens_kube::node_inspector::NodePodItem {
            name: "test-pod-2".into(),
            namespace: "default".into(),
            phase: "Running".into(),
            ready_containers: "1/1".into(),
            restarts: 0,
            age: "1d".into(),
            cpu_requests_millicores: 100,
            mem_requests_mib: 256,
            gpu_requests: 0,
            gpu_mem_requests_mib: 0,
            pod_ip: "10.244.0.6".into(),
        }],
        ..Default::default()
    };
    ni.set_details(details);
    app.active_view = ActiveView::NodeInspector(ni);

    press(&mut app, ch('S')).await;
    match &app.requires_terminal_suspend {
        Some(SuspendAction::NodeShell { node }) => {
            assert_eq!(node, "node-2");
        }
        _ => panic!("expected NodeShell suspend action when pressing capital S"),
    }
}

#[tokio::test]
async fn non_pod_and_custom_resources_reject_pod_actions() {
    let (mut app, _rx) = common::app().await;
    let secret_store_crd = ResourceKind::CustomResource(CrdMeta {
        crd_name: "secretstores.external-secrets.io".to_string(),
        group: "external-secrets.io".to_string(),
        version: "v1beta1".to_string(),
        kind: "SecretStore".to_string(),
        plural: "secretstores".to_string(),
        singular: "secretstore".to_string(),
        namespaced: true,
        short_names: vec![],
        printer_columns: vec![],
    });

    for kind in [secret_store_crd, ResourceKind::ConfigMaps, ResourceKind::Secrets] {
        set_table(
            &mut app,
            kind.clone(),
            vec![json!({ "name": "my-resource", "namespace": "default" })],
        );

        // 1. Port forward rejected
        press(&mut app, ch('f')).await;
        assert!(app.modal.is_none());
        assert_eq!(toast(&app), "Port forward is only available for Pods and Services");

        press(&mut app, ch('F')).await;
        assert!(app.modal.is_none());
        assert_eq!(toast(&app), "Port forward is only available for Pods and Services");

        // 2. Logs rejected
        press(&mut app, ch('l')).await;
        assert_eq!(toast(&app), "Logs are only available for Pods and Workloads");

        // 3. Rollout restart rejected
        press(&mut app, ch('r')).await;
        assert!(app.modal.is_none());
        assert_eq!(toast(&app), "Rollout restart is only available for Deployments, StatefulSets, and DaemonSets");

        // 4. Scale rejected
        press(&mut app, ctrl('s')).await;
        assert!(app.modal.is_none());
        assert_eq!(toast(&app), "Scale is only available for Deployments and StatefulSets");

        // 5. Shell rejected
        press(&mut app, ch('s')).await;
        assert_eq!(toast(&app), "Shell only available for Pods and Nodes");
    }
}

#[tokio::test]
async fn helm_detail_manifest_search_and_navigation_input_flow() {
    let (mut app, _rx) = common::app().await;
    let mut detail_state = srelens_tui::views::HelmDetailViewState::new("my-release".into(), "default".into());
    detail_state.set_detail(srelens_kube::helm::HelmReleaseDetail {
        name: "my-release".into(),
        namespace: "default".into(),
        revision: 1,
        status: "deployed".into(),
        chart: "my-chart".into(),
        chart_version: "1.0.0".into(),
        app_version: "1.0.0".into(),
        updated: "2026-09-10T12:00:00Z".into(),
        values_yaml: "".into(),
        chart_values_yaml: "".into(),
        computed_values_yaml: "".into(),
        manifest: "---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: app-token\ndata:\n  github_token: Z2hw...\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: app\n".into(),
        notes: "".into(),
        history: vec![],
    });
    detail_state.set_tab(srelens_tui::views::HelmDetailTab::Manifest);
    app.active_view = srelens_tui::app::ActiveView::HelmDetail(detail_state);

    // 1. Press '/' to enter search mode
    press(&mut app, ch('/')).await;
    assert_eq!(app.input_mode, srelens_tui::ui::statusbar::InputMode::Filter);

    // 2. Type "token"
    for c in "token".chars() {
        press(&mut app, ch(c)).await;
    }
    assert_eq!(app.filter_buffer, "token");
    if let srelens_tui::app::ActiveView::HelmDetail(ref detail) = app.active_view {
        assert_eq!(detail.search_query, "token");
        assert_eq!(detail.search_matches.len(), 2);
        assert_eq!(detail.current_match_idx, Some(0));
        assert_eq!(detail.scroll_offset, detail.search_matches[0]);
    } else {
        panic!("expected HelmDetail view");
    }

    // 3. Press Enter to return to Normal mode with search query active
    press(&mut app, key(KeyCode::Enter)).await;
    assert_eq!(app.input_mode, srelens_tui::ui::statusbar::InputMode::Normal);
    assert_eq!(app.filter_buffer, "token");

    // 4. Press 'n' to go to next match
    press(&mut app, ch('n')).await;
    if let srelens_tui::app::ActiveView::HelmDetail(ref detail) = app.active_view {
        assert_eq!(detail.current_match_idx, Some(1));
        assert_eq!(detail.scroll_offset, detail.search_matches[1]);
    } else {
        panic!("expected HelmDetail view");
    }

    // 5. Press 'N' to go to previous match
    press(&mut app, ch('N')).await;
    if let srelens_tui::app::ActiveView::HelmDetail(ref detail) = app.active_view {
        assert_eq!(detail.current_match_idx, Some(0));
        assert_eq!(detail.scroll_offset, detail.search_matches[0]);
    } else {
        panic!("expected HelmDetail view");
    }

    // 6. Press Esc to clear filter
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(app.filter_buffer.is_empty());
    if let srelens_tui::app::ActiveView::HelmDetail(ref detail) = app.active_view {
        assert!(detail.search_query.is_empty());
        assert!(detail.search_matches.is_empty());
        assert!(detail.current_match_idx.is_none());
    } else {
        panic!("expected HelmDetail view");
    }
}

#[tokio::test]
async fn config_command_opens_tui_config_view_and_keys_adjust_values() {
    let (tx, _rx) = unbounded_channel();
    let mut app = App::new(
        Some("test-ctx".into()),
        Some("default".into()),
        false,
        None,
        vec![],
        tx,
    )
    .await
    .expect("app");

    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("tui.json");
    std::env::set_var("SRELENS_TUI_CONFIG_PATH", &config_path);

    // Initial state: pods table
    assert!(matches!(app.active_view, ActiveView::Table(_)));

    // Open :config
    press(&mut app, ch(':')).await;
    assert_eq!(app.input_mode, InputMode::Command);
    type_str(&mut app, "config").await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Active view is now TuiConfig
    assert!(matches!(app.active_view, ActiveView::TuiConfig(_)));

    // Initial config values
    assert_eq!(app.tui_config.command_popup_max_width, 65);
    assert_eq!(app.tui_config.command_popup_max_visible, 6);

    // Adjust width (+5 with 'l')
    press(&mut app, ch('l')).await;
    assert_eq!(app.tui_config.command_popup_max_width, 70);

    // Adjust width (-5 with 'h')
    press(&mut app, ch('h')).await;
    assert_eq!(app.tui_config.command_popup_max_width, 65);

    // Switch to visible rows field with 'j'
    press(&mut app, ch('j')).await;
    if let ActiveView::TuiConfig(ref s) = app.active_view {
        assert_eq!(s.selected_field, 1);
    }

    // Adjust visible rows (+1 with '+')
    press(&mut app, ch('+')).await;
    assert_eq!(app.tui_config.command_popup_max_visible, 7);

    // Reset defaults with 'r'
    press(&mut app, ch('r')).await;
    assert_eq!(app.tui_config.command_popup_max_width, 65);
    assert_eq!(app.tui_config.command_popup_max_visible, 6);

    // Press Esc pops back to table view
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(matches!(app.active_view, ActiveView::Table(_)));

    std::env::remove_var("SRELENS_TUI_CONFIG_PATH");
}
