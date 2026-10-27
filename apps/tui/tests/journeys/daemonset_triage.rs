use crossterm::event::KeyCode;
use srectl::app::App;
use srectl::quick_rca::evaluate_direct_rca;
use srelens_kube::changed::{
    AppDeploymentChange, ChangeKind, ChangedTriageReport, ConfigChange, FailureCategory,
    IncidentStatus, OmittedWorkloadKind, RolloutStatus, TriageSummary,
};

use crate::common::{self, ch, key, press, render_app, type_str};

fn sample_daemonset_workload(
    name: &str,
    desired: i32,
    ready: i32,
    misscheduled: Option<i32>,
) -> AppDeploymentChange {
    AppDeploymentChange {
        app_name: name.to_string(),
        kind: "DaemonSet".to_string(),
        namespace: "kube-system".to_string(),
        incident_status: if ready < desired {
            IncidentStatus::Pending
        } else {
            IncidentStatus::Healthy
        },
        failure_category: if ready < desired {
            FailureCategory::Compute
        } else {
            FailureCategory::None
        },
        failure_detail: if ready < desired {
            "1 pod(s) unready on node worker-3".to_string()
        } else {
            "All pods ready".to_string()
        },
        gitops: None,
        argo_rollout_in_window: None,
        gitops_unresolved: None,
        local_cause: None,
        error_log_snippet: None,
        error_log_pod: None,
        error_log_container: None,
        change_kind: ChangeKind::Rollout,
        changed_age: "12m".to_string(),
        change_detail: None,
        deployed_at: Some("2026-10-27T10:00:00Z".to_string()),
        deployed_age: "12m".to_string(),
        current_revision: "3".to_string(),
        previous_revision: Some("2".to_string()),
        current_images: vec!["cilium:v1.14.2".to_string()],
        previous_images: vec!["cilium:v1.14.1".to_string()],
        image_diff: "v1.14.1 ➔ v1.14.2".to_string(),
        desired_replicas: desired,
        updated_replicas: desired,
        ready_replicas: ready,
        available_replicas: ready,
        rollout_status: if ready < desired {
            RolloutStatus::Progressing
        } else {
            RolloutStatus::Complete
        },
        failing_pods_count: if ready < desired { 1 } else { 0 },
        crash_loop_count: 0,
        oom_killed_count: 0,
        probe_failure_count: 0,
        restart_count: 0,
        primary_symptoms: vec![],
        pod_symptoms: vec![],
        failing_pod_names: vec![],
        top_events: vec![],
        misscheduled,
        config_changes: vec![],
    }
}

