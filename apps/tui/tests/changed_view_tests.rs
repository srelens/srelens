//! Integration and unit tests for Changed & SRE Post-Page Incident Triage view.

mod common;

use ratatui::backend::TestBackend;
use ratatui::{Frame, Terminal};
use srelens_kube::changed::{
    AppDeploymentChange, ChangedTriageReport, FailureCategory, GitOpsReleaseInfo, IncidentStatus,
    InfraChangeItem, PodIncidentDetail, RolloutStatus, TriageSummary,
};
use srelens_kube::events::EventSummary;
use srelens_tui::commands::{resolve_command, CommandTarget, ResourceKind};
use srelens_tui::views::changed_view::{
    render_changed_view, ChangedTab, ChangedViewState, IncidentFilter, QuickRca, QuickRcaStatus,
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
            oom_count: 0,
            error_count: 0,
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
                    sync_age: "12m".to_string(),
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
                pod_symptoms: vec![PodIncidentDetail {
                    pod_name: "checkout-api-7b89-abcd".to_string(),
                    status: "CrashLoopBackOff".to_string(),
                    detail_message: "exited with code 1".to_string(),
                }],
                failing_pod_names: vec!["checkout-api-7b89-abcd".to_string()],
                argo_rollout_in_window: Some("rev 7b89abc synced 12m ago".to_string()),
                error_log_pod: Some("checkout-api-7b89-abcd".to_string()),
                error_log_container: Some("api".to_string()),
                change_kind: srelens_kube::changed::ChangeKind::Rollout,
                changed_age: String::new(),
                change_detail: None,
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
                pod_symptoms: vec![PodIncidentDetail {
                    pod_name: "payment-worker-9988-xyz".to_string(),
                    status: "Pending".to_string(),
                    detail_message: "Insufficient cpu".to_string(),
                }],
                failing_pod_names: vec!["payment-worker-9988-xyz".to_string()],
                argo_rollout_in_window: None,
                error_log_pod: None,
                error_log_container: None,
                change_kind: srelens_kube::changed::ChangeKind::Rollout,
                changed_age: String::new(),
                change_detail: None,
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
                    sync_age: "27m".to_string(),
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
                pod_symptoms: vec![],
                failing_pod_names: vec![],
                argo_rollout_in_window: None,
                error_log_pod: None,
                error_log_container: None,
                change_kind: srelens_kube::changed::ChangeKind::Rollout,
                changed_age: String::new(),
                change_detail: None,
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
        includes_failing: false,
        includes_scaled: false,
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
    assert_eq!(state.incident_filter, IncidentFilter::OomOnly);
    assert_eq!(state.filtered_deployments().len(), 0);

    state.cycle_filter();
    assert_eq!(state.incident_filter, IncidentFilter::ErrorOnly);
    assert_eq!(state.filtered_deployments().len(), 0);

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
    assert!(rendered.contains("CRASH: 1"));
    assert!(rendered.contains("OOM: 0"));
    assert!(rendered.contains("ERROR: 0"));
    assert!(rendered.contains("PENDING: 1"));
    assert!(rendered.contains("HEALTHY: 1"));
    assert!(!rendered.contains("Headline:"));

    // Table checks
    assert!(rendered.contains("checkout-api"));
    assert!(rendered.contains("payment-worker"));
    assert!(rendered.contains("frontend"));
    assert!(rendered.contains("1/3"));

    // Diagnostic Card checks for selected deployment ("checkout-api")
    assert!(rendered.contains("Incident Diagnostic & Root Cause Investigator"));
    assert!(rendered.contains("Workload: prod/checkout-api (Deployment)"));
    assert!(rendered.contains("GitOps Release: checkout-prod"));
    assert!(rendered.contains("7b89abc"));
    assert!(rendered.contains("Root Cause: [APP]"));
    assert!(rendered.contains("Symptoms (1 pod failing):"));
    assert!(rendered.contains("checkout-api-7b89-abcd: CrashLoopBackOff | exited with code 1"));
    assert!(!rendered.contains("Failing Pods:"));
    assert!(rendered.contains("ArgoCD Rollout: rev 7b89abc synced 12m ago"));
    assert!(rendered.contains("Synced: 12m ago"));
    assert!(!rendered.contains("ago ago"));
    // The logs are one key away (`l`), and the events are gone: the card
    // carries neither, however much of either there is.
    assert!(!rendered.contains("Error Log Snippet"));
    assert!(!rendered.contains("Failed to connect to Redis cache"));
    assert!(!rendered.contains("Correlated Events"));
    assert!(!rendered.contains("Back-off restarting failed container"));
    assert!(rendered.contains(
        "[Enter/d] Describe   [l] Logs   [y] YAML   [s] Quick AI RCA   [a] Assistant   [r] Refresh"
    ));
    assert!(rendered.contains("Scope: [CHANGED]"));
    assert!(!rendered.contains("[j/k] Navigate"));
    assert!(!rendered.contains("[r] Rollout Restart"));
    assert!(rendered.contains("[y] YAML"));
    assert!(
        !rendered.contains("YAML Diff"),
        "y opens the manifest, not a diff"
    );
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

    // When filter matches nothing, it tells the user the filter hid them
    state.filter_query = "nonexistent".to_string();
    let lines = render_lines(120, 28, |f| {
        render_changed_view(f, f.area(), &state);
    });
    let rendered = lines.join("\n");
    assert!(rendered.contains("No infrastructure changes matching query 'nonexistent'."));
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

fn render_card(state: &ChangedViewState) -> String {
    render_lines(160, 44, |f| render_changed_view(f, f.area(), state)).join("\n")
}

fn with_rca(status: QuickRcaStatus) -> ChangedViewState {
    let mut state = ChangedViewState::new();
    state.context = "prod-eu".to_string();
    state.set_report(sample_report());
    let d = state.selected_deployment().unwrap().clone();
    let key = state.rca_key(&d);
    state.ai_summaries.insert(
        key,
        QuickRca {
            pod_name: Some("checkout-api-7b89-abcd".to_string()),
            provider: "Anthropic (Claude)".to_string(),
            status,
            updated_at: std::time::Instant::now(),
        },
    );
    state
}

#[test]
fn card_advertises_quick_rca_and_the_assistant_separately() {
    let _settings = common::env::isolate_settings();
    let mut state = ChangedViewState::new();
    state.set_report(sample_report());
    let rendered = render_card(&state);
    assert!(rendered.contains("[s] Quick AI RCA"));
    assert!(rendered.contains("[a] Assistant"));
    assert!(
        !rendered.contains("Quick AI RCA ("),
        "no RCA section until asked for"
    );
}

#[test]
fn card_renders_quick_rca_loading_ready_and_error() {
    let _settings = common::env::isolate_settings();

    let loading = render_card(&with_rca(QuickRcaStatus::Loading));
    assert!(loading.contains("Quick AI RCA (checkout-api-7b89-abcd, Anthropic (Claude)):"));
    assert!(loading.contains("Analyzing termination state, events, and error logs"));

    let ready = render_card(&with_rca(QuickRcaStatus::Ready {
        root_cause: "Redis at redis-master.prod:6379 refuses connections.".to_string(),
        action_item: "Check the redis-master pods, or roll back 7b89abc.".to_string(),
    }));
    assert!(ready.contains("[cached 0s ago]"));
    assert!(ready.contains("Root Cause: Redis at redis-master.prod:6379 refuses connections."));
    assert!(ready.contains("Action Item: Check the redis-master pods, or roll back 7b89abc."));
    let rca_at = ready.find("Quick AI RCA (").unwrap();
    assert!(
        ready.find("Symptoms (").unwrap() < rca_at,
        "after the symptoms"
    );
    assert!(
        rca_at < ready.find("Actions:").unwrap(),
        "before the actions"
    );

    let error = render_card(&with_rca(QuickRcaStatus::Error(
        "No API key configured for Anthropic (Claude). Add one in :ai-settings".to_string(),
    )));
    assert!(error.contains("No API key configured for Anthropic (Claude). Add one in :ai-settings"));
    assert!(
        !error.contains("Ctrl+s"),
        "Ctrl+s means Scale outside the Assistant"
    );
}

#[test]
fn quick_rca_is_keyed_by_cluster_and_revision() {
    let _settings = common::env::isolate_settings();
    let state = with_rca(QuickRcaStatus::Ready {
        root_cause: "old release".to_string(),
        action_item: String::new(),
    });
    let d = state.selected_deployment().unwrap().clone();
    assert!(state.rca_for(&d).is_some());

    // The same workload rolled forward: the old answer is not shown as its.
    let mut rolled = d.clone();
    rolled.current_revision = "6".to_string();
    assert!(state.rca_for(&rolled).is_none());

    // The same workload name on another regional cluster.
    let mut other = with_rca(QuickRcaStatus::Loading);
    other.context = "prod-us".to_string();
    assert!(other.rca_for(&d).is_none());

    // And an unformatted answer renders without an empty Action Item line.
    let rendered = render_card(&state);
    assert!(rendered.contains("Root Cause: old release"));
    assert!(!rendered.contains("Action Item:"));
}

fn crash_pod(name: &str) -> PodIncidentDetail {
    PodIncidentDetail {
        pod_name: name.to_string(),
        status: "CrashLoopBackOff".to_string(),
        detail_message: "exited with code 1".to_string(),
    }
}

#[test]
fn group_pod_symptoms_collapses_identical_pods() {
    use srelens_tui::views::changed_view::group_pod_symptoms;
    let pods: Vec<_> = (0..150).map(|i| crash_pod(&format!("api-{i}"))).collect();
    let groups = group_pod_symptoms(&pods);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].count, 150);
    assert_eq!(
        groups[0].describe(),
        "CrashLoopBackOff | exited with code 1 — 150 pods (api-0, api-1, +148 more)"
    );

    // A lone pod reads as itself; first-seen order is kept.
    let mixed = vec![
        crash_pod("a"),
        PodIncidentDetail {
            pod_name: "b".to_string(),
            status: "Pending".to_string(),
            detail_message: "Insufficient cpu".to_string(),
        },
        crash_pod("c"),
    ];
    let groups = group_pod_symptoms(&mixed);
    let lines: Vec<String> = groups.iter().map(|g| g.describe()).collect();
    assert_eq!(
        lines,
        [
            "CrashLoopBackOff | exited with code 1 — 2 pods (a, c)",
            "b: Pending | Insufficient cpu",
        ]
    );
}

