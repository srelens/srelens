//! `:cost`, the Cost & Compute dashboard for the selected cluster.

mod common;

use crossterm::event::KeyCode;
use srectl::app::ActiveView;
use srectl::commands::{resolve_command_with_crds, CommandTarget, ResourceKind};
use srectl::views::cost_view::CostViewState;
use srelens_kube::contexts::ContextDto;
use srelens_kube::cost::{ClusterCompute, NamespaceCompute, NodeCompute, Rates};
use srelens_kube::opencost::{NamespaceSpend, ProviderCosts, Source};

fn rates() -> Rates {
    Rates { cpu_core_hour: 0.04, mem_gib_hour: 0.005 }
}

fn data() -> ClusterCompute {
    let r = rates();
    let node = |name: &str, ty: Option<&str>, used: Option<(i64, i64)>| NodeCompute {
        name: name.into(),
        instance_type: ty.map(String::from),
        zone: Some("us-east-1a".into()),
        allocatable_cpu_m: 4000,
        allocatable_mem_mib: 16 * 1024,
        requested_cpu_m: 1000,
        requested_mem_mib: 4 * 1024,
        usage_cpu_m: used.map(|u| u.0),
        usage_mem_mib: used.map(|u| u.1),
        pods: 7,
        hourly: r.hourly(4000, 16 * 1024),
    };
    let ns = |name: &str, pods: usize, cpu: i64, mem: i64| NamespaceCompute {
        namespace: name.into(),
        pods,
        requested_cpu_m: cpu,
        requested_mem_mib: mem,
        hourly: r.hourly(cpu, mem),
    };
    ClusterCompute {
        rates: r,
        nodes: vec![
            node("worker-alpha", Some("m5.xlarge"), Some((1500, 6000))),
            node("worker-bravo", None, Some((500, 2000))),
        ],
        namespaces: vec![ns("payments", 9, 1500, 6 * 1024), ns("kube-system", 5, 500, 2 * 1024)],
        pods: 14,
        allocatable_cpu_m: 8000,
        allocatable_mem_mib: 32 * 1024,
        requested_cpu_m: 2000,
        requested_mem_mib: 8 * 1024,
        usage_cpu_m: Some(2000),
        usage_mem_mib: Some(8000),
        cluster_hourly: r.hourly(8000, 32 * 1024),
        requested_hourly: r.hourly(2000, 8 * 1024),
        truncated: false,
        usage_note: None,
        provider: None,
        provider_note: None,
    }
}

fn ctx(name: &str) -> ContextDto {
    ContextDto {
        name: name.to_string(),
        stable_id: format!("file/{name}"),
        key: format!("file/{name}"),
        pinned_id: None,
        cluster: format!("c-{name}"),
        server: format!("https://{name}.example.invalid"),
        namespace: String::new(),
        is_current: false,
        is_local: false,
        provider: None,
        source_file: "cluster-config.yaml".to_string(),
        auth_kind: "token".to_string(),
    }
}

fn render(state: &CostViewState, w: u16, h: u16) -> String {
    common::render_text(w, h, |f| srectl::views::cost_view::render(f, f.area(), state))
}

// ---- the command ----------------------------------------------------------

#[test]
fn cost_and_its_aliases_open_the_dashboard() {
    for word in ["cost", "costs", "compute", "finops"] {
        assert_eq!(
            resolve_command_with_crds(word, &[]),
            Some(CommandTarget::Resource(ResourceKind::Cost)),
            "{word}"
        );
    }
}

#[test]
fn the_dashboard_is_cluster_wide() {
    assert!(!ResourceKind::Cost.is_namespaced());
}

// ---- rendering ------------------------------------------------------------

#[test]
fn a_loading_dashboard_says_so() {
    let s = render(&CostViewState::new("prod"), 120, 30);
    assert!(s.contains("Loading"), "{s}");
    assert!(s.contains("prod"), "names the cluster: {s}");
}

#[test]
fn a_failed_fetch_shows_the_reason_and_how_to_retry() {
    let mut st = CostViewState::new("prod");
    st.set_error("Failed to list nodes: connection refused".into());
    let s = render(&st, 120, 30);
    assert!(s.contains("connection refused") && s.contains("r"), "{s}");
    assert!(!s.contains("Loading"));
}