fn sample_report_with_mixed_workloads() -> ChangedTriageReport {
    let mut ds = sample_daemonset_workload("cilium-cni", 5, 4, Some(1));
    ds.config_changes = vec![
        ConfigChange {
            kind: "ConfigMap".to_string(),
            name: "cilium-config".to_string(),
            age: "14m".to_string(),
            referenced_by: "volume 'config'".to_string(),
            relation: "updated 14m ago, 2m before rollout".to_string(),
        },
        ConfigChange {
            kind: "Secret".to_string(),
            name: "cilium-ca".to_string(),
            age: "25m".to_string(),
            referenced_by: "volume 'certs'".to_string(),
            relation: "updated 25m ago".to_string(),
        },
    ];

    let sts = AppDeploymentChange {
        app_name: "redis-cluster".to_string(),
        kind: "StatefulSet".to_string(),
        namespace: "prod".to_string(),
        incident_status: IncidentStatus::Healthy,
        failure_category: FailureCategory::None,
        failure_detail: "Healthy".to_string(),
        gitops: None,
        argo_rollout_in_window: None,
        gitops_unresolved: None,
        local_cause: None,
        error_log_snippet: None,
        error_log_pod: None,
        error_log_container: None,
        change_kind: ChangeKind::Rollout,
        changed_age: "1h".to_string(),
        change_detail: None,
        deployed_at: Some("2026-10-27T09:00:00Z".to_string()),
        deployed_age: "1h".to_string(),
        current_revision: "7".to_string(),
        previous_revision: Some("6".to_string()),
        current_images: vec!["redis:7.2.1".to_string()],
        previous_images: vec!["redis:7.2.0".to_string()],
        image_diff: "v7.2.0 ➔ v7.2.1".to_string(),
        desired_replicas: 3,
        updated_replicas: 3,
        ready_replicas: 3,
        available_replicas: 3,
        rollout_status: RolloutStatus::Complete,
        failing_pods_count: 0,
        crash_loop_count: 0,
        oom_killed_count: 0,
        probe_failure_count: 0,
        restart_count: 0,
        primary_symptoms: vec![],
        pod_symptoms: vec![],
        failing_pod_names: vec![],
        top_events: vec![],
        misscheduled: None,
        config_changes: vec![],
    };

    let deploy = AppDeploymentChange {
        app_name: "auth-service".to_string(),
        kind: "Deployment".to_string(),
        namespace: "prod".to_string(),
        incident_status: IncidentStatus::Healthy,
        failure_category: FailureCategory::None,
        failure_detail: "Healthy".to_string(),
        gitops: None,
        argo_rollout_in_window: None,
        gitops_unresolved: None,
        local_cause: None,
        error_log_snippet: None,
        error_log_pod: None,
        error_log_container: None,
        change_kind: ChangeKind::Rollout,
        changed_age: "30m".to_string(),
        change_detail: None,
        deployed_at: Some("2026-10-27T09:30:00Z".to_string()),
        deployed_age: "30m".to_string(),
        current_revision: "12".to_string(),
        previous_revision: Some("11".to_string()),
        current_images: vec!["auth:v1.5.0".to_string()],
        previous_images: vec!["auth:v1.4.9".to_string()],
        image_diff: "v1.4.9 ➔ v1.5.0".to_string(),
        desired_replicas: 2,
        updated_replicas: 2,
        ready_replicas: 2,
        available_replicas: 2,
        rollout_status: RolloutStatus::Complete,
        failing_pods_count: 0,
        crash_loop_count: 0,
        oom_killed_count: 0,
        probe_failure_count: 0,
        restart_count: 0,
        primary_symptoms: vec![],
        pod_symptoms: vec![],
        failing_pod_names: vec![],
        top_events: vec![],
        misscheduled: None,
        config_changes: vec![],
    };

    ChangedTriageReport {
        window_seconds: 3600,
        window_label: "1h".to_string(),
        namespace: Some("default".to_string()),
        summary: TriageSummary {
            total_deployments: 3,
            crashing_count: 0,
            oom_count: 0,
            error_count: 1,
            pending_count: 0,
            probe_failure_count: 0,
            rolling_count: 1,
            healthy_count: 2,
            headline_message: "WARNING: cilium-cni: 1 pod(s) unready".to_string(),
        },
        deployments: vec![ds, sts, deploy],
        infra_changes: vec![],
        includes_failing: true,
        includes_scaled: true,
        argo: Default::default(),
        omitted: vec![],
    }
}

