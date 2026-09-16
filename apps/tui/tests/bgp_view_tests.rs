//! Integration tests for the Cilium BGP Peering & Route Advertisement Dashboard view.
//!
//! Every test builds a state through its public constructors and mutators,
//! asserts the state, then renders it through ratatui's `TestBackend` at a
//! wide and a narrow size and asserts on text that only the intended branch
//! could have drawn. Nothing here touches a cluster, the network, or kubeconfig.

mod common;

use ratatui::backend::TestBackend;
use ratatui::{Frame, Terminal};

use srelens_kube::bgp::{
    BgpAdvertisedService, BgpClusterSummary, BgpEngineType, BgpIpPool, BgpNeighbor,
    BgpSessionState,
};
use srelens_tui::commands::{resolve_command, CommandTarget, ResourceKind};
use srelens_tui::deep_link::DeepLink;
use srelens_tui::views::bgp_view::{render_bgp_view, BgpTab, BgpViewState};

/// Render one frame and return the buffer as lines of text
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

fn sample_bgp_summary() -> BgpClusterSummary {
    BgpClusterSummary {
        engine: BgpEngineType::CiliumV2,
        total_nodes: 4,
        bgp_nodes: 3,
        total_peers: 3,
        established_peers: 2,
        degraded_peers: 1,
        peers: vec![
            BgpNeighbor {
                node_name: "node-worker-01".to_string(),
                peer_address: "10.0.0.254".to_string(),
                peer_asn: 65000,
                local_asn: 65100,
                session_state: BgpSessionState::Established,
                policy_name: "tor-spine-a".to_string(),
                export_pod_cidr: true,
                hold_time_seconds: Some(90),
                keepalive_time_seconds: Some(30),
                connect_retry_seconds: Some(120),
                multihop_ttl: Some(64),
                graceful_restart: true,
                advertised_prefixes: vec!["10.244.0.0/24".to_string(), "192.168.100.1/32".to_string()],
                routes_count: 8,
                routes_received: 316,
                uptime_or_last_change: Some("4d 12h".to_string()),
            },
            BgpNeighbor {
                node_name: "node-worker-02".to_string(),
                peer_address: "10.0.0.253".to_string(),
                peer_asn: 65000,
                local_asn: 65100,
                session_state: BgpSessionState::Established,
                policy_name: "tor-spine-b".to_string(),
                export_pod_cidr: true,
                hold_time_seconds: Some(90),
                keepalive_time_seconds: Some(30),
                connect_retry_seconds: Some(120),
                multihop_ttl: Some(64),
                graceful_restart: true,
                advertised_prefixes: vec!["10.244.1.0/24".to_string()],
                routes_count: 4,
                routes_received: 500,
                uptime_or_last_change: Some("2d 06h".to_string()),
            },
            BgpNeighbor {
                node_name: "node-worker-03".to_string(),
                peer_address: "10.0.1.254".to_string(),
                peer_asn: 65001,
                local_asn: 65100,
                session_state: BgpSessionState::Active,
                policy_name: "tor-backup".to_string(),
                export_pod_cidr: false,
                hold_time_seconds: Some(180),
                keepalive_time_seconds: Some(60),
                connect_retry_seconds: Some(120),
                multihop_ttl: None,
                graceful_restart: false,
                advertised_prefixes: vec![],
                routes_count: 0,
                routes_received: 0,
                uptime_or_last_change: Some("15m".to_string()),
            },
        ],
        advertised_services: vec![
            BgpAdvertisedService {
                service_name: "ingress-nginx-lb".to_string(),
                namespace: "ingress".to_string(),
                load_balancer_ip: "192.168.100.1".to_string(),
                ip_pool: Some("public-pool".to_string()),
                announcing_nodes: vec!["node-worker-01".to_string(), "node-worker-02".to_string()],
                peers: vec!["10.0.0.254".to_string(), "10.0.0.253".to_string()],
                service_type: "LoadBalancer".to_string(),
            },
            BgpAdvertisedService {
                service_name: "api-gateway".to_string(),
                namespace: "default".to_string(),
                load_balancer_ip: "192.168.100.2".to_string(),
                ip_pool: Some("public-pool".to_string()),
                announcing_nodes: vec!["node-worker-01".to_string()],
                peers: vec!["10.0.0.254".to_string()],
                service_type: "LoadBalancer".to_string(),
            },
        ],
        ip_pools: vec![
            BgpIpPool {
                name: "public-pool".to_string(),
                cidrs: vec!["192.168.100.0/24".to_string()],
                disabled: false,
                service_selector: "tier=public".to_string(),
            },
            BgpIpPool {
                name: "internal-pool".to_string(),
                cidrs: vec!["10.200.0.0/16".to_string()],
                disabled: true,
                service_selector: "tier=internal".to_string(),
            },
        ],
        error: None,
    }
}

