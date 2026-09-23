//! Integration and unit tests for Changed & SRE Post-Page Incident Triage view.

mod common;

use ratatui::backend::TestBackend;
use ratatui::{Frame, Terminal};
use srelens_kube::changed::{
    AppDeploymentChange, ChangedTriageReport, FailureCategory, GitOpsReleaseInfo, IncidentStatus,
    InfraChangeItem, RolloutStatus, TriageSummary,
};
use srelens_kube::events::EventSummary;
use srelens_tui::commands::{resolve_command, CommandTarget, ResourceKind};
use srelens_tui::views::changed_view::{
    render_changed_view, ChangedTab, ChangedViewState, IncidentFilter,
};

fn render_lines<F>(width: u16, height: u16, draw: F) -> Vec<String>
where
    F: FnOnce(&mut Frame),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(draw).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn sample_report() -> ChangedTriageReport {
    ChangedTriageReport {
        window_seconds: 3600,
        window_label: "1h".to_string(),
        namespace: None,
        summary: TriageSummary {
            total_deployments: 3,
            crashing_count: 1,
            pending_count: 1,
            rolling_count: 0,
            healthy_count: 1,
            headline_message: "CRITICAL: checkout-api: 1 pod(s) in CrashLoopBackOff".to_string(),
        },
        deployments: vec![
            AppDeploymentChange {
                app_name: "checkout-api".to_string(),
                kind: "Deployment".to_string(),
                namespace: "prod".to_string(),
                incident_status: IncidentStatus::CrashLoop,
                failure_category: FailureCategory::App,
                failure_detail: "checkout-api-7b89-abcd: exited with code 1".to_string(),
                gitops: Some(GitOpsReleaseInfo {
                    app_name: "checkout-prod".to_string(),
                    sync_status: "Synced".to_string(),
                    health_status: "Degraded".to_string(),
                    repo_url: "https://github.com/org/checkout.git".to_string(),
                    target_revision: "main".to_string(),
                    sync_revision: "7b89abc".to_string(),
                    sync_age: "12m ago".to_string(),
                    sync_message: None,
                }),
                error_log_snippet: Some(vec![
                    "2026-09-23T10:00:01Z [ERROR] Failed to connect to Redis cache: connection refused"
                        .to_string(),
                    "2026-09-23T10:00:01Z [FATAL] panic: initialization failed".to_string(),
                ]),
                deployed_at: Some("2026-09-23T10:00:00Z".to_string()),
                deployed_age: "12m".to_string(),
                current_revision: "5".to_string(),
                previous_revision: Some("4".to_string()),
                current_images: vec!["checkout:v2.1.0".to_string()],
                previous_images: vec!["checkout:v2.0.0".to_string()],
                image_diff: "v2.0.0 ➔ v2.1.0".to_string(),
                desired_replicas: 3,
                updated_replicas: 3,
                ready_replicas: 1,
                available_replicas: 1,
                rollout_status: RolloutStatus::Failed,
                failing_pods_count: 1,
                crash_loop_count: 1,
                oom_killed_count: 0,
                probe_failure_count: 0,
                restart_count: 4,
                primary_symptoms: vec!["checkout-api-7b89-abcd: CrashLoopBackOff".to_string()],
                failing_pod_names: vec!["checkout-api-7b89-abcd".to_string()],
                top_events: vec![EventSummary {
                    name: "checkout-api-7b89-abcd.ev1".to_string(),
                    namespace: "prod".to_string(),
                    type_: "Warning".to_string(),
                    reason: "BackOff".to_string(),
                    object: "Pod/checkout-api-7b89-abcd".to_string(),
                    first_age: "10m".to_string(),
                    first_created: None,
                    object_api_version: "v1".to_string(),
                    source: "kubelet".to_string(),
                    message: "Back-off restarting failed container".to_string(),
                    created: None,
                    age: "2m".to_string(),
                    created_at: "".to_string(),
                    count: 4,
                }],
            },
            AppDeploymentChange {
                app_name: "payment-worker".to_string(),
                kind: "Deployment".to_string(),
                namespace: "prod".to_string(),
                incident_status: IncidentStatus::Pending,
                failure_category: FailureCategory::Compute,
                failure_detail: "0/3 nodes available: 3 Insufficient cpu".to_string(),
                gitops: None,
                error_log_snippet: None,
                deployed_at: Some("2026-09-23T10:05:00Z".to_string()),
                deployed_age: "7m".to_string(),
                current_revision: "2".to_string(),
                previous_revision: Some("1".to_string()),
                current_images: vec!["worker:v1.1.0".to_string()],
                previous_images: vec!["worker:v1.0.0".to_string()],
                image_diff: "v1.0.0 ➔ v1.1.0".to_string(),
                desired_replicas: 3,
                updated_replicas: 3,
                ready_replicas: 0,
                available_replicas: 0,
                rollout_status: RolloutStatus::Progressing,
                failing_pods_count: 1,
                crash_loop_count: 0,
                oom_killed_count: 0,
                probe_failure_count: 0,
                restart_count: 0,
                primary_symptoms: vec!["Pod unschedulable: Insufficient cpu".to_string()],
                failing_pod_names: vec!["payment-worker-9988-xyz".to_string()],
                top_events: vec![],
            },
            AppDeploymentChange {
                app_name: "frontend".to_string(),
                kind: "Deployment".to_string(),
                namespace: "prod".to_string(),
                incident_status: IncidentStatus::Healthy,
                failure_category: FailureCategory::None,
                failure_detail: "Healthy".to_string(),
                gitops: Some(GitOpsReleaseInfo {
                    app_name: "frontend-prod".to_string(),
                    sync_status: "Synced".to_string(),
                    health_status: "Healthy".to_string(),
                    repo_url: "https://github.com/org/frontend.git".to_string(),
                    target_revision: "main".to_string(),
                    sync_revision: "a1b2c3d".to_string(),
                    sync_age: "27m ago".to_string(),
                    sync_message: None,
                }),
                error_log_snippet: None,
                deployed_at: Some("2026-09-23T09:45:00Z".to_string()),
                deployed_age: "27m".to_string(),
                current_revision: "10".to_string(),
                previous_revision: Some("9".to_string()),
                current_images: vec!["web:v3.0.0".to_string()],
                previous_images: vec!["web:v2.9.0".to_string()],
                image_diff: "v2.9.0 ➔ v3.0.0".to_string(),
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
                failing_pod_names: vec![],
                top_events: vec![],
            },
        ],
        infra_changes: vec![
            InfraChangeItem {
                age: "5m".to_string(),
                last_ts: None,
                kind: "ConfigMap".to_string(),
                name: "app-config".to_string(),
                namespace: "prod".to_string(),
                reason: "Updated".to_string(),
                message: "Configuration values updated".to_string(),
                count: 1,
                is_warning: false,
            },
            InfraChangeItem {
                age: "3m".to_string(),
                last_ts: None,
                kind: "Ingress".to_string(),
                name: "api-ingress".to_string(),
                namespace: "prod".to_string(),
                reason: "SyncFailed".to_string(),
                message: "Backend TLS certificate expired".to_string(),
                count: 3,
                is_warning: true,
            },
        ],
    }
}

