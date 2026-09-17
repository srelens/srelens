use k8s_openapi::api::core::v1::Node;
use kube::api::{Api, ApiResource, DynamicObject, ListParams};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

/// BGP control plane backend engine detected in the Kubernetes cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BgpEngineType {
    CiliumV2,
    CiliumV2Alpha1,
    MetalLB,
    Calico,
    None,
}

impl std::fmt::Display for BgpEngineType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CiliumV2 => write!(f, "Cilium BGP (v2)"),
            Self::CiliumV2Alpha1 => write!(f, "Cilium BGP (v2alpha1)"),
            Self::MetalLB => write!(f, "MetalLB BGP"),
            Self::Calico => write!(f, "Calico BGP"),
            Self::None => write!(f, "No BGP Engine Detected"),
        }
    }
}

/// BGP session state according to RFC 4271 / Cilium / MetalLB / Calico.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BgpSessionState {
    Established,
    Active,
    Connect,
    Idle,
    OpenSent,
    OpenConfirm,
    Configured,
    Unknown,
}

impl std::fmt::Display for BgpSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Established => write!(f, "Established"),
            Self::Active => write!(f, "Active"),
            Self::Connect => write!(f, "Connect"),
            Self::Idle => write!(f, "Idle"),
            Self::OpenSent => write!(f, "OpenSent"),
            Self::OpenConfirm => write!(f, "OpenConfirm"),
            Self::Configured => write!(f, "Configured"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

impl BgpSessionState {
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim().to_lowercase();
        if trimmed.contains("established") || trimmed == "up" {
            Self::Established
        } else if trimmed.contains("active") {
            Self::Active
        } else if trimmed.contains("connect") {
            Self::Connect
        } else if trimmed.contains("idle") || trimmed == "down" {
            Self::Idle
        } else if trimmed.contains("opensent") {
            Self::OpenSent
        } else if trimmed.contains("openconfirm") {
            Self::OpenConfirm
        } else if trimmed.contains("configured") || trimmed == "ready" {
            Self::Configured
        } else {
            Self::Unknown
        }
    }
}

/// One BGP neighbor session configured or active on a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BgpNeighbor {
    pub node_name: String,
    pub peer_address: String,
    pub peer_asn: u32,
    pub local_asn: u32,
    pub session_state: BgpSessionState,
    pub policy_name: String,
    #[serde(default)]
    pub policy_kind: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub export_pod_cidr: bool,
    pub hold_time_seconds: Option<u64>,
    pub keepalive_time_seconds: Option<u64>,
    pub connect_retry_seconds: Option<u64>,
    pub multihop_ttl: Option<u32>,
    pub graceful_restart: bool,
    pub advertised_prefixes: Vec<String>,
    pub routes_count: usize,
    #[serde(default)]
    pub routes_received: usize,
    pub uptime_or_last_change: Option<String>,
}

/// A Kubernetes Service whose LoadBalancer VIP is advertised via BGP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BgpAdvertisedService {
    pub service_name: String,
    pub namespace: String,
    pub load_balancer_ip: String,
    pub ip_pool: Option<String>,
    pub announcing_nodes: Vec<String>,
    pub peers: Vec<String>,
    pub service_type: String,
}

/// A Cilium or MetalLB LoadBalancer IP Pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BgpIpPool {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub cidrs: Vec<String>,
    pub service_selector: String,
    pub disabled: bool,
}

/// Overall cluster BGP peering topology and route summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BgpClusterSummary {
    pub engine: BgpEngineType,
    pub total_nodes: usize,
    pub bgp_nodes: usize,
    pub total_peers: usize,
    pub established_peers: usize,
    pub degraded_peers: usize,
    pub peers: Vec<BgpNeighbor>,
    pub advertised_services: Vec<BgpAdvertisedService>,
    pub ip_pools: Vec<BgpIpPool>,
    pub error: Option<String>,
}

impl Default for BgpClusterSummary {
    fn default() -> Self {
        Self {
            engine: BgpEngineType::None,
            total_nodes: 0,
            bgp_nodes: 0,
            total_peers: 0,
            established_peers: 0,
            degraded_peers: 0,
            peers: Vec::new(),
            advertised_services: Vec::new(),
            ip_pools: Vec::new(),
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ApiResource Definitions
// ---------------------------------------------------------------------------

pub fn cilium_bgp_peering_policy_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2alpha1".to_string(),
        api_version: "cilium.io/v2alpha1".to_string(),
        kind: "CiliumBGPPeeringPolicy".to_string(),
        plural: "ciliumbgppeeringpolicies".to_string(),
    }
}

pub fn cilium_bgp_cluster_config_v2_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2".to_string(),
        api_version: "cilium.io/v2".to_string(),
        kind: "CiliumBGPClusterConfig".to_string(),
        plural: "ciliumbgpclusterconfigs".to_string(),
    }
}

pub fn cilium_bgp_cluster_config_v2alpha1_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2alpha1".to_string(),
        api_version: "cilium.io/v2alpha1".to_string(),
        kind: "CiliumBGPClusterConfig".to_string(),
        plural: "ciliumbgpclusterconfigs".to_string(),
    }
}

pub fn cilium_bgp_peer_config_v2_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2".to_string(),
        api_version: "cilium.io/v2".to_string(),
        kind: "CiliumBGPPeerConfig".to_string(),
        plural: "ciliumbgppeerconfigs".to_string(),
    }
}

pub fn cilium_bgp_peer_config_v2alpha1_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2alpha1".to_string(),
        api_version: "cilium.io/v2alpha1".to_string(),
        kind: "CiliumBGPPeerConfig".to_string(),
        plural: "ciliumbgppeerconfigs".to_string(),
    }
}

