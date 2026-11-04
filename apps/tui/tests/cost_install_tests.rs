//! `:cost` offering to install Kubecost, asking first, and showing the real
//! figures once it is there.

mod common;

use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use srectl::app::ActiveView;
use srectl::commands::ResourceKind;
use srectl::ui::Modal;
use srectl::views::cost_view::InstallState;
use srelens_kube::cost::{ClusterCompute, NamespaceCompute, NodeCompute, Rates};
use srelens_kube::kubecost_install::Step;
use srelens_kube::opencost::{NamespaceSpend, ProviderCosts, Source};

fn data() -> ClusterCompute {
    let r = Rates::default();
    ClusterCompute {
        rates: r,
        nodes: vec![NodeCompute {
            name: "node-a".into(),
            instance_type: None,
            zone: None,
            allocatable_cpu_m: 4000,
            allocatable_mem_mib: 8192,
            requested_cpu_m: 500,
            requested_mem_mib: 512,
            usage_cpu_m: None,
            usage_mem_mib: None,
            pods: 3,
            hourly: r.hourly(4000, 8192),
        }],
        namespaces: vec![NamespaceCompute {
            namespace: "default".into(),
            pods: 3,
            requested_cpu_m: 500,
            requested_mem_mib: 512,
            hourly: r.hourly(500, 512),
        }],
        pods: 3,
        allocatable_cpu_m: 4000,
        allocatable_mem_mib: 8192,
        requested_cpu_m: 500,
        requested_mem_mib: 512,
        usage_cpu_m: None,
        usage_mem_mib: None,
        cluster_hourly: r.hourly(4000, 8192),
        requested_hourly: r.hourly(500, 512),
        truncated: false,
        usage_note: None,
        provider: None,
        provider_note: None,
        discovery_note: None,
    }
}

fn with_provider() -> ClusterCompute {
    let mut d = data();
    d.provider = Some(ProviderCosts {
        source: Source::Kubecost,
        location: "kubecost/kubecost-cost-analyzer".into(),
        window_days: 7.0,
        total: 10.0,
        idle: 4.0,
        namespaces: vec![NamespaceSpend { name: "default".into(), total: 6.0, cpu: 4.0, ram: 2.0, storage: 0.0, network: 0.0, efficiency: Some(0.5) }],
    });
    d
}

fn install_state(app: &srectl::app::App) -> InstallState {
    match &app.active_view {
        ActiveView::Cost(c) => c.install.clone(),
        _ => panic!("not on the cost view"),
    }
}

async fn on_cost(app: &mut srectl::app::App) {
    app.switch_view_to_kind(ResourceKind::Cost).await;
}

// ---- the offer -------------------------------------------------------------

#[tokio::test]
async fn finding_no_cost_tool_offers_to_install_one() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    assert!(matches!(&app.modal, Some(Modal::KubecostInstall { context, .. }) if context == "test-cluster"));
}

#[tokio::test]
async fn the_offer_is_made_once_per_cluster_not_on_every_refresh() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());
    for _ in 0..3 {
        app.handle_cost_result("test-cluster", Ok(data()));
    }
    assert!(app.modal.is_none(), "declined once, not nagged again");
}

#[tokio::test]
async fn no_offer_when_a_cost_tool_is_already_there() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(with_provider()));
    assert!(app.modal.is_none());
}

#[tokio::test]
async fn no_offer_when_one_is_installed_but_not_answering() {
    // Installing again would be wrong: it exists, it just has no data yet.
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    let mut d = data();
    d.provider_note = Some("Kubecost has no cost data yet".into());
    app.handle_cost_result("test-cluster", Ok(d));
    assert!(app.modal.is_none());
}

#[tokio::test]
async fn no_offer_on_top_of_another_dialog_or_after_an_error() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Err("connection refused".into()));
    assert!(app.modal.is_none(), "a failed load offers nothing");
}

#[tokio::test]
async fn i_reopens_the_offer_after_it_was_declined() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Esc)).await;
    common::press(&mut app, common::ch('i')).await;
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })));
}

#[tokio::test]
async fn i_with_a_cost_tool_present_says_so_instead_of_offering() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(with_provider()));
    common::press(&mut app, common::ch('i')).await;
    assert!(app.modal.is_none());
    let (msg, _, _) = app.toast.clone().expect("a toast");
    assert!(msg.contains("Kubecost") && msg.to_lowercase().contains("already"), "{msg}");
}

// ---- the note --------------------------------------------------------------

#[tokio::test]
async fn the_note_says_what_will_happen_and_how_to_undo_it() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    let s = common::render_app(&mut app, 140, 50);
    for needle in [
        "test-cluster",
        "2.8.7",
        "kubecost.github.io/cost-analyzer",
        "namespace",
        "4 pods",
        "410",
        "ClusterRole",
        "helm",
        "internet",
        "helm uninstall kubecost",
        "delete namespace kubecost",
        "[Enter/y]",
        "[Esc/n]",
    ] {
        assert!(s.contains(needle), "{needle} missing from the note:\n{s}");
    }
}