#[test]
fn card_shows_at_most_three_symptom_groups_however_many_pods_fail() {
    let _settings = common::env::isolate_settings();
    let mut report = sample_report();
    let d = &mut report.deployments[0];
    d.pod_symptoms = (0..150)
        .map(|i| crash_pod(&format!("checkout-api-{i}")))
        .collect();
    for (i, status) in ["OOMKilled", "Error", "Pending", "ImagePullBackOff"]
        .iter()
        .enumerate()
    {
        d.pod_symptoms.push(PodIncidentDetail {
            pod_name: format!("odd-{i}"),
            status: status.to_string(),
            detail_message: String::new(),
        });
    }
    let mut state = ChangedViewState::new();
    state.set_report(report);

    let rendered = render_card(&state);

    assert!(rendered.contains("Symptoms (154 pods failing):"));
    assert!(rendered.contains("150 pods (checkout-api-0, checkout-api-1, +148 more)"));
    assert!(rendered.contains("odd-0: OOMKilled"));
    assert!(rendered.contains("odd-1: Error"));
    assert!(!rendered.contains("odd-2"), "the fourth group is not drawn");
    assert!(rendered.contains("+2 other symptoms"));
    assert!(
        !rendered.contains("checkout-api-7,"),
        "individual pods are not listed"
    );
}