pub fn cilium_bgp_advertisement_v2_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2".to_string(),
        api_version: "cilium.io/v2".to_string(),
        kind: "CiliumBGPAdvertisement".to_string(),
        plural: "ciliumbgpadvertisements".to_string(),
    }
}

pub fn cilium_bgp_advertisement_v2alpha1_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2alpha1".to_string(),
        api_version: "cilium.io/v2alpha1".to_string(),
        kind: "CiliumBGPAdvertisement".to_string(),
        plural: "ciliumbgpadvertisements".to_string(),
    }
}

pub fn cilium_bgp_node_config_v2_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2".to_string(),
        api_version: "cilium.io/v2".to_string(),
        kind: "CiliumBGPNodeConfig".to_string(),
        plural: "ciliumbgpnodeconfigs".to_string(),
    }
}

pub fn cilium_bgp_node_config_v2alpha1_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2alpha1".to_string(),
        api_version: "cilium.io/v2alpha1".to_string(),
        kind: "CiliumBGPNodeConfig".to_string(),
        plural: "ciliumbgpnodeconfigs".to_string(),
    }
}

pub fn cilium_bgp_node_config_override_v2_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2".to_string(),
        api_version: "cilium.io/v2".to_string(),
        kind: "CiliumBGPNodeConfigOverride".to_string(),
        plural: "ciliumbgpnodeconfigoverrides".to_string(),
    }
}

pub fn cilium_load_balancer_ip_pool_v2alpha1_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2alpha1".to_string(),
        api_version: "cilium.io/v2alpha1".to_string(),
        kind: "CiliumLoadBalancerIPPool".to_string(),
        plural: "ciliumloadbalancerippools".to_string(),
    }
}

pub fn cilium_load_balancer_ip_pool_v2_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2".to_string(),
        api_version: "cilium.io/v2".to_string(),
        kind: "CiliumLoadBalancerIPPool".to_string(),
        plural: "ciliumloadbalancerippools".to_string(),
    }
}

pub fn cilium_node_resource() -> ApiResource {
    ApiResource {
        group: "cilium.io".to_string(),
        version: "v2".to_string(),
        api_version: "cilium.io/v2".to_string(),
        kind: "CiliumNode".to_string(),
        plural: "ciliumnodes".to_string(),
    }
}

pub fn metallb_bgp_peer_resource() -> ApiResource {
    ApiResource {
        group: "metallb.io".to_string(),
        version: "v1beta2".to_string(),
        api_version: "metallb.io/v1beta2".to_string(),
        kind: "BGPPeer".to_string(),
        plural: "bgppeers".to_string(),
    }
}

pub fn metallb_ip_pool_resource() -> ApiResource {
    ApiResource {
        group: "metallb.io".to_string(),
        version: "v1beta1".to_string(),
        api_version: "metallb.io/v1beta1".to_string(),
        kind: "IPAddressPool".to_string(),
        plural: "ipaddresspools".to_string(),
    }
}

pub fn calico_bgp_peer_resource() -> ApiResource {
    ApiResource {
        group: "crd.projectcalico.org".to_string(),
        version: "v1".to_string(),
        api_version: "crd.projectcalico.org/v1".to_string(),
        kind: "BGPPeer".to_string(),
        plural: "bgppeers".to_string(),
    }
}

fn request_timeout() -> Duration {
    Duration::from_secs(5)
}

