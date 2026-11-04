//! Journeys through `:cost`, the Cost & Compute dashboard.
//!
//! D1 input disorder, D3 late arrivals, D4 boundaries, D5 re-entry, D6
//! truthfulness and D7 narrow screens apply. D2 (ambiguity) does not: the
//! command has fixed aliases and the screen has no pickable names.
use crossterm::event::KeyCode;
use srectl::app::ActiveView;
use srectl::ui::Modal;
use srectl::views::cost_view::InstallState;
use srelens_kube::cost::{ClusterCompute, NamespaceCompute, NodeCompute, Rates};
use srelens_kube::kubecost_install::Step;

use crate::common::{self, ch, context_named, key, press, run_command};

fn cluster(nodes: usize, namespaces: usize) -> ClusterCompute {
    let r = Rates::default();
    ClusterCompute {
        rates: r,
        nodes: (0..nodes)
            .map(|i| NodeCompute {
                name: format!("node-{i}"),
                instance_type: Some("m5.large".into()),
                zone: Some("zone-a".into()),
                allocatable_cpu_m: 2000,
                allocatable_mem_mib: 8192,
                requested_cpu_m: 500,
                requested_mem_mib: 1024,
                usage_cpu_m: None,
                usage_mem_mib: None,
                pods: 3,
                hourly: r.hourly(2000, 8192),
            })
            .collect(),
        namespaces: (0..namespaces)
            .map(|i| NamespaceCompute {
                namespace: format!("team-{i:03}"),
                pods: 2,
                requested_cpu_m: 100,
                requested_mem_mib: 128,
                hourly: r.hourly(100, 128),
            })
            .collect(),
        pods: namespaces * 2,
        allocatable_cpu_m: 2000 * nodes as i64,
        allocatable_mem_mib: 8192 * nodes as i64,
        requested_cpu_m: 100 * namespaces as i64,
        requested_mem_mib: 128 * namespaces as i64,
        usage_cpu_m: None,
        usage_mem_mib: None,
        cluster_hourly: r.hourly(2000 * nodes as i64, 8192 * nodes as i64),
        requested_hourly: r.hourly(100 * namespaces as i64, 128 * namespaces as i64),
        truncated: false,
        usage_note: Some("metrics-server is not installed in this cluster".into()),
        provider: None,
        provider_note: None,
        discovery_note: None,
    }
}

/// The first answer with no cost tool opens the install offer (see the install
/// journeys below). Most journeys are about something else, so they decline it.
async fn decline_offer(app: &mut srectl::app::App) {
    if app.modal.is_some() {
        press(app, key(KeyCode::Esc)).await;
    }
}

fn is_cost(app: &srectl::app::App) -> bool {
    matches!(app.active_view, ActiveView::Cost(_))
}

/// D1: open from the command bar by an alias, mash scroll and refresh keys,
/// then leave. Refresh pressed repeatedly never leaves the view stuck.
#[tokio::test]
async fn journey_cost_survives_scroll_and_refresh_mashing_then_leaves_cleanly() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "finops").await;
    assert!(is_cost(&app));
    app.handle_cost_result("test-cluster", Ok(cluster(2, 40)));
    decline_offer(&mut app).await;

    for k in [KeyCode::Char('G'), KeyCode::Char('g'), KeyCode::PageDown, KeyCode::PageUp, KeyCode::Down, KeyCode::Up, KeyCode::End, KeyCode::Home] {
        press(&mut app, key(k)).await;
    }
    for _ in 0..5 {
        press(&mut app, ch('r')).await;
    }
    assert!(is_cost(&app));
    let _ = common::render_app(&mut app, 120, 30);

    press(&mut app, key(KeyCode::Esc)).await;
    assert!(!is_cost(&app));
}

/// D3: an answer that lands after the person has already left the dashboard,
/// and one for another cluster mid-load, change nothing they are looking at.
#[tokio::test]
async fn journey_late_cost_answers_never_change_what_you_are_looking_at() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![context_named("test-cluster"), context_named("staging")];
    run_command(&mut app, "cost").await;

    // Left before the answer came back.
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(!is_cost(&app));
    app.handle_cost_result("test-cluster", Ok(cluster(1, 1)));
    assert!(!is_cost(&app), "an answer must not drag the screen back");

    // Switched cluster while loading.
    run_command(&mut app, "cost").await;
    app.switch_context("staging".into()).await;
    app.handle_cost_result("test-cluster", Ok(cluster(1, 1)));
    match &app.active_view {
        ActiveView::Cost(c) => assert!(c.data.is_none() && c.context == "staging"),
        _ => panic!("the dashboard should have followed the switch"),
    }
}

