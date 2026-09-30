//! Integration and unit tests for Changed & SRE Post-Page Incident Triage view.

mod common;

use ratatui::backend::TestBackend;
use ratatui::{Frame, Terminal};
use srelens_kube::changed::{
    AppDeploymentChange, ArgoCoverage, ArgoCoverageState, ArgoRollout, ChangedTriageReport,
    FailureCategory, GitOpsReleaseInfo, IncidentStatus, InfraChangeItem, PodIncidentDetail,
    RolloutStatus, TriageSummary,
};
use srelens_kube::events::EventSummary;
use srelens_registry::github::{CausePull, RolloutCause};
use srelens_tui::commands::{resolve_command, CommandTarget, ResourceKind};
use srelens_tui::views::changed_view::{
    render_changed_view, wrap_message_text, CauseLookup, ChangedTab, ChangedViewState,
    IncidentFilter, QuickRca, QuickRcaStatus,
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
                    ..Default::default()
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
                gitops_unresolved: None,
                local_cause: None,
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
                gitops_unresolved: None,
                local_cause: None,
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
                    ..Default::default()
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
                gitops_unresolved: None,
                local_cause: None,
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
        argo: Default::default(),
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
    assert!(!rendered.contains("[j/k] Navigate"));
    assert!(!rendered.contains("[r] Rollout Restart"));
    assert!(rendered.contains("[y] YAML"));
    assert!(
        !rendered.contains("YAML Diff"),
        "y opens the manifest, not a diff"
    );
}

#[test]
fn wrap_message_text_splits_on_words_and_long_tokens() {
    // Normal sentence wrapping
    let text = "Error updating load balancer with new hosts in target pool";
    let wrapped = wrap_message_text(text, 25);
    assert!(wrapped.len() >= 2);
    assert_eq!(wrapped.join(" "), text);

    // Huge token that exceeds column width
    let huge_token = "gke-search-backend-p-amd64spot-cc-0-d-141c084a-gl6z";
    let wrapped_token = wrap_message_text(huge_token, 20);
    assert!(wrapped_token.len() >= 2);
    assert_eq!(wrapped_token.concat(), huge_token);

    // Empty and single words
    assert_eq!(wrap_message_text("", 30), vec![""]);
    assert_eq!(wrap_message_text("Ready", 30), vec!["Ready"]);
}

#[test]
fn renders_changed_view_infra_tab() {
    let _settings = common::env::isolate_settings();

    let mut report = sample_report();
    report.infra_changes.push(InfraChangeItem {
        age: "22s".to_string(),
        last_ts: None,
        kind: "Service".to_string(),
        name: "thanos-query-cluster-ingress".to_string(),
        namespace: "monitoring".to_string(),
        reason: "UpdateLoadBalancer".to_string(),
        message: "Error updating load balancer with new hosts [gke-search-backend-p-amd64spot-cc-0-d-141c084a-gl6z gke-search-backend-p-amd64spot-cc-0-d-141c084a-gl6z]".to_string(),
        count: 1,
        is_warning: true,
    });

    let mut state = ChangedViewState::new();
    state.set_report(report);
    state.active_tab = ChangedTab::Infra;

    let lines = render_lines(120, 28, |f| {
        render_changed_view(f, f.area(), &state);
    });

    let rendered = lines.join("\n");

    assert!(rendered.contains("Non-Deployment Cluster Changes & Warnings"));
    assert!(rendered.contains("app-config"));
    assert!(rendered.contains("api-ingress"));
    assert!(rendered.contains("Backend TLS certificate expired"));
    // Multi-line wrapped parts both rendered
    assert!(rendered.contains("Error updating load balancer"));
    assert!(rendered.contains("gke-search-backend-p-amd64spot"));

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
    // Rows that did change carry no marker.
    assert!(!rendered.contains("payment-worker (unchanged)"));

    // A healthy workload shown under include_failing due to historical warnings in window
    state.selected_idx = 2;
    state.report.as_mut().unwrap().deployments[2].change_kind =
        srelens_kube::changed::ChangeKind::FailingOnly;
    let rendered_healthy = render_card(&state);
    assert!(rendered_healthy.contains(
        "Not changed in the last 1h; shown because warning events occurred in this window (u to hide)."
    ));
}