#[test]
fn bgp_view_loading_and_empty_states() {
    let mut state = BgpViewState::new();
    assert!(state.is_loading);
    assert_eq!(state.active_tab, BgpTab::Peers);

    // Render loading state
    let lines = render_lines(120, 24, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");
    assert!(text.contains("BGP CONTROL PLANE & PEERING TOPOLOGY"));
    assert!(text.contains("Querying BGP control plane"));

    // Set error state
    state.set_error("CRD not installed".to_string());
    assert!(!state.is_loading);
    assert_eq!(state.error.as_deref(), Some("CRD not installed"));

    let lines = render_lines(120, 24, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");
    assert!(text.contains("BGP Error: CRD not installed"));
}

#[test]
fn bgp_view_renders_peers_and_summary_header() {
    let mut state = BgpViewState::new();
    state.set_summary(sample_bgp_summary());

    assert_eq!(state.filtered_peers().len(), 3);
    assert_eq!(state.filtered_services().len(), 2);
    assert_eq!(state.filtered_pools().len(), 2);

    let lines = render_lines(140, 30, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");

    // Header summary metrics
    assert!(text.contains("Cilium BGP (v2)"));
    assert!(text.contains("3/4 active BGP nodes"));
    assert!(text.contains("3 Total"));
    assert!(text.contains("2 Established"));
    assert!(text.contains("1 Degraded/Down"));
    assert!(text.contains("2 LoadBalancer Services"));

    // Peers table content
    assert!(text.contains("node-worker-01"));
    assert!(text.contains("10.0.0.254"));
    assert!(text.contains("65000"));
    assert!(text.contains("65100"));
    assert!(text.contains("Established"));
    assert!(text.contains("tor-spine-a"));
    assert!(text.contains("316 in / 8 out"));

    // Selected peer timer inspector in footer
    assert!(text.contains("Hold Time: 90s"));
    assert!(text.contains("KeepAlive: 30s"));
    assert!(text.contains("Graceful Restart: Enabled"));
    assert!(text.contains("316 Received │ 8 Advertised"));
    assert!(text.contains("10.244.0.0/24"));
    assert!(text.contains("192.168.100.1/32"));
}

#[test]
fn bgp_view_tab_cycling_and_row_navigation() {
    let mut state = BgpViewState::new();
    state.set_summary(sample_bgp_summary());

    // Initial state: Tab is Peers, selected index is 0
    assert_eq!(state.active_tab, BgpTab::Peers);
    assert_eq!(state.selected_peer_idx, 0);
    assert_eq!(state.selected_peer().map(|p| p.node_name.as_str()), Some("node-worker-01"));

    // Navigate rows down
    state.select_next();
    assert_eq!(state.selected_peer_idx, 1);
    assert_eq!(state.selected_peer().map(|p| p.node_name.as_str()), Some("node-worker-02"));

    state.select_next();
    assert_eq!(state.selected_peer_idx, 2);
    assert_eq!(state.selected_peer().map(|p| p.node_name.as_str()), Some("node-worker-03"));

    // Clamped at end
    state.select_next();
    assert_eq!(state.selected_peer_idx, 2);

    // Navigate up
    state.select_prev();
    assert_eq!(state.selected_peer_idx, 1);

    // Select first and last
    state.select_last();
    assert_eq!(state.selected_peer_idx, 2);
    state.select_first();
    assert_eq!(state.selected_peer_idx, 0);

    // Cycle tabs
    state.next_tab();
    assert_eq!(state.active_tab, BgpTab::Services);
    assert_eq!(state.selected_service().map(|s| s.service_name.as_str()), Some("ingress-nginx-lb"));

    state.next_tab();
    assert_eq!(state.active_tab, BgpTab::IpPools);
    assert_eq!(state.selected_pool().map(|p| p.name.as_str()), Some("public-pool"));

    state.next_tab();
    assert_eq!(state.active_tab, BgpTab::Peers);

    state.prev_tab();
    assert_eq!(state.active_tab, BgpTab::IpPools);
}

#[test]
fn bgp_view_search_and_filtering() {
    let mut state = BgpViewState::new();
    state.set_summary(sample_bgp_summary());

    // Search for worker-03
    state.search_query = "worker-03".to_string();
    let peers = state.filtered_peers();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].node_name, "node-worker-03");

    // Search for ASN 65001
    state.search_query = "65001".to_string();
    let peers = state.filtered_peers();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].peer_address, "10.0.1.254");

    // Switch to services tab and search for ingress
    state.active_tab = BgpTab::Services;
    state.search_query = "ingress".to_string();
    let svcs = state.filtered_services();
    assert_eq!(svcs.len(), 1);
    assert_eq!(svcs[0].service_name, "ingress-nginx-lb");

    // Switch to IP pools tab and search for internal
    state.active_tab = BgpTab::IpPools;
    state.search_query = "internal".to_string();
    let pools = state.filtered_pools();
    assert_eq!(pools.len(), 1);
    assert_eq!(pools[0].name, "internal-pool");

    // Clear filter
    state.search_query.clear();
    state.clamp_selection();
    assert_eq!(state.filtered_pools().len(), 2);
}