#[tokio::test]
async fn a_production_looking_cluster_gets_a_loud_warning_and_others_do_not() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app_with("eks-prod-east", "default").await;
    on_cost(&mut app).await;
    app.handle_cost_result("eks-prod-east", Ok(data()));
    assert!(common::render_app(&mut app, 140, 50).contains("PRODUCTION"));

    let (mut app, _rx) = common::app_with("kind-dev", "default").await;
    on_cost(&mut app).await;
    app.handle_cost_result("kind-dev", Ok(data()));
    assert!(!common::render_app(&mut app, 140, 50).contains("PRODUCTION"));
}

#[tokio::test]
async fn the_note_fits_a_small_terminal() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    for (w, h) in [(80, 24), (200, 50), (40, 12), (20, 6)] {
        assert!(!common::render_app(&mut app, w, h).trim().is_empty(), "{w}x{h}");
    }
}

// ---- asking first ----------------------------------------------------------

#[tokio::test]
async fn declining_installs_nothing() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    for decline in [common::key(KeyCode::Esc), common::ch('n'), common::ch('N')] {
        app.handle_cost_result("test-cluster", Ok(data()));
        common::press(&mut app, common::ch('i')).await;
        common::press(&mut app, decline).await;
        assert!(app.modal.is_none());
        assert_eq!(install_state(&app), InstallState::Idle);
        assert!(app.kubecost_install_task.is_none(), "no task was started");
    }
}

#[tokio::test]
async fn other_keys_in_the_dialog_neither_install_nor_close_it() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    for k in [common::ch(' '), common::ch('x'), common::key(KeyCode::Down), common::key(KeyCode::Tab)] {
        common::press(&mut app, k).await;
    }
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })));
    assert_eq!(install_state(&app), InstallState::Idle);
    assert!(app.kubecost_install_task.is_none());
}

#[tokio::test]
async fn confirming_starts_the_install_and_shows_the_first_step() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    assert!(app.modal.is_none());
    assert_eq!(install_state(&app), InstallState::Running(Step::AddingRepo));
    assert!(app.kubecost_install_task.is_some());
    assert!(common::render_app(&mut app, 140, 40).contains("Adding the Kubecost Helm repository"));
}

#[tokio::test]
async fn a_double_enter_starts_one_install_not_two() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    common::press(&mut app, common::ch('i')).await;
    assert!(app.modal.is_none(), "a second Enter or i cannot reopen the offer mid-install");
    let (msg, _, _) = app.toast.clone().unwrap();
    assert!(msg.to_lowercase().contains("already"), "{msg}");
}

// ---- progress and outcomes -------------------------------------------------

#[tokio::test]
async fn progress_steps_are_shown_in_order() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    for step in [Step::UpdatingRepo, Step::Installing, Step::WaitingForData] {
        app.handle_cost_install_step("test-cluster", step);
        assert_eq!(install_state(&app), InstallState::Running(step));
        assert!(common::render_app(&mut app, 140, 40).contains(step.label()));
    }
}

#[tokio::test]
async fn progress_for_another_cluster_changes_nothing() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    app.handle_cost_install_step("elsewhere", Step::Installing);
    assert_eq!(install_state(&app), InstallState::Running(Step::AddingRepo));
}

#[tokio::test]
async fn a_failed_install_says_why_and_how_to_retry_and_keeps_the_estimate() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    app.handle_cost_install_done("test-cluster", Err("Installing Kubecost: helm exited with code 1: no space".into()));

    assert!(matches!(install_state(&app), InstallState::Failed(_)));
    let s = common::render_app(&mut app, 150, 50);
    assert!(s.contains("no space"), "the reason:\n{s}");
    assert!(s.contains("again") && s.contains("i"), "how to retry:\n{s}");
    assert!(s.contains("ESTIMATE"), "the estimate is still shown:\n{s}");
}

#[tokio::test]
async fn a_failed_install_can_be_retried_with_i() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    app.handle_cost_install_done("test-cluster", Err("boom".into()));
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::ch('i')).await;
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })), "asks again, never installs unasked");
}

#[tokio::test]
async fn success_waits_for_data_then_the_real_figures_replace_the_waiting() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    app.handle_cost_install_done("test-cluster", Ok(()));
    assert_eq!(install_state(&app), InstallState::Running(Step::WaitingForData));

    // Kubecost has installed but says it has no data yet.
    let mut waiting = data();
    waiting.provider_note = Some("Kubecost has no cost data yet".into());
    app.handle_cost_result("test-cluster", Ok(waiting));
    assert_eq!(install_state(&app), InstallState::Running(Step::WaitingForData), "still waiting");
    assert!(app.modal.is_none(), "no second offer while waiting");

    app.handle_cost_result("test-cluster", Ok(with_provider()));
    assert_eq!(install_state(&app), InstallState::Idle);
    let (msg, _, _) = app.toast.clone().unwrap();
    assert!(msg.contains("Kubecost"), "{msg}");
    assert!(common::render_app(&mut app, 170, 60).contains("Source: Kubecost"));
}