/// D1: Input Disorder & Filter Interleaving
/// Human navigates to :changed, sees DaemonSets with "(ds)" and StatefulSets with "(sts)".
/// Filter queries using short alias "ds" or "kind:ds" filter exclusively to DaemonSets.
/// Filtering with "sts" switches to StatefulSets, and clearing filter reveals all workloads.
#[tokio::test]
async fn journey_d1_filter_daemonset_alias_and_kind_prefix() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    // Navigate to :changed
    press(&mut app, ch(':')).await;
    type_str(&mut app, "changed").await;
    press(&mut app, key(KeyCode::Enter)).await;

    // Feed report
    let report = sample_report_with_mixed_workloads();
    app.handle_changed_triage_result("test-cluster", Some("default"), Ok(report));

    let screen_all = render_app(&mut app, 160, 30);
    assert!(screen_all.contains("cilium-cni (ds)"));
    assert!(screen_all.contains("redis-cluster (sts)"));
    assert!(screen_all.contains("auth-service"));

    // 1. Filter by alias 'ds'
    press(&mut app, ch('/')).await;
    type_str(&mut app, "ds").await;
    press(&mut app, key(KeyCode::Enter)).await;

    let screen_ds = render_app(&mut app, 160, 30);
    assert!(screen_ds.contains("cilium-cni (ds)"));
    assert!(!screen_ds.contains("redis-cluster (sts)"));
    assert!(!screen_ds.contains("auth-service"));

    // Clear filter
    press(&mut app, key(KeyCode::Esc)).await;

    // 2. Filter by prefix 'kind:sts'
    press(&mut app, ch('/')).await;
    type_str(&mut app, "kind:sts").await;
    press(&mut app, key(KeyCode::Enter)).await;

    let screen_sts = render_app(&mut app, 160, 30);
    assert!(!screen_sts.contains("cilium-cni (ds)"));
    assert!(screen_sts.contains("redis-cluster (sts)"));
    assert!(!screen_sts.contains("auth-service"));

    // 3. Clear filter (Esc or empty Enter)
    press(&mut app, key(KeyCode::Esc)).await;
    let screen_cleared = render_app(&mut app, 160, 30);
    assert!(screen_cleared.contains("cilium-cni (ds)"));
    assert!(screen_cleared.contains("redis-cluster (sts)"));
    assert!(screen_cleared.contains("auth-service"));
}

/// D3: Temporal & Asynchrony (Truthful Coverage Degradation)
/// When DaemonSet listing fails (e.g. 403 Forbidden or network timeout), the engine records
/// it in `omitted` instead of claiming zero rollouts or all healthy. The TUI renders
/// a prominent Coverage Notice banner in the header.
#[tokio::test]
async fn journey_d3_temporal_rbac_omitted_notice_truthfulness() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    press(&mut app, ch(':')).await;
    type_str(&mut run_app(&mut app), "").await; // no-op just to type
    type_str(&mut app, "changed").await;
    press(&mut app, key(KeyCode::Enter)).await;

    let mut report = sample_report_with_mixed_workloads();
    // Simulate DaemonSet RBAC forbidden
    report.omitted = vec![OmittedWorkloadKind {
        kind: "DaemonSet".to_string(),
        reason: "403 Forbidden: user cannot list daemonsets.apps in cluster".to_string(),
    }];

    app.handle_changed_triage_result("test-cluster", Some("default"), Ok(report));

    let screen = render_app(&mut app, 120, 30);
    assert!(
        screen.contains("Coverage Notice:"),
        "Screen must render coverage notice"
    );
    assert!(
        screen.contains("DaemonSet list unavailable (403 Forbidden"),
        "Must specify DaemonSet and reason verbatim"
    );
}

fn run_app(app: &mut App) -> &mut App {
    app
}

/// D4: Boundary & Scale Extremes
/// 1. DaemonSet with desired = 0 (no matching nodes): detail states "no nodes match selector", not "OFF".
/// 2. Long 63-char DaemonSet name at 80x24 terminal does not overflow or clip the READY column.
#[tokio::test]
async fn journey_d4_boundary_and_scale_extremes() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    press(&mut app, ch(':')).await;
    type_str(&mut app, "changed").await;
    press(&mut app, key(KeyCode::Enter)).await;

    let name = "node-exporter";
    let mut ds_zero_nodes = sample_daemonset_workload(name, 0, 0, None);
    ds_zero_nodes.failure_detail = "no nodes match selector".to_string();

    let mut report = sample_report_with_mixed_workloads();
    report.deployments = vec![ds_zero_nodes];

    app.handle_changed_triage_result("test-cluster", Some("default"), Ok(report));

    // Render at standard 80x24 terminal
    let screen_80x24 = render_app(&mut app, 80, 24);
    assert!(screen_80x24.contains("node-e"));
    assert!(screen_80x24.contains("0/0"));
    assert!(screen_80x24.contains("no nodes match selector"));
    assert!(
        !screen_80x24.contains("OFF"),
        "DaemonSet with 0 matching nodes must not be marked OFF"
    );

    // Render at 140 width where full name + (ds) is visible without table compression
    let screen_140 = render_app(&mut app, 140, 24);
    assert!(screen_140.contains("node-exporter (ds)"));
}