#[test]
fn the_dashboard_shows_the_clusters_cost_requested_and_idle() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    let s = render(&st, 140, 60);
    assert!(s.contains("Cost & Compute"), "{s}");
    // cluster: 8 cores * 0.04 + 32 GiB * 0.005 = 0.48/hr
    assert!(s.contains("$0.48") || s.contains("$0.480"), "hourly cost:\n{s}");
    assert!(s.contains("$350"), "monthly (0.48 * 730 = 350.4):\n{s}");
    assert!(s.contains("Requested") && s.contains("Idle"), "{s}");
}

#[test]
fn nodes_and_namespaces_are_listed_with_their_cost() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    let s = render(&st, 140, 60);
    for needle in ["worker-alpha", "worker-bravo", "m5.xlarge", "us-east-1a", "payments", "kube-system"] {
        assert!(s.contains(needle), "{needle} missing:\n{s}");
    }
    let payments = s.find("payments").unwrap();
    let system = s.find("kube-system").unwrap();
    assert!(payments < system, "largest requester first");
}

#[test]
fn it_is_labelled_an_estimate_with_the_rates_used() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    let s = render(&st, 140, 60);
    assert!(s.contains("ESTIMATE"), "{s}");
    assert!(s.contains("0.04") && s.contains("0.005"), "the rates are shown:\n{s}");
}

#[test]
fn live_usage_is_shown_when_metrics_server_reported() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    let s = render(&st, 140, 60);
    assert!(s.contains("Used"), "{s}");
    assert!(!s.contains("metrics-server"), "no warning when it answered:\n{s}");
}

#[test]
fn missing_usage_is_explained_not_shown_as_zero() {
    let mut d = data();
    d.usage_cpu_m = None;
    d.usage_mem_mib = None;
    for n in &mut d.nodes {
        n.usage_cpu_m = None;
        n.usage_mem_mib = None;
    }
    d.usage_note = Some("metrics-server is not installed in this cluster".into());
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    let s = render(&st, 140, 60);
    assert!(s.contains("metrics-server"), "{s}");
    assert!(s.contains("n/a"), "unknown usage reads n/a:\n{s}");
}

#[test]
fn a_truncated_listing_is_flagged_as_partial() {
    let mut d = data();
    d.truncated = true;
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    assert!(render(&st, 140, 60).to_lowercase().contains("partial"));
}

#[test]
fn an_empty_cluster_says_so_instead_of_drawing_empty_tables() {
    let mut d = data();
    d.nodes.clear();
    d.namespaces.clear();
    d.pods = 0;
    d.cluster_hourly = 0.0;
    d.requested_hourly = 0.0;
    d.allocatable_cpu_m = 0;
    d.allocatable_mem_mib = 0;
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    let s = render(&st, 120, 30);
    assert!(s.contains("No nodes"), "{s}");
}

#[test]
fn it_renders_at_narrow_and_short_sizes_without_panicking() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    for (w, h) in [(40, 10), (60, 8), (200, 5), (20, 3), (10, 2)] {
        let _ = render(&st, w, h);
    }
}

#[test]
fn scrolling_reveals_the_lower_sections() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    let top = render(&st, 120, 12);
    for _ in 0..40 {
        st.scroll_down();
    }
    let lower = render(&st, 120, 12);
    assert_ne!(top, lower);
    st.scroll_to_top();
    assert_eq!(render(&st, 120, 12), top);
}

// ---- in the app -----------------------------------------------------------

#[tokio::test]
async fn opening_cost_shows_the_dashboard_for_the_active_cluster() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Cost).await;
    match &app.active_view {
        ActiveView::Cost(c) => {
            assert!(c.is_loading);
            assert_eq!(c.context, "test-cluster");
        }
        _ => panic!("expected the cost view"),
    }
}

#[tokio::test]
async fn a_result_for_the_active_cluster_is_shown() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Cost).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    match &app.active_view {
        ActiveView::Cost(c) => {
            assert!(!c.is_loading && c.data.is_some() && c.error.is_none());
        }
        _ => panic!(),
    }
}

#[tokio::test]
async fn a_result_for_another_cluster_is_ignored() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Cost).await;
    app.handle_cost_result("some-other-cluster", Ok(data()));
    match &app.active_view {
        ActiveView::Cost(c) => assert!(c.data.is_none() && c.is_loading),
        _ => panic!(),
    }
}