#[test]
fn changed_commands_resolve_to_changed_view() {
    let _settings = common::env::isolate_settings();

    assert_eq!(
        resolve_command(":changed"),
        Some(CommandTarget::Resource(ResourceKind::Changed))
    );
    assert_eq!(
        resolve_command("changed"),
        Some(CommandTarget::Resource(ResourceKind::Changed))
    );
    assert_eq!(
        resolve_command(":change"),
        Some(CommandTarget::Resource(ResourceKind::Changed))
    );
    assert_eq!(
        resolve_command(":chg"),
        Some(CommandTarget::Resource(ResourceKind::Changed))
    );
    assert_eq!(
        resolve_command(":recent"),
        Some(CommandTarget::Resource(ResourceKind::Changed))
    );
    assert_eq!(
        resolve_command(":triage"),
        Some(CommandTarget::Resource(ResourceKind::Changed))
    );
}

#[test]
fn changed_view_state_navigation_and_filters() {
    let _settings = common::env::isolate_settings();

    let mut state = ChangedViewState::new();
    assert_eq!(state.current_window_label(), "1h");
    assert_eq!(state.incident_filter, IncidentFilter::All);
    assert_eq!(state.active_tab, ChangedTab::Deployments);
    assert!(state.is_loading);

    state.set_report(sample_report());
    assert!(!state.is_loading);
    assert_eq!(state.filtered_deployments().len(), 3);
    assert_eq!(state.filtered_infra().len(), 2);

    // Navigation
    assert_eq!(state.selected_idx, 0);
    assert_eq!(
        state.selected_deployment().unwrap().app_name,
        "checkout-api"
    );
    state.select_next();
    assert_eq!(state.selected_idx, 1);
    assert_eq!(
        state.selected_deployment().unwrap().app_name,
        "payment-worker"
    );
    state.select_last();
    assert_eq!(state.selected_idx, 2);
    assert_eq!(state.selected_deployment().unwrap().app_name, "frontend");
    state.select_first();
    assert_eq!(state.selected_idx, 0);

    // Filter cycling
    state.cycle_filter();
    assert_eq!(state.incident_filter, IncidentFilter::CrashingOnly);
    assert_eq!(state.filtered_deployments().len(), 1);
    assert_eq!(state.filtered_deployments()[0].app_name, "checkout-api");

    state.cycle_filter();
    assert_eq!(state.incident_filter, IncidentFilter::PendingOnly);
    assert_eq!(state.filtered_deployments().len(), 1);
    assert_eq!(state.filtered_deployments()[0].app_name, "payment-worker");

    state.cycle_filter();
    assert_eq!(state.incident_filter, IncidentFilter::RollingOnly);
    assert_eq!(state.filtered_deployments().len(), 0);

    state.cycle_filter();
    assert_eq!(state.incident_filter, IncidentFilter::HealthyOnly);
    assert_eq!(state.filtered_deployments().len(), 1);
    assert_eq!(state.filtered_deployments()[0].app_name, "frontend");

    state.cycle_filter();
    assert_eq!(state.incident_filter, IncidentFilter::All);
    assert_eq!(state.filtered_deployments().len(), 3);

    // Time window cycling
    state.next_window();
    assert_eq!(state.current_window_label(), "3h");
    state.next_window();
    assert_eq!(state.current_window_label(), "24h");
    state.set_window_by_str("15m");
    assert_eq!(state.current_window_label(), "15m");

    // Tab toggle
    state.toggle_tab();
    assert_eq!(state.active_tab, ChangedTab::Infra);
    state.toggle_tab();
    assert_eq!(state.active_tab, ChangedTab::Deployments);
}