async fn list_dynamic_resource(
    client: &kube::Client,
    res1: &ApiResource,
    res2: &ApiResource,
) -> Vec<DynamicObject> {
    let api1: Api<DynamicObject> = Api::all_with(client.clone(), res1);
    if let Ok(Ok(list)) =
        tokio::time::timeout(request_timeout(), api1.list(&ListParams::default())).await
    {
        if !list.items.is_empty() {
            return list.items;
        }
    }
    let api2: Api<DynamicObject> = Api::all_with(client.clone(), res2);
    if let Ok(Ok(list)) =
        tokio::time::timeout(request_timeout(), api2.list(&ListParams::default())).await
    {
        return list.items;
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Main Discovery Pipeline
// ---------------------------------------------------------------------------

/// Fetch and correlate BGP peering topology and route advertisements across the cluster.
pub async fn fetch_bgp_summary(client: &kube::Client) -> Result<BgpClusterSummary, String> {
    // 1. Fetch cluster nodes for node selector matching & IPAM podCIDR extraction
    let nodes_api: Api<Node> = Api::all(client.clone());
    let nodes_list =
        tokio::time::timeout(request_timeout(), nodes_api.list(&ListParams::default()))
            .await
            .map_err(|_| "Node fetch timed out".to_string())?
            .map_err(|e| format!("Fetch nodes: {}", e))?;

    let total_nodes = nodes_list.items.len();
    let mut node_pod_cidrs: HashMap<String, Vec<String>> = HashMap::new();
    let mut node_labels_map: HashMap<String, BTreeMap<String, String>> = HashMap::new();

    for node in &nodes_list.items {
        let name = node.metadata.name.clone().unwrap_or_default();
        let mut cidrs = Vec::new();
        if let Some(ref spec) = node.spec {
            if let Some(ref c) = spec.pod_cidr {
                cidrs.push(c.clone());
            }
            if let Some(ref list) = spec.pod_cidrs {
                for item in list {
                    if !cidrs.contains(item) {
                        cidrs.push(item.clone());
                    }
                }
            }
        }
        node_pod_cidrs.insert(name.clone(), cidrs);
        let labels = node.metadata.labels.clone().unwrap_or_default();
        node_labels_map.insert(name, labels);
    }

    // 2. Fetch LoadBalancer Services
    let svc_api: Api<k8s_openapi::api::core::v1::Service> = Api::all(client.clone());
    let svc_list = tokio::time::timeout(request_timeout(), svc_api.list(&ListParams::default()))
        .await
        .map_err(|_| "Service fetch timed out".to_string())?
        .map_err(|e| format!("Fetch services: {}", e))?;

    let mut lb_services: Vec<(String, String, String, Option<String>)> = Vec::new();
    for svc in &svc_list.items {
        if let Some(ref spec) = svc.spec {
            if spec.type_.as_deref() == Some("LoadBalancer") {
                let s_name = svc.metadata.name.clone().unwrap_or_default();
                let s_ns = svc
                    .metadata
                    .namespace
                    .clone()
                    .unwrap_or_else(|| "default".to_string());
                let ip_pool_ann = svc.metadata.annotations.as_ref().and_then(|a| {
                    a.get("cilium.io/lb-ipam-ips")
                        .or_else(|| a.get("io.cilium/lb-ipam-ips"))
                        .or_else(|| a.get("lbipam.cilium.io/ips"))
                        .or_else(|| a.get("metallb.universe.tf/address-pool"))
                        .or_else(|| a.get("metallb.io/address-pool"))
                        .cloned()
                });

                let mut lb_ip = String::new();
                if let Some(ref status) = svc.status {
                    if let Some(ref lb) = status.load_balancer {
                        if let Some(ref ing_list) = lb.ingress {
                            if let Some(first) = ing_list.first() {
                                if let Some(ref ip) = first.ip {
                                    lb_ip = ip.clone();
                                } else if let Some(ref host) = first.hostname {
                                    lb_ip = host.clone();
                                }
                            }
                        }
                    }
                }
                if lb_ip.is_empty() {
                    if let Some(ref ip) = spec.load_balancer_ip {
                        lb_ip = ip.clone();
                    }
                }
                if !lb_ip.is_empty() {
                    lb_services.push((s_name, s_ns, lb_ip, ip_pool_ann));
                }
            }
        }
    }

    // 3. Try Cilium BGP Control Plane (v2 / v2alpha1)
    if let Ok(cilium_summary) =
        discover_cilium_bgp(client, &node_labels_map, &node_pod_cidrs, &lb_services).await
    {
        if cilium_summary.engine != BgpEngineType::None
            && (!cilium_summary.peers.is_empty() || !cilium_summary.ip_pools.is_empty())
        {
            let mut res = cilium_summary;
            res.total_nodes = total_nodes;
            return Ok(res);
        }
    }

    // 4. Fallback to MetalLB
    if let Ok(metallb_summary) = discover_metallb_bgp(client, &node_labels_map, &lb_services).await
    {
        if metallb_summary.engine != BgpEngineType::None
            && (!metallb_summary.peers.is_empty() || !metallb_summary.ip_pools.is_empty())
        {
            let mut res = metallb_summary;
            res.total_nodes = total_nodes;
            return Ok(res);
        }
    }

    // 5. Fallback to Calico
    if let Ok(calico_summary) = discover_calico_bgp(client, &node_labels_map).await {
        if calico_summary.engine != BgpEngineType::None && !calico_summary.peers.is_empty() {
            let mut res = calico_summary;
            res.total_nodes = total_nodes;
            return Ok(res);
        }
    }

    Ok(BgpClusterSummary {
        engine: BgpEngineType::None,
        total_nodes,
        bgp_nodes: 0,
        total_peers: 0,
        established_peers: 0,
        degraded_peers: 0,
        peers: Vec::new(),
        advertised_services: Vec::new(),
        ip_pools: Vec::new(),
        error: None,
    })
}

// ---------------------------------------------------------------------------
// Cilium Discovery Engine (v2 & v2alpha1)
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Debug)]
struct CiliumPeerConfigData {
    hold_time: Option<u64>,
    keepalive: Option<u64>,
    connect_retry: Option<u64>,
    multihop: Option<u32>,
    graceful_restart: bool,
}

#[derive(Debug, Clone)]
struct LiveBgpPeerInfo {
    peer_address: String,
    peer_asn: u32,
    local_asn: u32,
    session_state: BgpSessionState,
    uptime: Option<String>,
    routes_advertised: usize,
    routes_received: usize,
    peer_name: String,
}