#[test]
fn an_unchanged_row_says_why_it_is_shown() {
    let _settings = common::env::isolate_settings();
    let mut report = sample_report();
    report.includes_failing = true;
    report.deployments[0].change_kind = srelens_kube::changed::ChangeKind::FailingOnly;
    let mut state = ChangedViewState::new();
    state.include_failing = true;
    state.set_report(report);

    let rendered = render_card(&state);

    assert!(
        rendered.contains("checkout-api (unchanged)"),
        "a word, not only colour"
    );
    assert!(rendered
        .contains("Not changed in the last 1h; shown because it is failing now (u to hide)."));
    assert!(rendered.contains("Scope: [CHANGED + FAILING]"));
    assert!(rendered.contains("[u] Hide unchanged"));
    // Rows that did change carry no marker.
    assert!(!rendered.contains("payment-worker (unchanged)"));
}

fn footer_line(width: u16, height: u16, state: &ChangedViewState) -> String {
    let lines = render_lines(width, height, |f| render_changed_view(f, f.area(), state));
    lines.last().unwrap().trim_end().to_string()
}

#[test]
fn footer_leaves_the_cards_keys_to_the_card() {
    let _settings = common::env::isolate_settings();
    let mut state = ChangedViewState::new();
    state.set_report(sample_report());

    // Tall enough for the card: only the view-wide keys.
    let footer = footer_line(200, 44, &state);
    assert_eq!(
        footer,
        "[[/]] Window (1h)  [f] Filter  [u] Include failing  [S] Include scaled  [Tab] Toggle Infra  [/] Search"
    );

    // Too short for a card: its keys move to the footer.
    let short = footer_line(200, 24, &state);
    assert!(
        short.starts_with(
            "[Enter] Describe  [y] YAML  [l] Logs  [s] Quick RCA  [a] Assistant  [r] Refresh"
        ),
        "{short}"
    );

    // Infra tab: no card and no workload keys, but describe/yaml/refresh.
    state.toggle_tab();
    let infra = footer_line(200, 44, &state);
    assert!(
        infra.starts_with("[Enter] Describe  [y] YAML  [r] Refresh  [[/]] Window"),
        "{infra}"
    );
    assert!(!infra.contains("Quick RCA"));
}

