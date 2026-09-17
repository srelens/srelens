use srelens_kube::bgp::{
    BgpAdvertisedService, BgpClusterSummary, BgpEngineType, BgpIpPool, BgpNeighbor,
    BgpSessionState,
};

#[test]
fn test_bgp_session_state_parsing() {
    assert_eq!(BgpSessionState::parse("Established"), BgpSessionState::Established);
    assert_eq!(BgpSessionState::parse("established"), BgpSessionState::Established);
    assert_eq!(BgpSessionState::parse("up"), BgpSessionState::Established);
    assert_eq!(BgpSessionState::parse("Active"), BgpSessionState::Active);
    assert_eq!(BgpSessionState::parse("Connect"), BgpSessionState::Connect);
    assert_eq!(BgpSessionState::parse("Idle"), BgpSessionState::Idle);
    assert_eq!(BgpSessionState::parse("down"), BgpSessionState::Idle);
    assert_eq!(BgpSessionState::parse("OpenSent"), BgpSessionState::OpenSent);
    assert_eq!(BgpSessionState::parse("OpenConfirm"), BgpSessionState::OpenConfirm);
    assert_eq!(BgpSessionState::parse("Configured"), BgpSessionState::Configured);
    assert_eq!(BgpSessionState::parse("ready"), BgpSessionState::Configured);
    assert_eq!(BgpSessionState::parse("some_random_string"), BgpSessionState::Unknown);
}

#[test]
fn test_bgp_engine_type_display() {
    assert_eq!(BgpEngineType::CiliumV2.to_string(), "Cilium BGP (v2)");
    assert_eq!(BgpEngineType::CiliumV2Alpha1.to_string(), "Cilium BGP (v2alpha1)");
    assert_eq!(BgpEngineType::MetalLB.to_string(), "MetalLB BGP");
    assert_eq!(BgpEngineType::Calico.to_string(), "Calico BGP");
    assert_eq!(BgpEngineType::None.to_string(), "No BGP Engine Detected");
}

#[test]
fn test_bgp_cluster_summary_default() {
    let summary = BgpClusterSummary::default();
    assert_eq!(summary.engine, BgpEngineType::None);
    assert_eq!(summary.total_nodes, 0);
    assert_eq!(summary.bgp_nodes, 0);
    assert_eq!(summary.total_peers, 0);
    assert!(summary.peers.is_empty());
    assert!(summary.advertised_services.is_empty());
    assert!(summary.ip_pools.is_empty());
}

#[test]
fn test_bgp_neighbor_model() {
    let neighbor = BgpNeighbor {
        node_name: "worker-01".to_string(),
        peer_address: "10.0.0.1".to_string(),
        peer_asn: 64511,
        local_asn: 64512,
        session_state: BgpSessionState::Established,
        policy_name: "tor-rack-1".to_string(),
        policy_kind: "CiliumBGPClusterConfig".to_string(),
        export_pod_cidr: true,
        hold_time_seconds: Some(90),
        keepalive_time_seconds: Some(30),
        connect_retry_seconds: Some(120),
        multihop_ttl: Some(64),
        graceful_restart: true,
        advertised_prefixes: vec!["10.244.0.0/24".to_string(), "192.168.1.100/32".to_string()],
        routes_count: 2,
        routes_received: 316,
        uptime_or_last_change: Some("2d4h".to_string()),
    };

    assert_eq!(neighbor.session_state.to_string(), "Established");
    assert_eq!(neighbor.routes_count, 2);
    assert_eq!(neighbor.routes_received, 316);
    assert!(neighbor.export_pod_cidr);
    assert!(neighbor.graceful_restart);
}

#[test]
fn test_bgp_advertised_service_model() {
    let svc = BgpAdvertisedService {
        service_name: "ingress-gateway".to_string(),
        namespace: "cilium-system".to_string(),
        load_balancer_ip: "192.168.10.50".to_string(),
        ip_pool: Some("public-lb-pool".to_string()),
        announcing_nodes: vec!["worker-01".to_string(), "worker-02".to_string()],
        peers: vec!["10.0.0.1:64511".to_string()],
        service_type: "LoadBalancer".to_string(),
    };

    assert_eq!(svc.service_name, "ingress-gateway");
    assert_eq!(svc.announcing_nodes.len(), 2);
}

#[test]
fn test_bgp_ip_pool_model() {
    let pool = BgpIpPool {
        name: "dmz-pool".to_string(),
        cidrs: vec!["192.168.10.0/24".to_string()],
        service_selector: "io.cilium/pool=dmz".to_string(),
        disabled: false,
    };

    assert_eq!(pool.name, "dmz-pool");
    assert!(!pool.disabled);
    assert_eq!(pool.cidrs[0], "192.168.10.0/24");
}

#[test]
fn test_bgp_neighbor_missing_optional_fields() {
    let neighbor = BgpNeighbor {
        node_name: "worker-sparse".to_string(),
        peer_address: "10.0.0.99".to_string(),
        peer_asn: 65000,
        local_asn: 65000,
        session_state: BgpSessionState::Idle,
        policy_name: String::new(),
        policy_kind: String::new(),
        export_pod_cidr: false,
        hold_time_seconds: None,
        keepalive_time_seconds: None,
        connect_retry_seconds: None,
        multihop_ttl: None,
        graceful_restart: false,
        advertised_prefixes: vec![],
        routes_count: 0,
        routes_received: 0,
        uptime_or_last_change: None,
    };

    assert_eq!(neighbor.session_state, BgpSessionState::Idle);
    assert!(neighbor.hold_time_seconds.is_none());
    assert!(neighbor.advertised_prefixes.is_empty());
}

#[test]
fn test_bgp_summary_serde_roundtrip() {
    let summary = BgpClusterSummary {
        engine: BgpEngineType::CiliumV2Alpha1,
        total_nodes: 2,
        bgp_nodes: 1,
        total_peers: 1,
        established_peers: 1,
        degraded_peers: 0,
        peers: vec![BgpNeighbor {
            node_name: "node-1".to_string(),
            peer_address: "172.16.0.1".to_string(),
            peer_asn: 65530,
            local_asn: 65531,
            session_state: BgpSessionState::Established,
            policy_name: "cilium-bgp-peering".to_string(),
            policy_kind: "CiliumBGPPeeringPolicy".to_string(),
            export_pod_cidr: true,
            hold_time_seconds: Some(180),
            keepalive_time_seconds: Some(60),
            connect_retry_seconds: Some(120),
            multihop_ttl: Some(3),
            graceful_restart: true,
            advertised_prefixes: vec!["10.0.0.0/16".to_string()],
            routes_count: 1,
            routes_received: 500,
            uptime_or_last_change: Some("1d".to_string()),
        }],
        advertised_services: vec![],
        ip_pools: vec![],
        error: None,
    };

    let json = serde_json::to_string(&summary).expect("serialize bgp summary");
    let deserialized: BgpClusterSummary =
        serde_json::from_str(&json).expect("deserialize bgp summary");
    assert_eq!(deserialized, summary);
}