#[tokio::test]
async fn an_error_for_the_active_cluster_is_shown_and_one_for_another_is_not() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Cost).await;
    app.handle_cost_result("elsewhere", Err("nope".into()));
    app.handle_cost_result("test-cluster", Err("connection refused".into()));
    match &app.active_view {
        ActiveView::Cost(c) => assert_eq!(c.error.as_deref(), Some("connection refused")),
        _ => panic!(),
    }
}

#[tokio::test]
async fn switching_cluster_keeps_the_dashboard_and_loads_the_new_one() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.contexts = vec![ctx("test-cluster"), ctx("staging")];
    app.switch_view_to_kind(ResourceKind::Cost).await;
    app.handle_cost_result("test-cluster", Ok(data()));

    app.switch_context("staging".into()).await;

    match &app.active_view {
        ActiveView::Cost(c) => {
            assert_eq!(c.context, "staging", "now the other cluster's dashboard");
            assert!(c.data.is_none() && c.is_loading, "the old cluster's numbers are gone");
        }
        _ => panic!("the dashboard should follow the cluster, not be thrown away"),
    }
    // A late answer for the cluster we left must not land on the new one.
    app.handle_cost_result("test-cluster", Ok(data()));
    match &app.active_view {
        ActiveView::Cost(c) => assert!(c.data.is_none()),
        _ => panic!(),
    }
}

#[tokio::test]
async fn r_reloads_and_esc_goes_back() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Cost).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    // No cost tool in `data()`, so the install offer is open; decline it.
    common::press(&mut app, common::key(KeyCode::Esc)).await;
    assert!(app.modal.is_none());

    common::press(&mut app, common::ch('r')).await;
    match &app.active_view {
        ActiveView::Cost(c) => assert!(c.is_loading, "r starts a reload"),
        _ => panic!(),
    }

    common::press(&mut app, common::key(KeyCode::Esc)).await;
    assert!(!matches!(app.active_view, ActiveView::Cost(_)), "Esc leaves the dashboard");
}

#[tokio::test]
async fn the_rates_come_from_the_tui_config_and_bad_ones_fall_back() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    assert_eq!(app.cost_rates(), Rates::default());

    app.tui_config.cost_cpu_core_hour = Some(0.05);
    app.tui_config.cost_mem_gib_hour = Some(0.006);
    assert_eq!(app.cost_rates(), Rates { cpu_core_hour: 0.05, mem_gib_hour: 0.006 });

    for bad in [-1.0, f64::NAN, f64::INFINITY, 0.0] {
        app.tui_config.cost_cpu_core_hour = Some(bad);
        app.tui_config.cost_mem_gib_hour = Some(bad);
        assert_eq!(app.cost_rates(), Rates::default(), "{bad} is not a price");
    }
}

#[tokio::test]
async fn the_dashboard_appears_in_the_app_frame_with_its_hints() {
    let _settings = common::env::isolate_settings();
    let (mut app, _rx) = common::app().await;
    app.switch_view_to_kind(ResourceKind::Cost).await;
    app.handle_cost_result("test-cluster", Ok(data()));
    let screen = common::render_app(&mut app, 160, 50);
    assert!(screen.contains("Cost & Compute"), "{screen}");
    assert!(screen.contains("Refresh"), "key hints: {screen}");
}

#[test]
fn node_table_columns_do_not_run_into_each_other() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    let s = render(&st, 160, 60);
    let row = s.lines().find(|l| l.contains("worker-alpha")).expect("a node row");
    // CPU req/alloc, then MEM req/alloc, separated by space.
    let re = regex::Regex::new(r"1\.0/4\.0\s+4\.0Gi/16\.0Gi").unwrap();
    assert!(re.is_match(row), "columns collide: {row}");
    let header = s.lines().find(|l| l.contains("CPU req/all")).expect("the node header");
    assert!(!header.contains("CPU req/allMEM"), "header collides: {header}");
}


// ---- real cost from Kubecost / OpenCost ------------------------------------

fn spend(name: &str, total: f64, eff: Option<f64>) -> NamespaceSpend {
    NamespaceSpend { name: name.into(), total, cpu: total * 0.6, ram: total * 0.3, storage: total * 0.1, network: 0.0, efficiency: eff }
}