#[test]
fn an_empty_list_says_why_it_is_empty() {
    let _settings = common::env::isolate_settings();
    let mut report = sample_report();
    report.deployments.clear();
    let mut state = ChangedViewState::new();
    state.set_report(report.clone());
    let strict = render_card(&state);
    // The message wraps; judge it on one line.
    let strict_flat = strict
        .replace('│', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        strict_flat.contains("No workloads changed within the last 1h."),
        "{strict_flat}"
    );
    assert!(
        strict_flat.contains("S to include scaled workloads"),
        "{strict_flat}"
    );
    assert!(strict_flat.contains("u to include workloads failing without a change"));

    state.include_failing = true;
    let wide = render_card(&state);
    assert!(wide.contains("Workloads Changed or Failing in Window"));
    assert!(wide.contains("No workloads changed or failing within the last 1h."));

    // Rows exist, but the incident filter hides them: say so, not "none".
    let mut state = ChangedViewState::new();
    state.set_report(sample_report());
    for _ in 0..2 {
        state.cycle_filter(); // ALL -> CRASH -> OOM
    }
    let filtered = render_card(&state);
    assert!(filtered.contains("No workloads in the window match the OOM filter."));
    assert!(!filtered.contains("No workloads changed"));
}

#[test]
fn a_scaled_row_shows_when_it_scaled_and_says_so() {
    let _settings = common::env::isolate_settings();
    use srelens_kube::changed::ChangeKind;
    let mut report = sample_report();
    report.includes_scaled = true;
    let d = &mut report.deployments[0];
    d.change_kind = ChangeKind::Scaled;
    d.changed_age = "5m".to_string();
    d.deployed_age = "66d".to_string();
    d.change_detail = Some("Scaled 3→4".to_string());
    let mut state = ChangedViewState::new();
    state.include_scaled = true;
    state.set_report(report);

    let rendered = render_card(&state);

    assert!(rendered.contains("checkout-api (scaled)"), "{rendered}");
    let row = rendered
        .lines()
        .find(|l| l.contains("checkout-api (scaled)"))
        .unwrap();
    assert!(
        row.trim_end()
            .trim_end_matches('│')
            .trim_end()
            .ends_with("5m"),
        "CHANGED is the scale time: {row}"
    );
    assert!(rendered.contains("CHANGED"), "column header");
    assert!(rendered.contains("Scaled 3→4 5m ago; last rollout 66d ago (S to hide)."));
    assert!(rendered.contains("Scope: [CHANGED + SCALED]"));
    assert!(rendered.contains("[S] Hide scaled"));
}

#[test]
fn scope_label_names_every_combination() {
    let mut state = ChangedViewState::new();
    assert_eq!(state.scope_label(), "[CHANGED]");
    state.include_scaled = true;
    assert_eq!(state.scope_label(), "[CHANGED + SCALED]");
    state.include_failing = true;
    assert_eq!(state.scope_label(), "[CHANGED + SCALED + FAILING]");
    state.include_scaled = false;
    assert_eq!(state.scope_label(), "[CHANGED + FAILING]");
}

/// The card's rows, from its "Workload:" line to "Actions:", border-trimmed.
fn card_rows(rendered: &str) -> Vec<String> {
    let rows: Vec<String> = rendered
        .lines()
        .skip_while(|l| !l.contains("Workload: "))
        .map(|l| l.trim_matches(|c| c == '│' || c == ' ').to_string())
        .collect();
    let end = rows.iter().position(|l| l.starts_with("Actions:")).unwrap();
    rows[..=end].to_vec()
}