#[test]
fn renders_changed_view_wide_with_diagnostic_card() {
    let _settings = common::env::isolate_settings();

    let mut state = ChangedViewState::new();
    state.set_report(sample_report());

    let lines = render_lines(160, 36, |f| {
        render_changed_view(f, f.area(), &state);
    });

    let rendered = lines.join("\n");

    // Banner checks
    assert!(rendered.contains("POST-PAGE INCIDENT INVESTIGATOR"));
    assert!(rendered.contains("CRASH/OOM: 1"));
    assert!(rendered.contains("PENDING: 1"));
    assert!(rendered.contains("HEALTHY: 1"));

    // Table checks
    assert!(rendered.contains("checkout-api"));
    assert!(rendered.contains("payment-worker"));
    assert!(rendered.contains("frontend"));
    assert!(rendered.contains("1/3"));

    // Diagnostic Card checks for selected deployment ("checkout-api")
    assert!(rendered.contains("Incident Diagnostic & Root Cause Investigator"));
    assert!(rendered.contains("GitOps Release: checkout-prod"));
    assert!(rendered.contains("7b89abc"));
    assert!(rendered.contains("Root Cause: [APP]"));
    assert!(rendered.contains("Error Log Snippet (checkout-api-7b89-abcd)"));
    assert!(rendered.contains("Failed to connect to Redis cache"));
    assert!(rendered.contains("panic: initialization failed"));
    assert!(rendered.contains("Back-off restarting failed container"));
    assert!(rendered.contains("[l] Full Logs"));
}

#[test]
fn renders_changed_view_infra_tab() {
    let _settings = common::env::isolate_settings();

    let mut state = ChangedViewState::new();
    state.set_report(sample_report());
    state.active_tab = ChangedTab::Infra;

    let lines = render_lines(120, 28, |f| {
        render_changed_view(f, f.area(), &state);
    });

    let rendered = lines.join("\n");

    assert!(rendered.contains("Non-Deployment Cluster Changes & Warnings"));
    assert!(rendered.contains("app-config"));
    assert!(rendered.contains("api-ingress"));
    assert!(rendered.contains("Backend TLS certificate expired"));
}

#[test]
fn renders_loading_and_error_states() {
    let _settings = common::env::isolate_settings();

    // Loading state
    let state = ChangedViewState::new();
    let lines = render_lines(100, 24, |f| {
        render_changed_view(f, f.area(), &state);
    });
    let rendered = lines.join("\n");
    assert!(
        rendered.contains("SRE Incident Investigation")
            || rendered.contains("Analyzing deployments")
    );

    // Error state
    let mut err_state = ChangedViewState::new();
    err_state.set_error("Connection refused to API server".to_string());
    let err_lines = render_lines(100, 24, |f| {
        render_changed_view(f, f.area(), &err_state);
    });
    let err_rendered = err_lines.join("\n");
    assert!(err_rendered.contains("Connection refused to API server"));
}