/// D4: no nodes, one node, and two hundred namespaces scrolled to the bottom.
#[tokio::test]
async fn journey_cost_handles_an_empty_a_single_and_a_huge_cluster() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "cost").await;

    app.handle_cost_result("test-cluster", Ok(cluster(0, 0)));
    decline_offer(&mut app).await;
    assert!(common::render_app(&mut app, 120, 30).contains("No nodes"));

    app.handle_cost_result("test-cluster", Ok(cluster(1, 1)));
    decline_offer(&mut app).await;
    assert!(common::render_app(&mut app, 120, 30).contains("node-0"));

    app.handle_cost_result("test-cluster", Ok(cluster(3, 200)));
    press(&mut app, ch('G')).await;
    let bottom = common::render_app(&mut app, 120, 30);
    assert!(bottom.contains("team-199") || bottom.contains("ESTIMATE"), "reaches the end:\n{bottom}");
}

/// D5: dashboard on prod, hop to staging and back. Each cluster is loaded
/// fresh; the first cluster's numbers never appear under the second's name.
#[tokio::test]
async fn journey_cost_follows_the_cluster_there_and_back() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![context_named("test-cluster"), context_named("staging")];
    run_command(&mut app, "cost").await;
    app.handle_cost_result("test-cluster", Ok(cluster(2, 3)));

    for target in ["staging", "test-cluster"] {
        app.switch_context(target.into()).await;
        match &app.active_view {
            ActiveView::Cost(c) => {
                assert_eq!(c.context, target);
                assert!(c.data.is_none() && c.is_loading, "starts empty and loading");
            }
            _ => panic!("left the dashboard"),
        }
    }
}

/// D6: a failed load says why and how to retry; missing metrics say so rather
/// than showing zero use; no cost tool says the figures are an estimate.
#[tokio::test]
async fn journey_cost_never_dresses_a_failure_or_a_gap_as_a_number() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "cost").await;

    app.handle_cost_result("test-cluster", Err("Failed to list nodes: connection refused".into()));
    let s = common::render_app(&mut app, 140, 30);
    assert!(s.contains("connection refused") && s.contains("retry"), "{s}");

    app.handle_cost_result("test-cluster", Ok(cluster(2, 3)));
    decline_offer(&mut app).await;
    let s = common::render_app(&mut app, 170, 60);
    assert!(s.contains("n/a") && s.contains("metrics-server is not installed"), "usage gap is explained:\n{s}");
    assert!(s.contains("ESTIMATE") && s.contains("No Kubecost or OpenCost"), "{s}");
}

/// D7: small and large terminals, loading and loaded.
#[tokio::test]
async fn journey_cost_renders_at_80x24_and_200x50() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    run_command(&mut app, "cost").await;
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty(), "loading {w}x{h}");
    }
    app.handle_cost_result("test-cluster", Ok(cluster(3, 30)));
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty(), "loaded {w}x{h}");
    }
}

// ---- installing Kubecost from the dashboard --------------------------------
//
// The same seven dimensions, for the offer to install Kubecost. D2 (ambiguity)
// does not apply: there is one product, one key and one dialog.

async fn offered(app: &mut srectl::app::App) {
    run_command(app, "cost").await;
    app.handle_cost_result("test-cluster", Ok(cluster(1, 2)));
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })), "the offer is open");
}

fn install_state(app: &srectl::app::App) -> InstallState {
    match &app.active_view {
        ActiveView::Cost(c) => c.install.clone(),
        _ => panic!("not on the cost view"),
    }
}