async fn discover_cilium_bgp(
    client: &kube::Client,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
    node_pod_cidrs: &HashMap<String, Vec<String>>,
    lb_services: &[(String, String, String, Option<String>)],
) -> Result<BgpClusterSummary, String> {
    let mut neighbors: Vec<BgpNeighbor> = Vec::new();
    let mut ip_pools: Vec<BgpIpPool> = Vec::new();
    let mut bgp_node_set: BTreeSet<String> = BTreeSet::new();
    let mut is_cilium_v2 = false;
    let mut is_cilium_v2alpha1 = false;

    // A. Load CiliumLoadBalancerIPPool (v2 / v2alpha1)
    let pool_items = list_dynamic_resource(
        client,
        &cilium_load_balancer_ip_pool_v2_resource(),
        &cilium_load_balancer_ip_pool_v2alpha1_resource(),
    )
    .await;

    for obj in pool_items {
        is_cilium_v2 = true;
        let name = obj.metadata.name.clone().unwrap_or_default();
        let mut cidrs = Vec::new();
        let mut disabled = false;
        let mut svc_sel = String::new();

        if let Some(spec) = obj.data.get("spec") {
            if let Some(c_list) = spec
                .get("cidrs")
                .or_else(|| spec.get("blocks"))
                .and_then(|v| v.as_array())
            {
                for c in c_list {
                    if let Some(s) = c.as_str() {
                        cidrs.push(s.to_string());
                    } else if let Some(s) = c.get("cidr").and_then(|v| v.as_str()) {
                        cidrs.push(s.to_string());
                    }
                }
            }
            if let Some(d) = spec.get("disabled").and_then(|v| v.as_bool()) {
                disabled = d;
            }
            if let Some(sel) = spec.get("serviceSelector") {
                svc_sel = serde_json::to_string(sel).unwrap_or_default();
            }
        }
        ip_pools.push(BgpIpPool {
            name,
            namespace: None,
            cidrs,
            service_selector: svc_sel,
            disabled,
        });
    }

    // B. Load CiliumBGPPeerConfig (v2 / v2alpha1)
    let mut peer_configs: HashMap<String, CiliumPeerConfigData> = HashMap::new();
    let peer_cfg_items = list_dynamic_resource(
        client,
        &cilium_bgp_peer_config_v2_resource(),
        &cilium_bgp_peer_config_v2alpha1_resource(),
    )
    .await;

    for obj in peer_cfg_items {
        is_cilium_v2 = true;
        let name = obj.metadata.name.clone().unwrap_or_default();
        let mut cfg = CiliumPeerConfigData::default();
        if let Some(spec) = obj.data.get("spec") {
            if let Some(timers) = spec.get("timers") {
                cfg.hold_time = timers.get("holdTimeSeconds").and_then(|v| v.as_u64());
                cfg.keepalive = timers.get("keepAliveTimeSeconds").and_then(|v| v.as_u64());
                cfg.connect_retry = timers
                    .get("connectRetryTimeSeconds")
                    .and_then(|v| v.as_u64());
            }
            cfg.multihop = spec
                .get("ebgpMultihop")
                .or_else(|| spec.get("ebgpMultihopTTL"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            cfg.graceful_restart = spec
                .get("gracefulRestart")
                .and_then(|g| g.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        }
        peer_configs.insert(name, cfg);
    }

    // C. Load CiliumBGPAdvertisement (v2 / v2alpha1)
    let mut export_pod_cidr_default = true;
    let adv_items = list_dynamic_resource(
        client,
        &cilium_bgp_advertisement_v2_resource(),
        &cilium_bgp_advertisement_v2alpha1_resource(),
    )
    .await;
    if !adv_items.is_empty() {
        is_cilium_v2 = true;
        export_pod_cidr_default = adv_items.iter().any(|adv| {
            if let Some(arr) = adv
                .data
                .get("spec")
                .and_then(|s| s.get("advertisements"))
                .and_then(|v| v.as_array())
            {
                arr.iter().any(|item| {
                    item.get("advertisementType")
                        .and_then(|v| v.as_str())
                        .map_or(false, |t| {
                            t.eq_ignore_ascii_case("podcidr") || t.eq_ignore_ascii_case("pod")
                        })
                })
            } else {
                true
            }
        });
    }

    // D. Load CiliumBGPClusterConfig (v2 / v2alpha1)
    let cluster_cfg_items = list_dynamic_resource(
        client,
        &cilium_bgp_cluster_config_v2_resource(),
        &cilium_bgp_cluster_config_v2alpha1_resource(),
    )
    .await;

    for obj in cluster_cfg_items {
        is_cilium_v2 = true;
        let pol_name = obj
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "cilium-bgp-cluster-config".to_string());
        let spec = match obj.data.get("spec") {
            Some(s) => s,
            None => continue,
        };

        let node_selector = spec.get("nodeSelector");
        let matching_nodes: Vec<String> = find_matching_nodes(node_selector, node_labels);

        if let Some(instances) = spec
            .get("bgpInstances")
            .or_else(|| spec.get("instances"))
            .and_then(|v| v.as_array())
        {
            for instance in instances {
                let local_asn = instance
                    .get("localASN")
                    .or_else(|| instance.get("localAsn"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;

                if let Some(peers) = instance
                    .get("peers")
                    .or_else(|| instance.get("neighbors"))
                    .and_then(|v| v.as_array())
                {
                    for peer in peers {
                        let peer_addr = peer
                            .get("peerAddress")
                            .or_else(|| peer.get("peerIP"))
                            .or_else(|| peer.get("address"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        if peer_addr.is_empty() {
                            continue;
                        }

                        let peer_asn = peer
                            .get("peerASN")
                            .or_else(|| peer.get("peerAsn"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;

                        let peer_cfg_name = peer
                            .get("peerConfigRef")
                            .and_then(|r| r.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let peer_cfg =
                            peer_configs.get(peer_cfg_name).cloned().unwrap_or_else(|| {
                                let mut fallback = CiliumPeerConfigData::default();
                                if let Some(timers) = peer.get("timers") {
                                    fallback.hold_time =
                                        timers.get("holdTimeSeconds").and_then(|v| v.as_u64());
                                    fallback.keepalive =
                                        timers.get("keepAliveTimeSeconds").and_then(|v| v.as_u64());
                                    fallback.connect_retry = timers
                                        .get("connectRetryTimeSeconds")
                                        .and_then(|v| v.as_u64());
                                }
                                fallback
                            });

                        for node in &matching_nodes {
                            bgp_node_set.insert(node.clone());
                            let mut prefixes = Vec::new();
                            if export_pod_cidr_default {
                                if let Some(cidrs) = node_pod_cidrs.get(node) {
                                    for c in cidrs {
                                        prefixes.push(format!("PodCIDR: {}", c));
                                    }
                                }
                            }
                            for (s_name, s_ns, lb_ip, _) in lb_services {
                                prefixes.push(format!("VIP: {} ({}/{})", lb_ip, s_ns, s_name));
                            }
                            let routes_count = prefixes.len();

                            neighbors.push(BgpNeighbor {
                                node_name: node.clone(),
                                peer_address: peer_addr.clone(),
                                peer_asn,
                                local_asn,
                                session_state: BgpSessionState::Configured,
                                policy_name: pol_name.clone(),
                                policy_kind: "CiliumBGPClusterConfig".to_string(),
                                namespace: None,
                                export_pod_cidr: export_pod_cidr_default,
                                hold_time_seconds: peer_cfg.hold_time,
                                keepalive_time_seconds: peer_cfg.keepalive,
                                connect_retry_seconds: peer_cfg.connect_retry,
                                multihop_ttl: peer_cfg.multihop,
                                graceful_restart: peer_cfg.graceful_restart,
                                advertised_prefixes: prefixes,
                                routes_count,
                                routes_received: 0,
                                uptime_or_last_change: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // E. Load CiliumBGPNodeConfig and Override (v2 / v2alpha1)
    let node_cfg_items = list_dynamic_resource(
        client,
        &cilium_bgp_node_config_v2_resource(),
        &cilium_bgp_node_config_v2alpha1_resource(),
    )
    .await;

    for obj in node_cfg_items {
        is_cilium_v2 = true;
        let node_name = obj.metadata.name.clone().unwrap_or_default();
        if let Some(status) = obj.data.get("status") {
            let live_peers = extract_live_bgp_peers(status, 0);
            for live in live_peers {
                bgp_node_set.insert(node_name.clone());
                update_or_insert_neighbor(
                    &mut neighbors,
                    &node_name,
                    live,
                    &node_name,
                    "CiliumBGPNodeConfig",
                    export_pod_cidr_default,
                    node_pod_cidrs,
                    lb_services,
                );
            }
        }
    }

    // F. Load CiliumBGPPeeringPolicy (v2alpha1 Legacy)
    let policy_api: Api<DynamicObject> =
        Api::all_with(client.clone(), &cilium_bgp_peering_policy_resource());
    if let Ok(pol_res) =
        tokio::time::timeout(request_timeout(), policy_api.list(&ListParams::default())).await
    {
        if let Ok(policies) = pol_res {
            if !policies.items.is_empty() {
                is_cilium_v2alpha1 = true;
                for pol in policies.items {
                    let pol_name = pol.metadata.name.clone().unwrap_or_default();
                    let spec = match pol.data.get("spec") {
                        Some(s) => s,
                        None => continue,
                    };
                    let matching_nodes: Vec<String> =
                        find_matching_nodes(spec.get("nodeSelectors"), node_labels);

                    if let Some(routers) = spec.get("virtualRouters").and_then(|v| v.as_array()) {
                        for router in routers {
                            let local_asn =
                                router.get("localASN").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let export_pod_cidr = router
                                .get("exportPodCIDR")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            if let Some(nbrs) = router.get("neighbors").and_then(|v| v.as_array()) {
                                for nbr in nbrs {
                                    let peer_addr = nbr
                                        .get("peerAddress")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let peer_asn =
                                        nbr.get("peerASN").and_then(|v| v.as_u64()).unwrap_or(0)
                                            as u32;
                                    let hold_time =
                                        nbr.get("holdTimeSeconds").and_then(|v| v.as_u64());
                                    let keepalive =
                                        nbr.get("keepAliveTimeSeconds").and_then(|v| v.as_u64());
                                    let connect_retry =
                                        nbr.get("connectRetryTimeSeconds").and_then(|v| v.as_u64());
                                    let multihop = nbr
                                        .get("eBGPMultihopTTL")
                                        .and_then(|v| v.as_u64())
                                        .map(|v| v as u32);
                                    let graceful = nbr
                                        .get("gracefulRestart")
                                        .and_then(|v| v.get("enabled"))
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);

                                    for node in &matching_nodes {
                                        bgp_node_set.insert(node.clone());
                                        let mut prefixes = Vec::new();
                                        if export_pod_cidr {
                                            if let Some(cidrs) = node_pod_cidrs.get(node) {
                                                for c in cidrs {
                                                    prefixes.push(format!("PodCIDR: {}", c));
                                                }
                                            }
                                        }
                                        for (s_name, s_ns, lb_ip, _) in lb_services {
                                            prefixes.push(format!(
                                                "VIP: {} ({}/{})",
                                                lb_ip, s_ns, s_name
                                            ));
                                        }

                                        let routes_count = prefixes.len();
                                        neighbors.push(BgpNeighbor {
                                            node_name: node.clone(),
                                            peer_address: peer_addr.clone(),
                                            peer_asn,
                                            local_asn,
                                            session_state: BgpSessionState::Configured,
                                            policy_name: pol_name.clone(),
                                            policy_kind: "CiliumBGPPeeringPolicy".to_string(),
                                            namespace: None,
                                            export_pod_cidr,
                                            hold_time_seconds: hold_time,
                                            keepalive_time_seconds: keepalive,
                                            connect_retry_seconds: connect_retry,
                                            multihop_ttl: multihop,
                                            graceful_restart: graceful,
                                            advertised_prefixes: prefixes,
                                            routes_count,
                                            routes_received: 0,
                                            uptime_or_last_change: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // G. Load CiliumNode live status for active peering sessions
    let cnode_api: Api<DynamicObject> = Api::all_with(client.clone(), &cilium_node_resource());
    if let Ok(nodes_res) =
        tokio::time::timeout(request_timeout(), cnode_api.list(&ListParams::default())).await
    {
        if let Ok(cnodes) = nodes_res {
            for cnode in cnodes.items {
                let n_name = cnode.metadata.name.clone().unwrap_or_default();
                if let Some(status) = cnode.data.get("status") {
                    let live_peers = extract_live_bgp_peers(status, 0);
                    for live in live_peers {
                        bgp_node_set.insert(n_name.clone());
                        update_or_insert_neighbor(
                            &mut neighbors,
                            &n_name,
                            live,
                            &n_name,
                            "CiliumNode",
                            export_pod_cidr_default,
                            node_pod_cidrs,
                            lb_services,
                        );
                    }
                }
            }
        }
    }

    // Correlate Advertised Services
    let mut advertised_services: Vec<BgpAdvertisedService> = Vec::new();
    let announcing_node_names: Vec<String> = bgp_node_set.iter().cloned().collect();
    let peer_addresses: Vec<String> = neighbors
        .iter()
        .map(|n| format!("{}:{}", n.peer_address, n.peer_asn))
        .collect();

    for (s_name, s_ns, lb_ip, ip_pool_ann) in lb_services {
        advertised_services.push(BgpAdvertisedService {
            service_name: s_name.clone(),
            namespace: s_ns.clone(),
            load_balancer_ip: lb_ip.clone(),
            ip_pool: ip_pool_ann.clone(),
            announcing_nodes: announcing_node_names.clone(),
            peers: peer_addresses.clone(),
            service_type: "LoadBalancer".to_string(),
        });
    }

    neighbors.sort_by(|a, b| {
        a.node_name
            .cmp(&b.node_name)
            .then_with(|| a.peer_address.cmp(&b.peer_address))
            .then_with(|| a.peer_asn.cmp(&b.peer_asn))
    });

    let established = neighbors
        .iter()
        .filter(|n| n.session_state == BgpSessionState::Established)
        .count();
    let degraded = neighbors
        .iter()
        .filter(|n| {
            n.session_state == BgpSessionState::Active
                || n.session_state == BgpSessionState::Connect
                || n.session_state == BgpSessionState::Idle
        })
        .count();

    let engine = if is_cilium_v2 {
        BgpEngineType::CiliumV2
    } else if is_cilium_v2alpha1 {
        BgpEngineType::CiliumV2Alpha1
    } else if !neighbors.is_empty() || !ip_pools.is_empty() {
        BgpEngineType::CiliumV2
    } else {
        BgpEngineType::None
    };

    Ok(BgpClusterSummary {
        engine,
        total_nodes: 0,
        bgp_nodes: bgp_node_set.len(),
        total_peers: neighbors.len(),
        established_peers: established,
        degraded_peers: degraded,
        peers: neighbors,
        advertised_services,
        ip_pools,
        error: None,
    })
}

fn update_or_insert_neighbor(
    neighbors: &mut Vec<BgpNeighbor>,
    node_name: &str,
    live: LiveBgpPeerInfo,
    default_policy: &str,
    default_policy_kind: &str,
    export_pod_cidr: bool,
    node_pod_cidrs: &HashMap<String, Vec<String>>,
    lb_services: &[(String, String, String, Option<String>)],
) {
    if let Some(existing) = neighbors.iter_mut().find(|n| {
        (n.node_name == node_name || n.node_name.is_empty())
            && (n.peer_address == live.peer_address
                || (live.peer_asn > 0
                    && n.peer_asn == live.peer_asn
                    && n.peer_address.contains(&live.peer_address)))
    }) {
        if existing.node_name.is_empty() {
            existing.node_name = node_name.to_string();
        }
        existing.session_state = live.session_state;
        existing.uptime_or_last_change = live.uptime;
        if live.peer_asn > 0 {
            existing.peer_asn = live.peer_asn;
        }
        if live.local_asn > 0 {
            existing.local_asn = live.local_asn;
        }
        if live.routes_advertised > 0 {
            existing.routes_count = live.routes_advertised;
        }
        if live.routes_received > 0 {
            existing.routes_received = live.routes_received;
        }
    } else {
        let mut prefixes = Vec::new();
        if export_pod_cidr {
            if let Some(cidrs) = node_pod_cidrs.get(node_name) {
                for c in cidrs {
                    prefixes.push(format!("PodCIDR: {}", c));
                }
            }
        }
        for (s_name, s_ns, lb_ip, _) in lb_services {
            prefixes.push(format!("VIP: {} ({}/{})", lb_ip, s_ns, s_name));
        }
        let routes_count = if live.routes_advertised > 0 {
            live.routes_advertised
        } else {
            prefixes.len()
        };

        neighbors.push(BgpNeighbor {
            node_name: node_name.to_string(),
            peer_address: live.peer_address,
            peer_asn: live.peer_asn,
            local_asn: live.local_asn,
            session_state: live.session_state,
            policy_name: if !live.peer_name.is_empty() {
                live.peer_name
            } else {
                default_policy.to_string()
            },
            policy_kind: default_policy_kind.to_string(),
            namespace: None,
            export_pod_cidr,
            hold_time_seconds: None,
            keepalive_time_seconds: None,
            connect_retry_seconds: None,
            multihop_ttl: None,
            graceful_restart: false,
            advertised_prefixes: prefixes,
            routes_count,
            routes_received: live.routes_received,
            uptime_or_last_change: live.uptime,
        });
    }
}

fn extract_live_bgp_peers(val: &Value, fallback_local_asn: u32) -> Vec<LiveBgpPeerInfo> {
    let mut out = Vec::new();

    // 1. Array of instances or peers
    if let Some(arr) = val.as_array() {
        for item in arr {
            out.extend(extract_live_bgp_peers(item, fallback_local_asn));
        }
        return out;
    }

    // 2. Object inspection
    if let Some(obj) = val.as_object() {
        // Direct peer item
        if let Some(p) = parse_single_peer_status(val, fallback_local_asn) {
            out.push(p);
            return out;
        }

        let local_asn = obj
            .get("localASN")
            .or_else(|| obj.get("localAsn"))
            .or_else(|| obj.get("local_asn"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(fallback_local_asn);

        for (k, v) in obj {
            let lower_k = k.to_lowercase();
            if lower_k.contains("bgp")
                || lower_k.contains("peer")
                || lower_k.contains("instance")
                || lower_k.contains("neighbor")
            {
                out.extend(extract_live_bgp_peers(v, local_asn));
            }
        }
    }

    out
}

fn parse_single_peer_status(p: &Value, fallback_local_asn: u32) -> Option<LiveBgpPeerInfo> {
    let peer_addr = p
        .get("peerAddress")
        .or_else(|| p.get("peerIP"))
        .or_else(|| p.get("peer_address"))
        .or_else(|| p.get("peer"))
        .or_else(|| p.get("address"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if peer_addr.is_empty() {
        return None;
    }

    let peer_asn = p
        .get("peerASN")
        .or_else(|| p.get("peerAsn"))
        .or_else(|| p.get("peer_asn"))
        .or_else(|| p.get("asn"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let local_asn = p
        .get("localASN")
        .or_else(|| p.get("localAsn"))
        .or_else(|| p.get("local_asn"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(fallback_local_asn);

    let state_str = p
        .get("sessionState")
        .or_else(|| p.get("state"))
        .or_else(|| p.get("session_state"))
        .or_else(|| p.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("Established");

    let state = BgpSessionState::parse(state_str);

    let uptime = p
        .get("uptime")
        .or_else(|| p.get("establishedTime"))
        .or_else(|| p.get("established_time"))
        .or_else(|| p.get("lastChange"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let routes_adv = p
        .get("routesAdvertised")
        .or_else(|| p.get("advertisedRoutes"))
        .or_else(|| p.get("routes_advertised"))
        .or_else(|| p.get("advertised"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let routes_rec = p
        .get("routesReceived")
        .or_else(|| p.get("receivedRoutes"))
        .or_else(|| p.get("routes_received"))
        .or_else(|| p.get("received"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let peer_name = p
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Some(LiveBgpPeerInfo {
        peer_address: peer_addr,
        peer_asn,
        local_asn,
        session_state: state,
        uptime,
        routes_advertised: routes_adv,
        routes_received: routes_rec,
        peer_name,
    })
}

// ---------------------------------------------------------------------------
// Fallbacks: MetalLB & Calico
// ---------------------------------------------------------------------------

async fn discover_metallb_bgp(
    client: &kube::Client,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
    lb_services: &[(String, String, String, Option<String>)],
) -> Result<BgpClusterSummary, String> {
    let peer_api: Api<DynamicObject> = Api::all_with(client.clone(), &metallb_bgp_peer_resource());
    let peers_res = match tokio::time::timeout(
        request_timeout(),
        peer_api.list(&ListParams::default()),
    )
    .await
    {
        Ok(Ok(list)) if !list.items.is_empty() => list,
        _ => return Ok(BgpClusterSummary::default()),
    };

    let mut neighbors = Vec::new();
    let mut bgp_nodes = BTreeSet::new();

    for peer in peers_res.items {
        let p_name = peer.metadata.name.clone().unwrap_or_default();
        let p_ns = peer.metadata.namespace.clone();
        let spec = match peer.data.get("spec") {
            Some(s) => s,
            None => continue,
        };

        let peer_addr = spec
            .get("peerAddress")
            .or_else(|| spec.get("peerIP"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let peer_asn = spec
            .get("peerASN")
            .or_else(|| spec.get("peerAsn"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let local_asn = spec
            .get("myASN")
            .or_else(|| spec.get("localAsn"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let hold_time = spec.get("holdTime").and_then(|v| v.as_u64());

        let matching_nodes = find_matching_nodes(spec.get("nodeSelectors"), node_labels);
        for node in matching_nodes {
            bgp_nodes.insert(node.clone());
            neighbors.push(BgpNeighbor {
                node_name: node,
                peer_address: peer_addr.clone(),
                peer_asn,
                local_asn,
                session_state: BgpSessionState::Configured,
                policy_name: p_name.clone(),
                policy_kind: "BGPPeer".to_string(),
                namespace: p_ns.clone(),
                export_pod_cidr: false,
                hold_time_seconds: hold_time,
                keepalive_time_seconds: None,
                connect_retry_seconds: None,
                multihop_ttl: None,
                graceful_restart: false,
                advertised_prefixes: lb_services
                    .iter()
                    .map(|s| format!("VIP: {}", s.2))
                    .collect(),
                routes_count: lb_services.len(),
                routes_received: 0,
                uptime_or_last_change: None,
            });
        }
    }

    let mut ip_pools = Vec::new();
    let pool_api: Api<DynamicObject> = Api::all_with(client.clone(), &metallb_ip_pool_resource());
    if let Ok(Ok(list)) =
        tokio::time::timeout(request_timeout(), pool_api.list(&ListParams::default())).await
    {
        for pool in list.items {
            let name = pool.metadata.name.clone().unwrap_or_default();
            let pool_ns = pool.metadata.namespace.clone();
            let mut cidrs = Vec::new();
            if let Some(arr) = pool
                .data
                .get("spec")
                .and_then(|s| s.get("addresses"))
                .and_then(|v| v.as_array())
            {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        cidrs.push(s.to_string());
                    }
                }
            }
            ip_pools.push(BgpIpPool {
                name,
                namespace: pool_ns,
                cidrs,
                service_selector: String::new(),
                disabled: false,
            });
        }
    }

    let mut advertised_services = Vec::new();
    let ann_nodes: Vec<String> = bgp_nodes.iter().cloned().collect();
    for (s_name, s_ns, lb_ip, ip_pool_ann) in lb_services {
        advertised_services.push(BgpAdvertisedService {
            service_name: s_name.clone(),
            namespace: s_ns.clone(),
            load_balancer_ip: lb_ip.clone(),
            ip_pool: ip_pool_ann.clone(),
            announcing_nodes: ann_nodes.clone(),
            peers: neighbors.iter().map(|n| n.peer_address.clone()).collect(),
            service_type: "LoadBalancer".to_string(),
        });
    }

    neighbors.sort_by(|a, b| {
        a.node_name
            .cmp(&b.node_name)
            .then_with(|| a.peer_address.cmp(&b.peer_address))
            .then_with(|| a.peer_asn.cmp(&b.peer_asn))
    });

    let count = neighbors.len();
    let established = neighbors
        .iter()
        .filter(|n| n.session_state == BgpSessionState::Established)
        .count();
    Ok(BgpClusterSummary {
        engine: BgpEngineType::MetalLB,
        total_nodes: 0,
        bgp_nodes: bgp_nodes.len(),
        total_peers: count,
        established_peers: established,
        degraded_peers: 0,
        peers: neighbors,
        advertised_services,
        ip_pools,
        error: None,
    })
}

async fn discover_calico_bgp(
    client: &kube::Client,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
) -> Result<BgpClusterSummary, String> {
    let peer_api: Api<DynamicObject> = Api::all_with(client.clone(), &calico_bgp_peer_resource());
    let peers_res = match tokio::time::timeout(
        request_timeout(),
        peer_api.list(&ListParams::default()),
    )
    .await
    {
        Ok(Ok(list)) if !list.items.is_empty() => list,
        _ => return Ok(BgpClusterSummary::default()),
    };

    let mut neighbors = Vec::new();
    let mut bgp_nodes = BTreeSet::new();

    for peer in peers_res.items {
        let p_name = peer.metadata.name.clone().unwrap_or_default();
        let spec = match peer.data.get("spec") {
            Some(s) => s,
            None => continue,
        };

        let peer_ip = spec
            .get("peerIP")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let as_num = spec.get("asNumber").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        for (node_name, _) in node_labels {
            bgp_nodes.insert(node_name.clone());
            neighbors.push(BgpNeighbor {
                node_name: node_name.clone(),
                peer_address: peer_ip.clone(),
                peer_asn: as_num,
                local_asn: 0,
                session_state: BgpSessionState::Configured,
                policy_name: p_name.clone(),
                policy_kind: "BGPPeer".to_string(),
                namespace: None,
                export_pod_cidr: true,
                hold_time_seconds: None,
                keepalive_time_seconds: None,
                connect_retry_seconds: None,
                multihop_ttl: None,
                graceful_restart: false,
                advertised_prefixes: Vec::new(),
                routes_count: 0,
                routes_received: 0,
                uptime_or_last_change: None,
            });
        }
    }

    neighbors.sort_by(|a, b| {
        a.node_name
            .cmp(&b.node_name)
            .then_with(|| a.peer_address.cmp(&b.peer_address))
            .then_with(|| a.peer_asn.cmp(&b.peer_asn))
    });

    let count = neighbors.len();
    let established = neighbors
        .iter()
        .filter(|n| n.session_state == BgpSessionState::Established)
        .count();
    Ok(BgpClusterSummary {
        engine: BgpEngineType::Calico,
        total_nodes: 0,
        bgp_nodes: bgp_nodes.len(),
        total_peers: count,
        established_peers: established,
        degraded_peers: 0,
        peers: neighbors,
        advertised_services: Vec::new(),
        ip_pools: Vec::new(),
        error: None,
    })
}

// ---------------------------------------------------------------------------
// Selector Matching
// ---------------------------------------------------------------------------

fn find_matching_nodes(
    selectors_val: Option<&Value>,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
) -> Vec<String> {
    let mut out = Vec::new();
    let selectors = match selectors_val {
        Some(Value::Array(arr)) if !arr.is_empty() => arr.clone(),
        Some(Value::Object(obj)) if !obj.is_empty() => vec![Value::Object(obj.clone())],
        _ => {
            // If no nodeSelector specified, all nodes match
            return node_labels.keys().cloned().collect();
        }
    };

    for (node_name, labels) in node_labels {
        let mut matched = false;
        for sel in &selectors {
            if matches_node_selector(sel, labels) {
                matched = true;
                break;
            }
        }
        if matched {
            out.push(node_name.clone());
        }
    }
    out
}

fn matches_node_selector(selector: &Value, node_labels: &BTreeMap<String, String>) -> bool {
    if selector.is_null() {
        return true;
    }
    if let Some(obj) = selector.as_object() {
        if obj.is_empty() {
            return true;
        }
        // 1. matchLabels
        if let Some(match_labels) = obj.get("matchLabels").and_then(|v| v.as_object()) {
            for (k, v) in match_labels {
                let v_str = match v.as_str() {
                    Some(s) => s,
                    None => continue,
                };
                if node_labels.get(k).map(|s| s.as_str()) != Some(v_str) {
                    return false;
                }
            }
        }
        // 2. matchExpressions
        if let Some(match_exprs) = obj.get("matchExpressions").and_then(|v| v.as_array()) {
            for expr in match_exprs {
                let key = match expr.get("key").and_then(|v| v.as_str()) {
                    Some(k) => k,
                    None => continue,
                };
                let op = expr
                    .get("operator")
                    .and_then(|v| v.as_str())
                    .unwrap_or("In");
                let values: Vec<&str> = expr
                    .get("values")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect())
                    .unwrap_or_default();

                match op {
                    "Exists" => {
                        if !node_labels.contains_key(key) {
                            return false;
                        }
                    }
                    "DoesNotExist" => {
                        if node_labels.contains_key(key) {
                            return false;
                        }
                    }
                    "In" => match node_labels.get(key) {
                        Some(val) if values.contains(&val.as_str()) => {}
                        _ => return false,
                    },
                    "NotIn" => {
                        if let Some(val) = node_labels.get(key) {
                            if values.contains(&val.as_str()) {
                                return false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    true
}