#[tokio::test]
async fn waiting_for_data_does_not_wait_forever() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.set_install(
        "test-cluster",
        InstallState::Running(Step::WaitingForData),
        Some(Instant::now() - Duration::from_secs(20 * 60)),
    );
    app.handle_tick();
    assert_eq!(install_state(&app), InstallState::Idle, "gave up quietly");
    let (msg, _, _) = app.toast.clone().expect("told the user");
    assert!(msg.contains("no data yet"), "{msg}");
}

// ---- clusters --------------------------------------------------------------

#[tokio::test]
async fn an_install_finishing_on_a_cluster_you_left_is_reported_not_shown() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![common::context_named("test-cluster"), common::context_named("staging")];
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;

    app.switch_context("staging".into()).await;
    assert_eq!(install_state(&app), InstallState::Idle, "the new cluster's dashboard starts clean");

    app.handle_cost_install_done("test-cluster", Ok(()));
    assert_eq!(install_state(&app), InstallState::Idle, "not applied to staging");
    let (msg, _, _) = app.toast.clone().expect("a toast");
    assert!(msg.contains("test-cluster") && msg.contains("Kubecost"), "{msg}");
}

#[tokio::test]
async fn switching_clusters_does_not_kill_an_install_in_progress() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![common::context_named("test-cluster"), common::context_named("staging")];
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Enter)).await;
    app.switch_context("staging".into()).await;
    // Killing helm half way would leave a half-installed release behind.
    assert!(app.kubecost_install_task.is_some(), "the task is left to finish");
}

// ---- a dashboard left under another screen -----------------------------------

#[tokio::test]
async fn a_saved_dashboard_hears_about_an_install_that_finished_while_it_was_away() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Esc)).await; // decline the offer
    app.start_kubecost_install();
    assert_eq!(install_state(&app), InstallState::Running(Step::AddingRepo));

    // Open :pods over it; the dashboard is parked on the back stack.
    app.switch_view_to_kind(ResourceKind::Pods).await;
    assert!(!matches!(app.active_view, ActiveView::Cost(_)));

    app.handle_cost_install_done("test-cluster", Err("Installing Kubecost: no space".into()));
    let parked = app
        .nav_stack
        .iter()
        .find_map(|v| match v {
            ActiveView::Cost(c) => Some(c.install.clone()),
            _ => None,
        })
        .expect("the dashboard is saved under the pods view");
    assert!(matches!(parked, InstallState::Failed(_)), "not stuck on Running: {parked:?}");

    // Esc back: the failure is what it shows, so `i` can offer a retry.
    common::press(&mut app, common::key(KeyCode::Esc)).await;
    assert!(matches!(install_state(&app), InstallState::Failed(_)));
}

#[tokio::test]
async fn a_dashboard_opened_later_on_the_same_cluster_knows_an_install_is_running() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Esc)).await;
    app.start_kubecost_install();
    app.switch_view_to_kind(ResourceKind::Pods).await;
    on_cost(&mut app).await;
    assert_eq!(install_state(&app), InstallState::Running(Step::AddingRepo));
}

#[tokio::test]
async fn a_late_step_does_not_resurrect_a_finished_install() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    common::press(&mut app, common::key(KeyCode::Esc)).await;
    app.start_kubecost_install();
    app.handle_cost_install_done("test-cluster", Err("boom".into()));
    app.handle_cost_install_step("test-cluster", Step::Installing);
    assert!(matches!(install_state(&app), InstallState::Failed(_)));
}

// ---- the note is readable on a small terminal --------------------------------

#[tokio::test]
async fn the_install_note_scrolls_and_keeps_the_choice_visible_at_80x24() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    on_cost(&mut app).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })));

    let top = common::render_app(&mut app, 80, 24);
    assert!(top.contains("[Enter/y]") && top.contains("[Esc/n]"), "the choice is always on screen:\n{top}");
    assert!(!top.contains("helm uninstall"), "the removal commands are below the fold at first:\n{top}");

    common::press(&mut app, common::key(KeyCode::End)).await;
    let end = common::render_app(&mut app, 80, 24);
    assert!(end.contains("helm uninstall"), "scrolled to the removal commands:\n{end}");
    assert!(end.contains("[Enter/y]") && end.contains("[Esc/n]"), "and the choice is still there:\n{end}");

    // Down at the end builds no hidden offset: one Up moves at once.
    for _ in 0..20 {
        common::press(&mut app, common::key(KeyCode::Down)).await;
    }
    common::press(&mut app, common::key(KeyCode::Up)).await;
    let up = common::render_app(&mut app, 80, 24);
    assert_ne!(end, up, "one step up changes the view");

    common::press(&mut app, common::key(KeyCode::Home)).await;
    let home = common::render_app(&mut app, 80, 24);
    assert_eq!(top, home);

    // Scrolling never answers the question.
    assert!(matches!(app.modal, Some(Modal::KubecostInstall { .. })));
    assert_eq!(install_state(&app), InstallState::Idle);
}