fn footer_line(width: u16, height: u16, state: &ChangedViewState) -> String {
    let lines = render_lines(width, height, |f| render_changed_view(f, f.area(), state));
    lines
        .into_iter()
        .rev()
        .map(|l| l.trim_end().to_string())
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
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
        "[[/]] Window (1h)  [f] Filter  [Tab] Toggle Infra  [b] Hide guide  [/] Search"
    );

    // When guide banner is hidden, footer reflects "Show guide"
    state.show_guide_banner = false;
    let footer_hidden = footer_line(200, 44, &state);
    assert!(footer_hidden.contains("[b] Show guide"), "{footer_hidden}");
    state.show_guide_banner = true;

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

    state.include_failing = false;
    state.include_scaled = true;
    let scaled_only = render_card(&state);
    assert!(scaled_only.contains("No workloads changed or scaled within the last 1h."));

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
    state.set_report(report.clone());

    let rendered = render_lines(220, 44, |f| render_changed_view(f, f.area(), &state)).join("\n");

    assert!(rendered.contains("external-secrets-operator"), "{rendered}");
    assert!(
        rendered.contains("wiz-package-wiz-admission-controller-manager (cj)"),
        "{rendered}"
    );

    // Also verify StatefulSet format and column fitting
    report.deployments[1].kind = "StatefulSet".to_string();
    state.set_report(report);
    let rendered_sts =
        render_lines(220, 44, |f| render_changed_view(f, f.area(), &state)).join("\n");
    assert!(
        rendered_sts.contains("wiz-package-wiz-admission-controller-manager (sts)"),
        "{rendered_sts}"
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

const SYNCED: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
const PREVIOUS: &str = "9f8e7d6c5b4a39281706f5e4d3c2b1a098765432";

fn sync_rollout() -> ArgoRollout {
    ArgoRollout {
        history_id: 17,
        revision: SYNCED.to_string(),
        previous_revision: Some(PREVIOUS.to_string()),
        deployed_at: "2026-09-23T10:00:00Z".to_string(),
        initiated_by: Some("alice".to_string()),
        repo_url: "https://github.com/org/checkout.git".to_string(),
        path: "apps/checkout".to_string(),
        is_chart: false,
        approximate: false,
    }
}

/// The sample report with its first row (checkout-api) changed by `edit`.
fn why_state(edit: impl FnOnce(&mut AppDeploymentChange)) -> ChangedViewState {
    let mut report = sample_report();
    edit(&mut report.deployments[0]);
    let mut state = ChangedViewState::new();
    state.set_report(report);
    state
}

fn with_sync(d: &mut AppDeploymentChange) {
    let g = d.gitops.as_mut().unwrap();
    g.matched_by = "trackingId".to_string();
    g.rollout = Some(sync_rollout());
}

fn answer(state: &mut ChangedViewState, lookup: CauseLookup) {
    let ask = state.next_cause_ask().expect("a GitHub question");
    state.causes.insert(ask.key, lookup);
}

fn pr(number: u64, title: &str, user: &str, is_bot: bool) -> CausePull {
    CausePull {
        number,
        title: title.to_string(),
        user: user.to_string(),
        is_bot,
        ..Default::default()
    }
}

#[test]
fn the_card_says_which_sync_and_which_prs_caused_the_rollout() {
    let _settings = common::env::isolate_settings();
    let mut state = why_state(with_sync);
    answer(
        &mut state,
        CauseLookup::Ready(RolloutCause {
            revision: SYNCED.to_string(),
            previous_revision: Some(PREVIOUS.to_string()),
            path: "apps/checkout".to_string(),
            pulls: vec![
                pr(1842, "fix: bump checkout timeout", "bob", false),
                pr(1843, "chore(deps): bump checkout", "renovate[bot]", true),
            ],
            other_commits: 2,
            ..Default::default()
        }),
    );
    let rendered = render_card(&state);
    assert!(
        rendered.contains("Why: Argo sync #17 to a1b2c3d from 9f8e7d6, by alice"),
        "{rendered}"
    );
    assert!(rendered.contains("[via tracking id]"), "{rendered}");
    assert!(
        rendered.contains("PR #1842 \"fix: bump checkout timeout\" by bob"),
        "{rendered}"
    );
    assert!(rendered.contains("Bot PR #1843"), "{rendered}");
    assert!(
        rendered.contains("(image v2.0.0 ➔ v2.1.0)"),
        "the bump names its image: {rendered}"
    );
    assert!(
        rendered.contains("2 other commits in this range changed other paths"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("ArgoCD Rollout:"),
        "the Why line names the sync once: {rendered}"
    );
}

#[test]
fn github_is_asked_once_per_rollout_and_a_failure_is_retried_on_refresh() {
    let _settings = common::env::isolate_settings();
    let mut state = why_state(with_sync);
    assert!(render_card(&state).contains("GitHub: looking up the pull requests"));

    let ask = state.next_cause_ask().unwrap();
    assert_eq!(ask.revision, SYNCED);
    assert_eq!(ask.previous.as_deref(), Some(PREVIOUS));
    assert_eq!(ask.path, "apps/checkout");
    state.causes.insert(ask.key.clone(), CauseLookup::Loading);
    assert_eq!(state.next_cause_ask(), None, "in flight: not asked again");

    state.causes.insert(
        ask.key.clone(),
        CauseLookup::Failed(
            "not found on GitHub, or the repository is private and no GITHUB_TOKEN is set"
                .to_string(),
        ),
    );
    let rendered = render_card(&state);
    assert!(
        rendered.contains("private and no GITHUB_TOKEN is set"),
        "{rendered}"
    );
    assert!(rendered.contains("[r] to retry"), "{rendered}");
    assert_eq!(
        state.next_cause_ask(),
        None,
        "a failure waits for a refresh"
    );

    let report = state.report.clone().unwrap();
    state.set_report(report);
    assert_eq!(state.next_cause_ask().map(|a| a.key), Some(ask.key));

    state.active_tab = ChangedTab::Infra;
    assert_eq!(state.next_cause_ask(), None, "only for the Deployments tab");
}

#[test]
fn a_sync_github_cannot_explain_says_why_and_asks_nothing() {
    let _settings = common::env::isolate_settings();
    let chart = why_state(|d| {
        with_sync(d);
        let r = d.gitops.as_mut().unwrap().rollout.as_mut().unwrap();
        r.is_chart = true;
        r.revision = "1.4.3".to_string();
        r.previous_revision = Some("1.4.2".to_string());
    });
    let rendered = render_card(&chart);
    assert!(
        rendered.contains("Argo sync #17 to 1.4.3 from 1.4.2"),
        "{rendered}"
    );
    assert!(
        rendered.contains("chart source, chart version 1.4.3"),
        "{rendered}"
    );
    assert_eq!(chart.next_cause_ask(), None);

    let gitlab = why_state(|d| {
        with_sync(d);
        d.gitops
            .as_mut()
            .unwrap()
            .rollout
            .as_mut()
            .unwrap()
            .repo_url = "https://gitlab.com/org/checkout.git".to_string();
    });
    assert!(render_card(&gitlab).contains("not a github.com repository"));
    assert_eq!(gitlab.next_cause_ask(), None);
}

#[test]
fn what_could_not_be_found_out_is_said_as_such() {
    let _settings = common::env::isolate_settings();
    let unresolved = why_state(|d| {
        d.gitops = None;
        d.gitops_unresolved = Some(
            "tracking id names Argo app checkout-prod; Argo unavailable: list timed out"
                .to_string(),
        );
    });
    assert!(render_card(&unresolved)
        .contains("Why: tracking id names Argo app checkout-prod; Argo unavailable"));

    // What the report puts on a row with no app while Argo cannot be read.
    let argo_down = why_state(|d| {
        d.gitops = None;
        d.gitops_unresolved = Some(
            "Argo unavailable: Failed to list ArgoCD Applications: 403; ownership not known"
                .to_string(),
        );
    });
    assert!(render_card(&argo_down)
        .contains("Why: Argo unavailable: Failed to list ArgoCD Applications: 403"));

    let restarted = why_state(|d| {
        with_sync(d);
        d.local_cause = Some("rollout restart at 2026-09-23T10:00:00Z".to_string());
    });
    let rendered = render_card(&restarted);
    assert!(
        rendered.contains("Why: rollout restart at 2026-09-23T10:00:00Z"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("GitHub:"),
        "a restart brought no commits: {rendered}"
    );

    let unmatched = why_state(|d| {
        d.gitops.as_mut().unwrap().rollout_unmatched =
            Some("no Argo sync around this rollout: a change made outside Argo".to_string());
    });
    assert!(render_card(&unmatched).contains("Why: no Argo sync around this rollout"));

    let unmanaged = why_state(|d| d.gitops = None);
    assert!(
        !render_card(&unmanaged).contains("Why:"),
        "no Argo, nothing to say"
    );
}

#[test]
fn g_opens_a_persons_pr_before_a_bots_else_a_commit_else_the_compare_view() {
    let _settings = common::env::isolate_settings();
    let mut state = why_state(with_sync);
    assert_eq!(
        state.cause_link(),
        Err("Still looking up the pull requests on GitHub".to_string())
    );
    assert!(!render_card(&state).contains("[g] Open PR"));

    let url = |n| format!("https://github.com/org/checkout/pull/{n}");
    let with_url = |mut p: CausePull| {
        p.html_url = url(p.number);
        p
    };
    answer(
        &mut state,
        CauseLookup::Ready(RolloutCause {
            pulls: vec![
                with_url(pr(1843, "chore(deps): bump", "renovate[bot]", true)),
                with_url(pr(1842, "fix: timeout", "bob", false)),
            ],
            compare_url: Some("https://github.com/org/checkout/compare/a...b".to_string()),
            ..Default::default()
        }),
    );
    assert_eq!(
        state.cause_link(),
        Ok((url(1842), "PR #1842 (1 of 2 in this sync)".to_string()))
    );
    assert!(render_card(&state).contains("[g] Open PR"));

    let key = state.causes.keys().next().unwrap().clone();
    let commit = srelens_registry::github::CauseCommit {
        sha: SYNCED.to_string(),
        html_url: "https://github.com/org/checkout/commit/a1b2c3d".to_string(),
        ..Default::default()
    };
    state.causes.insert(
        key.clone(),
        CauseLookup::Ready(RolloutCause {
            commits: vec![commit],
            direct_commits: vec![SYNCED.to_string()],
            ..Default::default()
        }),
    );
    assert_eq!(
        state.cause_link().map(|(_, what)| what),
        Ok("commit a1b2c3d".to_string())
    );

    state.causes.insert(
        key,
        CauseLookup::Ready(RolloutCause {
            compare_url: Some("https://github.com/org/checkout/compare/a...b".to_string()),
            ..Default::default()
        }),
    );
    assert_eq!(
        state.cause_link().map(|(_, what)| what),
        Ok("the sync's compare view".to_string())
    );

    let chart = why_state(|d| {
        with_sync(d);
        d.gitops
            .as_mut()
            .unwrap()
            .rollout
            .as_mut()
            .unwrap()
            .is_chart = true;
    });
    assert!(chart.cause_link().unwrap_err().starts_with("chart source"));
    assert!(!render_card(&chart).contains("[g] Open PR"));
}

fn card_with_argo(argo: ArgoCoverage) -> String {
    let mut report = sample_report();
    report.argo = argo;
    let mut state = ChangedViewState::new();
    state.set_report(report);
    render_card(&state)
}

#[test]
fn the_card_says_how_much_of_argo_its_gitops_fields_were_matched_against() {
    let _settings = common::env::isolate_settings();
    let hub = || Some("tools".to_string());

    let none = card_with_argo(ArgoCoverage::default());
    assert!(
        !none.contains("Argo: "),
        "no Argo, nothing to qualify: {none}"
    );

    let complete = card_with_argo(ArgoCoverage {
        state: ArgoCoverageState::Complete,
        apps_loaded: 171,
        hub: hub(),
        fetched_at: Some(srelens_kube::k8s_openapi::jiff::Timestamp::now().to_string()),
        error: None,
    });
    assert!(
        complete.contains("Argo: 171 apps from hub tools, as of 0s ago"),
        "{complete}"
    );

    let loading = card_with_argo(ArgoCoverage {
        state: ArgoCoverageState::Partial,
        apps_loaded: 12,
        hub: hub(),
        ..Default::default()
    });
    assert!(
        loading.contains("Argo: loading from hub tools (12 apps for this cluster so far)"),
        "{loading}"
    );

    let stale = card_with_argo(ArgoCoverage {
        state: ArgoCoverageState::Stale,
        apps_loaded: 171,
        hub: hub(),
        fetched_at: Some("2026-09-01T10:00:00Z".to_string()),
        error: Some("hub unreachable".to_string()),
    });
    assert!(
        stale.contains("Argo: 171 apps from hub tools, as of"),
        "{stale}"
    );
    assert!(stale.contains("last refresh: hub unreachable"), "{stale}");

    let stale_no_err = card_with_argo(ArgoCoverage {
        state: ArgoCoverageState::Stale,
        apps_loaded: 171,
        hub: hub(),
        fetched_at: Some("2026-09-01T10:00:00Z".to_string()),
        error: None,
    });
    assert!(
        stale_no_err.contains("Argo: 171 apps from hub tools, as of"),
        "{stale_no_err}"
    );
    assert!(
        !stale_no_err.contains("refreshing"),
        "stale with no active refresh must not say refreshing: {stale_no_err}"
    );

    let down = card_with_argo(ArgoCoverage {
        state: ArgoCoverageState::Unavailable,
        error: Some("Argo lookup timed out after 20s".to_string()),
        ..Default::default()
    });
    assert!(
        down.contains("Argo: unavailable: Argo lookup timed out after 20s"),
        "{down}"
    );
}

#[test]
fn a_range_with_nothing_under_the_app_path_says_so() {
    let _settings = common::env::isolate_settings();
    let mut state = why_state(with_sync);
    answer(
        &mut state,
        CauseLookup::Ready(RolloutCause {
            revision: SYNCED.to_string(),
            previous_revision: Some(PREVIOUS.to_string()),
            path: "apps/checkout".to_string(),
            other_commits: 3,
            ..Default::default()
        }),
    );
    let rendered = render_card(&state);
    assert!(
        rendered.contains("GitHub: no commits under apps/checkout between 9f8e7d6 and a1b2c3d"),
        "{rendered}"
    );
}

#[test]
fn card_does_not_render_successful_sync_as_sync_error() {
    let _settings = common::env::isolate_settings();
    let mut report = sample_report();
    report.deployments[0].gitops.as_mut().unwrap().sync_message =
        Some("successfully synced (all tasks run)".into());
    let mut state = ChangedViewState::new();
    state.set_report(report);
    let card = render_card(&state);
    assert!(
        !card.contains("Sync Error:"),
        "must not render Sync Error for successful sync: {card}"
    );
}

#[test]
fn card_labels_health_message_as_health_not_sync_error_even_when_out_of_sync() {
    let _settings = common::env::isolate_settings();
    let mut report = sample_report();
    let mut gitops = report.deployments[0].gitops.clone().unwrap();
    gitops.sync_status = "OutOfSync".into();
    gitops.health_status = "Degraded".into();
    gitops.sync_message = Some("Deployment has 0/2 ready pods".into());
    gitops.is_health_message = true;
    report.deployments[0].gitops = Some(gitops);

    let mut state = ChangedViewState::new();
    state.set_report(report);
    let card = render_card(&state);
    assert!(
        card.contains("Health: Deployment has 0/2 ready pods"),
        "{card}"
    );
    assert!(
        !card.contains("Sync Error:"),
        "must not say Sync Error for health message: {card}"
    );
}

#[test]
fn changed_view_renders_sre_guide_banner_and_respects_toggle_and_height() {
    let _settings = common::env::isolate_settings();
    let mut state = ChangedViewState::new();
    state.set_report(sample_report());

    // 1. Tall screen (height >= 28) with guide banner enabled
    let lines = render_lines(140, 35, |f| render_changed_view(f, f.area(), &state));
    let full = lines.join("\n");
    assert!(full.contains("SRE Scope & Triage Guide"), "{full}");
    assert!(full.contains("[CHANGED]"), "{full}");
    assert!(full.contains("[S] Scaled:"), "{full}");
    assert!(full.contains("[u] Failing:"), "{full}");
    assert!(full.contains("[Tab] Infra:"), "{full}");
    assert!(full.contains("Non-deployment warnings"), "{full}");
    assert!(!full.contains("CNI, Ingress"), "{full}");
    assert!(!full.contains("Quick RCA  [a] Assistant"), "{full}");

    // 2. Guide banner toggled off
    state.show_guide_banner = false;
    let lines_off = render_lines(140, 35, |f| render_changed_view(f, f.area(), &state));
    let full_off = lines_off.join("\n");
    assert!(!full_off.contains("SRE Scope & Triage Guide"), "{full_off}");

    // 3. Short screen (height < 28) suppresses guide banner even when enabled
    state.show_guide_banner = true;
    let lines_short = render_lines(140, 25, |f| render_changed_view(f, f.area(), &state));
    let full_short = lines_short.join("\n");
    assert!(
        !full_short.contains("SRE Scope & Triage Guide"),
        "{full_short}"
    );
}