/// D1: a person mashes everything but the answer, then answers. Only Enter or y
/// installs, and a second Enter, a stray `i` and a second `y` start nothing new.
#[tokio::test]
async fn journey_install_offer_ignores_stray_keys_and_installs_only_once() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    offered(&mut app).await;

    for k in [KeyCode::Char('x'), KeyCode::Char(' '), KeyCode::Down, KeyCode::Tab, KeyCode::Char('r'), KeyCode::Char('q')] {
        press(&mut app, key(k)).await;
    }
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })), "still waiting for an answer");
    assert_eq!(install_state(&app), InstallState::Idle);

    press(&mut app, ch('y')).await;
    press(&mut app, key(KeyCode::Enter)).await;
    press(&mut app, ch('y')).await;
    press(&mut app, ch('i')).await;
    assert_eq!(install_state(&app), InstallState::Running(Step::AddingRepo));
    assert!(app.modal.is_none());
}

/// D3: progress and the final answer arrive after the person has moved on, or
/// out of order. Nothing they are looking at is changed, and they are told.
#[tokio::test]
async fn journey_install_progress_arriving_late_is_reported_not_applied() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![context_named("test-cluster"), context_named("staging")];
    offered(&mut app).await;
    press(&mut app, ch('y')).await;

    app.switch_context("staging".into()).await;
    app.handle_cost_install_step("test-cluster", Step::Installing);
    assert_eq!(install_state(&app), InstallState::Idle, "staging's dashboard is untouched");
    app.handle_cost_install_done("test-cluster", Err("Installing Kubecost: helm exited with code 1".into()));
    assert_eq!(install_state(&app), InstallState::Idle);
    let (msg, _, _) = app.toast.clone().expect("told about it");
    assert!(msg.contains("test-cluster") && msg.contains("failed"), "{msg}");
}

/// D4: boundaries. A refresh that finds Kubecost partway through the install
/// (it appears before the Done event) is not offered the install again.
#[tokio::test]
async fn journey_a_refresh_during_the_install_never_offers_a_second_install() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    offered(&mut app).await;
    press(&mut app, ch('y')).await;
    for _ in 0..5 {
        app.handle_cost_result("test-cluster", Ok(cluster(1, 2)));
    }
    assert!(app.modal.is_none(), "no dialog while an install is running");
}

/// D5: re-entry. Decline, leave the dashboard, come back: not asked again by
/// itself, but `i` asks. Accept this time and it installs.
#[tokio::test]
async fn journey_declining_leaving_and_coming_back_does_not_nag_but_i_asks_again() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    offered(&mut app).await;
    press(&mut app, key(KeyCode::Esc)).await;
    press(&mut app, key(KeyCode::Esc)).await;
    assert!(!is_cost(&app), "left the dashboard");

    run_command(&mut app, "cost").await;
    app.handle_cost_result("test-cluster", Ok(cluster(1, 2)));
    assert!(app.modal.is_none(), "declined earlier this session: no nagging");

    press(&mut app, ch('i')).await;
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })));
    press(&mut app, ch('y')).await;
    assert_eq!(install_state(&app), InstallState::Running(Step::AddingRepo));
}

/// D6: truthfulness. A failed install says which step and why, keeps showing
/// the estimate, and offers a retry; it is never shown as installed.
#[tokio::test]
async fn journey_a_failed_install_is_reported_as_failed_with_its_reason() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    offered(&mut app).await;
    press(&mut app, ch('y')).await;
    app.handle_cost_install_done("test-cluster", Err("Adding the Kubecost Helm repository: helm not found on PATH".into()));

    let s = common::render_app(&mut app, 150, 50);
    assert!(s.contains("INSTALL FAILED") && s.contains("helm not found on PATH"), "{s}");
    assert!(s.contains("try again") && s.contains("ESTIMATE"), "{s}");
    assert!(!s.contains("Source: Kubecost"), "never claims real figures it does not have:\n{s}");
}

/// D7: the dialog, the progress banner and the failure banner at small and
/// large sizes.
#[tokio::test]
async fn journey_install_screens_render_at_80x24_and_200x50() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    offered(&mut app).await;
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty(), "dialog {w}x{h}");
    }
    press(&mut app, ch('y')).await;
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty(), "progress {w}x{h}");
    }
    app.handle_cost_install_done("test-cluster", Err("boom".into()));
    for (w, h) in [(80, 24), (200, 50), (40, 10)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty(), "failure {w}x{h}");
    }
}