#[test]
fn bgp_view_renders_services_and_ip_pools_tabs() {
    let mut state = BgpViewState::new();
    state.set_summary(sample_bgp_summary());

    // Render Services Tab
    state.active_tab = BgpTab::Services;
    let lines = render_lines(140, 24, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");
    assert!(text.contains("ingress-nginx-lb"));
    assert!(text.contains("192.168.100.1"));
    assert!(text.contains("public-pool"));
    assert!(text.contains("api-gateway"));
    assert!(text.contains("192.168.100.2"));

    // Render IP Pools Tab
    state.active_tab = BgpTab::IpPools;
    let lines = render_lines(140, 24, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");
    assert!(text.contains("public-pool"));
    assert!(text.contains("192.168.100.0/24"));
    assert!(text.contains("Active"));
    assert!(text.contains("internal-pool"));
    assert!(text.contains("10.200.0.0/16"));
    assert!(text.contains("Disabled"));
}

#[test]
fn bgp_command_resolution_and_deep_links() {
    // Check command aliases
    assert_eq!(
        resolve_command(":bgp"),
        Some(CommandTarget::Resource(ResourceKind::BgpPeers))
    );
    assert_eq!(
        resolve_command(":peers"),
        Some(CommandTarget::Resource(ResourceKind::BgpPeers))
    );
    assert_eq!(
        resolve_command(":bgppeers"),
        Some(CommandTarget::Resource(ResourceKind::BgpPeers))
    );
    assert_eq!(
        resolve_command(":peering"),
        Some(CommandTarget::Resource(ResourceKind::BgpPeers))
    );
    assert_eq!(
        resolve_command(":bgproutes"),
        Some(CommandTarget::Resource(ResourceKind::BgpPeers))
    );

    // Check deep link parsing and generation
    let link = DeepLink::parse("srelens://view/prod-cluster/_/bgp").expect("parse deep link");
    match &link {
        DeepLink::View { context, namespace, target } => {
            assert_eq!(context.as_deref(), Some("prod-cluster"));
            assert_eq!(namespace, &None);
            assert_eq!(target, &CommandTarget::Resource(ResourceKind::BgpPeers));
        }
        _ => panic!("Expected DeepLink::View"),
    }

    assert_eq!(link.to_url(), "srelens://view/prod-cluster/_/bgp");
}

#[test]
fn bgp_view_renders_cleanly_on_constrained_viewports() {
    let mut state = BgpViewState::new();
    state.set_summary(sample_bgp_summary());

    // Very small terminal dimensions: 20x4 (less than min size 10x5)
    let lines = render_lines(20, 4, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    assert_eq!(lines.len(), 4);

    // Narrow terminal: 50x15
    let lines = render_lines(50, 15, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");
    assert!(text.contains("BGP"));
    assert!(text.contains("node-worker-01"));
}

#[test]
fn bgp_view_empty_peer_table_onboarding_guidance() {
    let mut state = BgpViewState::new();
    let mut summary = sample_bgp_summary();
    summary.peers.clear();
    state.set_summary(summary);

    let lines = render_lines(120, 24, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");
    assert!(text.contains("No BGP peering sessions detected"));
    assert!(text.contains("Deploy CiliumBGPPeeringPolicy"));
}

#[test]
fn bgp_view_footer_inspector_with_missing_optional_fields() {
    let mut state = BgpViewState::new();
    let mut summary = sample_bgp_summary();
    summary.peers[0].hold_time_seconds = None;
    summary.peers[0].keepalive_time_seconds = None;
    summary.peers[0].multihop_ttl = None;
    summary.peers[0].advertised_prefixes.clear();
    state.set_summary(summary);

    let lines = render_lines(140, 24, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");
    assert!(text.contains("90s (default)"));
    assert!(text.contains("30s (default)"));
    assert!(text.contains("Disabled (direct L2)"));
    assert!(text.contains("None"));
}

#[test]
fn bgp_view_dynamic_column_widths_and_uptime_formatting() {
    use srelens_tui::views::bgp_view::format_uptime_display;

    // Test format_uptime_display
    assert_eq!(format_uptime_display(None), "-");
    assert_eq!(format_uptime_display(Some("")), "-");
    assert_eq!(format_uptime_display(Some("344h38m2s")), "344h38m2s");
    assert_eq!(format_uptime_display(Some("15m")), "15m");

    let past_iso = "2026-09-02T09:58:52Z";
    let formatted_uptime = format_uptime_display(Some(past_iso));
    assert!(!formatted_uptime.is_empty() && formatted_uptime != "-");

    // Test wide render with long node name and long policy name
    let mut summary = sample_bgp_summary();
    summary.peers[0].node_name = "data-processing-stage-general-ko0sx".to_string();
    summary.peers[0].policy_name = "cilium-bgp-fabric-peering".to_string();
    summary.peers[0].uptime_or_last_change = Some(past_iso.to_string());

    let mut state = BgpViewState::new();
    state.set_summary(summary);

    // Wide terminal 200x30
    let lines = render_lines(200, 30, |f| {
        render_bgp_view(f, f.area(), &state);
    });
    let text = lines.join("\n");

    // Must contain the full long node name without any truncation cuts
    assert!(text.contains("data-processing-stage-general-ko0sx"));
    // Must contain the full long policy name without any truncation cuts
    assert!(text.contains("cilium-bgp-fabric-peering"));
    // Must contain the formatted relative uptime
    assert!(text.contains(&formatted_uptime));
}