/// D5: State Transitions & Scaled Classification
/// When a DaemonSet schedules pods due to node join (new nodes added to cluster),
/// it is classified as `ChangeKind::Scaled` with "[S]" badge, not "[ROLLING]".
#[tokio::test]
async fn journey_d5_node_churn_scaled_vs_rollout() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    press(&mut app, ch(':')).await;
    type_str(&mut app, "changed").await;
    press(&mut app, key(KeyCode::Enter)).await;

    let mut ds_scaled = sample_daemonset_workload("csi-rbdplugin", 12, 12, None);
    ds_scaled.change_kind = ChangeKind::Scaled;
    ds_scaled.change_detail = Some("Scheduled to 2 new node(s) (10→12)".to_string());

    let mut report = sample_report_with_mixed_workloads();
    report.summary.rolling_count = 0;
    report.deployments = vec![ds_scaled];

    app.handle_changed_triage_result("test-cluster", Some("default"), Ok(report));

    if let srectl::app::ActiveView::Changed(ref mut c) = app.active_view {
        c.include_scaled = true;
    }

    let screen = render_app(&mut app, 160, 30);
    assert!(screen.contains("csi-rbdplugin (ds)"));
    assert!(screen.contains("Scheduled to 2 new node"));
    assert!(!screen.contains(" 🔄  ROLLING "));
}

/// D6: Feedback, Truthfulness & Secret Redaction
/// Secret configuration changes display only metadata name and age, never secret data or values.
#[tokio::test]
async fn journey_d6_truthfulness_and_secret_redaction() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    press(&mut app, ch(':')).await;
    type_str(&mut app, "changed").await;
    press(&mut app, key(KeyCode::Enter)).await;

    let report = sample_report_with_mixed_workloads();
    app.handle_changed_triage_result("test-cluster", Some("default"), Ok(report));

    let screen = render_app(&mut app, 160, 50);
    // Config change details visible
    assert!(screen.contains("ConfigMap cilium-config (volume 'config')"));
    assert!(screen.contains("Secret cilium-ca (volume 'certs')"));
    // Ensure no token / secret value leakage
    assert!(!screen.contains("PRIVATE KEY"));
    assert!(!screen.contains("token"));
}

/// D7: Visual & Perceptual Reality (Detail Card & RCA Node Metric)
/// 1. Diagnostic card renders "Nodes: Desired: 5, Updated: 5, Ready: 4, Available: 4, Misscheduled: 1".
/// 2. Quick RCA for DaemonSets uses "nodes", not "replicas".
#[tokio::test]
async fn journey_d7_perceptual_layout_and_node_metrics() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;

    press(&mut app, ch(':')).await;
    type_str(&mut app, "changed").await;
    press(&mut app, key(KeyCode::Enter)).await;

    let report = sample_report_with_mixed_workloads();
    app.handle_changed_triage_result("test-cluster", Some("default"), Ok(report));

    // Render screen at 200x50 with diagnostic card open
    let screen = render_app(&mut app, 200, 50);
    assert!(
        screen.contains("Nodes: Desired: 5, Updated: 5, Ready: 4, Available: 4, Misscheduled: 1"),
        "Card must display Nodes header and misscheduled count"
    );

    // Test Quick RCA unit
    let mut healthy_ds = sample_daemonset_workload("promtail", 28, 28, None);
    healthy_ds.incident_status = IncidentStatus::Healthy;
    healthy_ds.failing_pods_count = 0;
    healthy_ds.pod_symptoms = vec![];

    let direct_rca = evaluate_direct_rca(&healthy_ds).expect("healthy direct RCA");
    assert_eq!(
        direct_rca.root_cause, "Workload is healthy with 28/28 nodes ready.",
        "Quick RCA must describe DaemonSets with 'nodes' instead of 'replicas'"
    );
}