#[test]
fn card_sections_are_spaced_by_exactly_one_blank_line() {
    let _settings = common::env::isolate_settings();
    // A healthy row with an RCA: no symptoms block between the two.
    let mut report = sample_report();
    report.deployments.swap(0, 2); // frontend (healthy, no symptoms) first
    let mut state = ChangedViewState::new();
    state.context = "prod-eu".to_string();
    state.set_report(report);
    let d = state.selected_deployment().unwrap().clone();
    let key = state.rca_key(&d);
    state.ai_summaries.insert(
        key,
        QuickRca {
            pod_name: None,
            provider: "Anthropic (Claude)".to_string(),
            status: QuickRcaStatus::Ready {
                root_cause: "Nothing is failing.".to_string(),
                action_item: "No action needed.".to_string(),
            },
            updated_at: std::time::Instant::now(),
        },
    );

    let rows = card_rows(&render_card(&state));

    let at = |needle: &str| {
        rows.iter()
            .position(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("{needle}: {rows:#?}"))
    };
    let root = at("Root Cause: [OK]");
    let header = at("Quick AI RCA (");
    let first_bullet = at("• Root Cause: Nothing is failing.");
    let actions = at("Actions:");
    // Workload block, blank, Root Cause, blank, RCA header, blank, RCA, blank, Actions.
    assert!(
        rows[root - 1].is_empty() && !rows[root - 2].is_empty(),
        "{rows:#?}"
    );
    assert_eq!(
        header,
        root + 2,
        "one blank between Root Cause and the RCA header: {rows:#?}"
    );
    assert_eq!(
        first_bullet,
        header + 2,
        "one blank between the header and its body: {rows:#?}"
    );
    assert!(
        rows[actions - 1].is_empty() && !rows[actions - 2].is_empty(),
        "{rows:#?}"
    );
    // Never two blanks in a row.
    assert!(
        rows.windows(2)
            .all(|w| !(w[0].is_empty() && w[1].is_empty())),
        "{rows:#?}"
    );
}

#[test]
fn table_columns_fit_their_longest_namespace_and_workload() {
    let _settings = common::env::isolate_settings();
    let mut report = sample_report();
    // Longer than the old fixed 14-column NAMESPACE and 44-column WORKLOAD.
    report.deployments[0].namespace = "external-secrets-operator".to_string();
    report.deployments[1].app_name = "wiz-package-wiz-admission-controller-manager".to_string();
    report.deployments[1].kind = "CronJob".to_string();
    let mut state = ChangedViewState::new();
    state.set_report(report);

    let rendered = render_lines(220, 44, |f| render_changed_view(f, f.area(), &state)).join("\n");

    assert!(rendered.contains("external-secrets-operator"), "{rendered}");
    assert!(
        rendered.contains("wiz-package-wiz-admission-controller-manager (cj)"),
        "{rendered}"
    );
}

#[test]
fn clamp_selection_clamps_both_deployments_and_infra() {
    let _settings = common::env::isolate_settings();
    let mut state = ChangedViewState::new();
    let mut report = sample_report();
    report.infra_changes = vec![
        InfraChangeItem {
            age: "5m".to_string(),
            last_ts: Some("2026-09-25T12:00:00Z".to_string()),
            kind: "Node".to_string(),
            name: "node-1".to_string(),
            namespace: "".to_string(),
            reason: "NodeNotReady".to_string(),
            message: "Kubelet stopped posting node status.".to_string(),
            count: 1,
            is_warning: true,
        },
        InfraChangeItem {
            age: "10m".to_string(),
            last_ts: Some("2026-09-25T11:55:00Z".to_string()),
            kind: "Node".to_string(),
            name: "node-2".to_string(),
            namespace: "".to_string(),
            reason: "NodeReady".to_string(),
            message: "Node is ready.".to_string(),
            count: 1,
            is_warning: false,
        },
    ];
    state.set_report(report);
    state.selected_idx = 50;
    state.infra_selected_idx = 50;
    state.clamp_selection();

    assert_eq!(state.selected_idx, 2);
    assert_eq!(state.infra_selected_idx, 1);

    state.filter_query = "node-1".to_string();
    state.clamp_selection();
    assert_eq!(state.infra_selected_idx, 0);

    state.filter_query = "non-existent-filter-query".to_string();
    state.clamp_selection();
    assert_eq!(state.selected_idx, 0);
    assert_eq!(state.infra_selected_idx, 0);
}

#[test]
fn narrow_terminal_allocates_multiline_footer() {
    let _settings = common::env::isolate_settings();
    let mut state = ChangedViewState::new();
    state.set_report(sample_report());

    // On narrow terminal (80 columns), footer has more than 80 chars of shortcuts,
    // so it wraps onto two rows.
    let rendered = render_lines(80, 24, |f| render_changed_view(f, f.area(), &state));
    let last_three = &rendered[rendered.len() - 3..];
    let footer_text = last_three.join(" ");
    assert!(footer_text.contains("Filter"), "{footer_text}");
    assert!(footer_text.contains("Search"), "{footer_text}");
}