fn provider() -> ProviderCosts {
    ProviderCosts {
        source: Source::Kubecost,
        location: "kubecost/kubecost-cost-analyzer".into(),
        window_days: 7.0,
        total: 20.0,
        idle: 5.0,
        namespaces: vec![spend("payments", 12.0, Some(0.6)), spend("kube-system", 3.0, Some(0.25))],
    }
}

fn with_provider() -> CostViewState {
    let mut d = data();
    d.provider = Some(provider());
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    st
}

#[test]
fn real_spend_is_shown_and_its_source_named() {
    let s = render(&with_provider(), 170, 70);
    assert!(s.contains("Kubecost"), "{s}");
    assert!(s.contains("kubecost/kubecost-cost-analyzer"), "where it came from:
{s}");
    assert!(s.contains("7"), "the window: {s}");
    // 20 over 7 days -> 86.90/mo; idle 5 -> 21.73/mo.
    assert!(s.contains("$86.9"), "real monthly total:\n{s}");
    assert!(s.contains("$21.7"), "real monthly idle:\n{s}");
}

#[test]
fn each_namespace_gets_its_real_cost_and_efficiency_beside_the_estimate() {
    let s = render(&with_provider(), 170, 70);
    let row = s.lines().find(|l| l.contains("payments")).expect("a payments row");
    // 12 over 7 days -> 52.14/mo, 60% efficient.
    assert!(row.contains("$52.14"), "{row}");
    assert!(row.contains("60%"), "{row}");
    let header = s.lines().find(|l| l.contains("NAMESPACE") && l.contains("SHARE")).unwrap();
    assert!(header.contains("REAL"), "{header}");
}

#[test]
fn a_namespace_the_cost_tool_does_not_know_shows_a_dash() {
    let mut d = data();
    let mut p = provider();
    p.namespaces.retain(|n| n.name != "kube-system");
    d.provider = Some(p);
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    let s = render(&st, 170, 70);
    let row = s.lines().find(|l| l.contains("kube-system")).unwrap();
    assert!(row.contains('-'), "{row}");
}

#[test]
fn without_one_the_screen_says_it_is_showing_the_estimate() {
    let mut st = CostViewState::new("prod");
    st.set_data(data());
    let s = render(&st, 170, 70);
    assert!(s.contains("No Kubecost or OpenCost"), "{s}");
    assert!(s.contains("ESTIMATE"), "{s}");
    assert!(!s.contains("REAL"), "no real columns without a source:\n{s}");
}

#[test]
fn a_cost_tool_that_is_installed_but_failed_is_reported_and_the_estimate_is_kept() {
    let mut d = data();
    d.provider_note = Some("Kubecost at kubecost/kubecost-cost-analyzer failed: 503".into());
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    let s = render(&st, 170, 70);
    assert!(s.contains("kubecost-cost-analyzer failed: 503"), "{s}");
    assert!(s.to_lowercase().contains("estimate"), "{s}");
    assert!(!s.contains("No Kubecost or OpenCost"), "it was found, it just failed:\n{s}");
}

#[test]
fn opencost_is_named_when_that_is_the_source() {
    let mut d = data();
    let mut p = provider();
    p.source = Source::OpenCost;
    p.location = "opencost/opencost".into();
    d.provider = Some(p);
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    let s = render(&st, 170, 70);
    assert!(s.contains("OpenCost") && s.contains("opencost/opencost"), "{s}");
}

#[test]
fn the_history_behind_the_figures_is_stated_in_readable_units() {
    let s = render(&with_provider(), 170, 70);
    assert!(s.contains("7 days"), "{s}");
    assert!(!s.contains("last 0"), "{s}");
}

#[test]
fn a_short_sample_is_labelled_in_minutes_and_flagged_as_extrapolated() {
    let mut d = data();
    let mut p = provider();
    p.window_days = 5.0 / 1440.0;
    d.provider = Some(p);
    let mut st = CostViewState::new("prod");
    st.set_data(d);
    let s = render(&st, 170, 70);
    assert!(s.contains("5 min"), "{s}");
    assert!(!s.contains("last 0"), "{s}");
    let lower = s.to_lowercase();
    assert!(lower.contains("short") && lower.contains("extrapolat"), "warns the monthly figures will move:\n{s}");
}

#[test]
fn a_full_window_carries_no_short_sample_warning() {
    let s = render(&with_provider(), 170, 70).to_lowercase();
    assert!(!s.contains("extrapolat"), "{s}");
}
