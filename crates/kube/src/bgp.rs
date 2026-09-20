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
    /// The `apiVersion` (`group/version`) of the object named by
    /// `policy_name`, when discovery pinned one. Two operators may ship the
    /// same kind — MetalLB and Calico both have a `BGPPeer` — and resolving a
    /// drill-down by kind name alone lands on whichever the static table
    /// happens to list. Empty means discovery did not pin a version and the
    /// name-only resolution is correct for this kind.
    #[serde(default)]
    pub policy_api_version: String,
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

/// A LoadBalancer Service the cluster might advertise. Its labels travel with
/// it because a `CiliumBGPAdvertisement` selects Services by label, and
/// whether a Service is advertised is the whole question the Services tab
/// answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LbService {
    pub name: String,
    pub namespace: String,
    pub load_balancer_ip: String,
    pub ip_pool: Option<String>,
    pub labels: BTreeMap<String, String>,
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
    /// Why discovery could not see everything: a refused list, a timeout, an
    /// unreachable server. `engine: None` with no error is the cluster
    /// answering that it runs no BGP control plane; `engine: None` *with* an
    /// error is discovery failing, and the two must not read alike.
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

/// List one CRD.
///
/// `Ok` means the API server answered: either with objects, or — for a CRD
/// this cluster does not have installed, which answers `404` — with none.
/// `Err` means it did not answer: the request was refused, timed out, or the
/// server was unreachable. Those two must never render as one sentence
/// (AGENTS.md, "Say what you know, not what you guess"): a caller who cannot
/// read `CiliumBGPClusterConfig` has learned nothing about the cluster's BGP,
/// and reporting "no BGP engine" would be a confident, wrong claim.
async fn list_one_resource(
    client: &kube::Client,
    res: &ApiResource,
) -> Result<Vec<DynamicObject>, String> {
    let api: Api<DynamicObject> = Api::all_with(client.clone(), res);
    match tokio::time::timeout(request_timeout(), api.list(&ListParams::default())).await {
        Ok(Ok(list)) => Ok(list.items),
        // 404 is this cluster saying it serves no such resource.
        Ok(Err(kube::Error::Api(e))) if e.code == 404 => Ok(Vec::new()),
        Ok(Err(e)) => Err(format!("list {}: {}", res.kind, e)),
        Err(_) => Err(format!("list {} timed out", res.kind)),
    }
}

/// List a CRD that exists under two API versions, preferring whichever
/// version has objects. The lookup only fails when *neither* version could be
/// read — one version answering `404` while the other serves objects is the
/// normal shape of a cluster that has upgraded.
async fn list_dynamic_resource(
    client: &kube::Client,
    res1: &ApiResource,
    res2: &ApiResource,
) -> Result<Vec<DynamicObject>, String> {
    let first = list_one_resource(client, res1).await;
    if matches!(&first, Ok(items) if !items.is_empty()) {
        return first;
    }
    let second = list_one_resource(client, res2).await;
    match (first, second) {
        (_, Ok(items)) if !items.is_empty() => Ok(items),
        (Ok(items), _) => Ok(items),
        (Err(_), Ok(items)) => Ok(items),
        (Err(e), Err(_)) => Err(e),
    }
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

    let mut lb_services: Vec<LbService> = Vec::new();
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
                    lb_services.push(LbService {
                        name: s_name,
                        namespace: s_ns,
                        load_balancer_ip: lb_ip,
                        ip_pool: ip_pool_ann,
                        labels: svc.metadata.labels.clone().unwrap_or_default(),
                    });
                }
            }
        }
    }

    // A lookup that was refused or timed out is not an answer about the
    // cluster. Each engine's failures are kept so that, if no engine is found,
    // the summary can say discovery failed rather than claim there is none.
    let mut failures: Vec<String> = Vec::new();

    // 3. Try Cilium BGP Control Plane (v2 / v2alpha1)
    match discover_cilium_bgp(client, &node_labels_map, &node_pod_cidrs, &lb_services).await {
        Ok(cilium_summary) => {
            if cilium_summary.engine != BgpEngineType::None
                && (!cilium_summary.peers.is_empty() || !cilium_summary.ip_pools.is_empty())
            {
                let mut res = cilium_summary;
                res.total_nodes = total_nodes;
                return Ok(res);
            }
            if let Some(e) = cilium_summary.error {
                failures.push(e);
            }
        }
        Err(e) => failures.push(e),
    }

    // 4. Fallback to MetalLB
    match discover_metallb_bgp(client, &node_labels_map, &lb_services).await {
        Ok(metallb_summary) => {
            if metallb_summary.engine != BgpEngineType::None
                && (!metallb_summary.peers.is_empty() || !metallb_summary.ip_pools.is_empty())
            {
                let mut res = metallb_summary;
                res.total_nodes = total_nodes;
                return Ok(res);
            }
            if let Some(e) = metallb_summary.error {
                failures.push(e);
            }
        }
        Err(e) => failures.push(e),
    }

    // 5. Fallback to Calico
    match discover_calico_bgp(client, &node_labels_map).await {
        Ok(calico_summary) => {
            if calico_summary.engine != BgpEngineType::None && !calico_summary.peers.is_empty() {
                let mut res = calico_summary;
                res.total_nodes = total_nodes;
                return Ok(res);
            }
            if let Some(e) = calico_summary.error {
                failures.push(e);
            }
        }
        Err(e) => failures.push(e),
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
        // No engine was found. Whether that is a fact about the cluster or
        // only about what this caller may read is the difference between
        // `None` and a reason here.
        error: join_failures(&failures).map(|why| format!("BGP discovery failed: {}", why)),
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
    lb_services: &[LbService],
) -> Result<BgpClusterSummary, String> {
    // Every lookup that could not be read is remembered rather than collapsed
    // into "this cluster has none of these".
    let mut failures: Vec<String> = Vec::new();
    let mut take = |result: Result<Vec<DynamicObject>, String>| match result {
        Ok(items) => items,
        Err(e) => {
            failures.push(e);
            Vec::new()
        }
    };

    let pool_items = take(
        list_dynamic_resource(
            client,
            &cilium_load_balancer_ip_pool_v2_resource(),
            &cilium_load_balancer_ip_pool_v2alpha1_resource(),
        )
        .await,
    );

    let peer_cfg_items = take(
        list_dynamic_resource(
            client,
            &cilium_bgp_peer_config_v2_resource(),
            &cilium_bgp_peer_config_v2alpha1_resource(),
        )
        .await,
    );

    let adv_items = take(
        list_dynamic_resource(
            client,
            &cilium_bgp_advertisement_v2_resource(),
            &cilium_bgp_advertisement_v2alpha1_resource(),
        )
        .await,
    );

    let cluster_cfg_items = take(
        list_dynamic_resource(
            client,
            &cilium_bgp_cluster_config_v2_resource(),
            &cilium_bgp_cluster_config_v2alpha1_resource(),
        )
        .await,
    );

    let node_cfg_items = take(
        list_dynamic_resource(
            client,
            &cilium_bgp_node_config_v2_resource(),
            &cilium_bgp_node_config_v2alpha1_resource(),
        )
        .await,
    );

    let policy_items = take(list_one_resource(client, &cilium_bgp_peering_policy_resource()).await);
    let cnode_items = take(list_one_resource(client, &cilium_node_resource()).await);

    let mut summary = build_cilium_bgp_summary(
        pool_items,
        peer_cfg_items,
        adv_items,
        cluster_cfg_items,
        node_cfg_items,
        policy_items,
        cnode_items,
        node_labels,
        node_pod_cidrs,
        lb_services,
    );
    summary.error = join_failures(&failures);
    Ok(summary)
}

/// One sentence naming every lookup that could not be read, or `None` when
/// they all answered.
fn join_failures(failures: &[String]) -> Option<String> {
    if failures.is_empty() {
        None
    } else {
        Some(failures.join("; "))
    }
}

pub(crate) fn build_cilium_bgp_summary(
    pool_items: Vec<DynamicObject>,
    peer_cfg_items: Vec<DynamicObject>,
    adv_items: Vec<DynamicObject>,
    cluster_cfg_items: Vec<DynamicObject>,
    node_cfg_items: Vec<DynamicObject>,
    policy_items: Vec<DynamicObject>,
    cnode_items: Vec<DynamicObject>,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
    node_pod_cidrs: &HashMap<String, Vec<String>>,
    lb_services: &[LbService],
) -> BgpClusterSummary {
    let mut neighbors: Vec<BgpNeighbor> = Vec::new();
    let mut ip_pools: Vec<BgpIpPool> = Vec::new();
    let mut bgp_node_set: BTreeSet<String> = BTreeSet::new();
    let mut is_cilium_v2 = false;
    let mut is_cilium_v2alpha1 = false;

    // A. Load CiliumLoadBalancerIPPool (v2 / v2alpha1)
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
    //
    // The advertisements say what the cluster announces. Only the Services
    // they select are advertised: an advertisement carrying just `PodCIDR`
    // announces no VIP at all, and one with a Service selector announces only
    // the Services that selector matches.
    let advertised = advertised_lb_services(&adv_items, lb_services);
    let mut export_pod_cidr_default = true;
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
                            // The advertised VIPs are counted, not copied:
                            // `advertised_services` already carries each one
                            // once, with its announcing nodes and peers. A
                            // copy per neighbour grows as nodes × peers ×
                            // services.
                            let routes_count = prefixes.len() + advertised.len();

                            neighbors.push(BgpNeighbor {
                                node_name: node.clone(),
                                peer_address: peer_addr.clone(),
                                peer_asn,
                                local_asn,
                                session_state: BgpSessionState::Configured,
                                policy_name: pol_name.clone(),
                                policy_kind: "CiliumBGPClusterConfig".to_string(),
                                // Listed under v2 or v2alpha1; the name-only resolver
                                // already probes both for a Cilium kind.
                                policy_api_version: String::new(),
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
                    advertised.len(),
                );
            }
        }
    }

    // F. Load CiliumBGPPeeringPolicy (v2alpha1 Legacy)
    if !policy_items.is_empty() {
        is_cilium_v2alpha1 = true;
        for pol in policy_items {
            let pol_name = pol.metadata.name.clone().unwrap_or_default();
            let spec = match pol.data.get("spec") {
                Some(s) => s,
                None => continue,
            };
            // `CiliumBGPPeeringPolicy` spells this `nodeSelector`, singular.
            // Reading only the plural made every targeted policy look
            // unscoped, and `find_matching_nodes` then put the peer on every
            // node in the cluster. The plural is still accepted so a
            // hand-written manifest with the MetalLB spelling is not silently
            // widened either.
            let matching_nodes: Vec<String> = find_matching_nodes(
                spec.get("nodeSelector")
                    .or_else(|| spec.get("nodeSelectors")),
                node_labels,
            );

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
                                nbr.get("peerASN").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let hold_time = nbr.get("holdTimeSeconds").and_then(|v| v.as_u64());
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
                                // Counted once at the summary level; see the
                                // cluster-config path above.
                                let routes_count = prefixes.len() + advertised.len();
                                neighbors.push(BgpNeighbor {
                                    node_name: node.clone(),
                                    peer_address: peer_addr.clone(),
                                    peer_asn,
                                    local_asn,
                                    session_state: BgpSessionState::Configured,
                                    policy_name: pol_name.clone(),
                                    policy_kind: "CiliumBGPPeeringPolicy".to_string(),
                                    policy_api_version: cilium_bgp_peering_policy_resource()
                                        .api_version
                                        .clone(),
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

    // G. Load CiliumNode live status for active peering sessions
    for cnode in cnode_items {
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
                    advertised.len(),
                );
            }
        }
    }

    // Correlate Advertised Services: only the ones an advertisement selects.
    let mut advertised_services: Vec<BgpAdvertisedService> = Vec::new();
    let announcing_node_names: Vec<String> = bgp_node_set.iter().cloned().collect();
    let peer_addresses: Vec<String> = neighbors
        .iter()
        .map(|n| format!("{}:{}", n.peer_address, n.peer_asn))
        .collect();

    for svc in &advertised {
        advertised_services.push(BgpAdvertisedService {
            service_name: svc.name.clone(),
            namespace: svc.namespace.clone(),
            load_balancer_ip: svc.load_balancer_ip.clone(),
            ip_pool: svc.ip_pool.clone(),
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

    BgpClusterSummary {
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
    }
}

fn update_or_insert_neighbor(
    neighbors: &mut Vec<BgpNeighbor>,
    node_name: &str,
    live: LiveBgpPeerInfo,
    default_policy: &str,
    default_policy_kind: &str,
    export_pod_cidr: bool,
    node_pod_cidrs: &HashMap<String, Vec<String>>,
    // How many LoadBalancer VIPs the cluster advertises. A count, not a list:
    // the VIPs themselves are carried once, by `advertised_services`.
    advertised_vips: usize,
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
        let routes_count = if live.routes_advertised > 0 {
            live.routes_advertised
        } else {
            prefixes.len() + advertised_vips
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
            policy_api_version: String::new(),
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
        .and_then(|v| v.as_str());

    let state = state_str
        .map(BgpSessionState::parse)
        .unwrap_or(BgpSessionState::Unknown);

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
    lb_services: &[LbService],
) -> Result<BgpClusterSummary, String> {
    let mut failures: Vec<String> = Vec::new();

    let peer_items = match list_one_resource(client, &metallb_bgp_peer_resource()).await {
        Ok(items) if items.is_empty() => {
            return Ok(BgpClusterSummary::default());
        }
        Ok(items) => items,
        Err(e) => {
            // The cluster may run MetalLB and simply not let us look.
            return Err(e);
        }
    };

    let pool_items = match list_one_resource(client, &metallb_ip_pool_resource()).await {
        Ok(items) => items,
        Err(e) => {
            failures.push(e);
            Vec::new()
        }
    };

    let mut summary = build_metallb_bgp_summary(peer_items, pool_items, node_labels, lb_services);
    summary.error = join_failures(&failures);
    Ok(summary)
}

pub(crate) fn build_metallb_bgp_summary(
    peer_items: Vec<DynamicObject>,
    pool_items: Vec<DynamicObject>,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
    lb_services: &[LbService],
) -> BgpClusterSummary {
    let mut neighbors = Vec::new();
    let mut bgp_nodes = BTreeSet::new();

    for peer in peer_items {
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
                // Calico ships a `BGPPeer` too, so the kind alone does not
                // identify this row's CRD.
                policy_api_version: metallb_bgp_peer_resource().api_version.clone(),
                namespace: p_ns.clone(),
                export_pod_cidr: false,
                hold_time_seconds: hold_time,
                keepalive_time_seconds: None,
                connect_retry_seconds: None,
                multihop_ttl: None,
                graceful_restart: false,
                // The VIPs are listed once, in `advertised_services`, rather
                // than copied onto every neighbour of every node.
                advertised_prefixes: Vec::new(),
                routes_count: lb_services.len(),
                routes_received: 0,
                uptime_or_last_change: None,
            });
        }
    }

    let mut ip_pools = Vec::new();
    for pool in pool_items {
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

    let mut advertised_services = Vec::new();
    let ann_nodes: Vec<String> = bgp_nodes.iter().cloned().collect();
    for svc in lb_services {
        advertised_services.push(BgpAdvertisedService {
            service_name: svc.name.clone(),
            namespace: svc.namespace.clone(),
            load_balancer_ip: svc.load_balancer_ip.clone(),
            ip_pool: svc.ip_pool.clone(),
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
    BgpClusterSummary {
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
    }
}

async fn discover_calico_bgp(
    client: &kube::Client,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
) -> Result<BgpClusterSummary, String> {
    let peer_items = list_one_resource(client, &calico_bgp_peer_resource()).await?;
    if peer_items.is_empty() {
        return Ok(BgpClusterSummary::default());
    }

    Ok(build_calico_bgp_summary(peer_items, node_labels))
}

pub(crate) fn build_calico_bgp_summary(
    peer_items: Vec<DynamicObject>,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
) -> BgpClusterSummary {
    let mut neighbors = Vec::new();
    let mut bgp_nodes = BTreeSet::new();

    for peer in peer_items {
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

        // A Calico `BGPPeer` scopes itself: `spec.node` names one node,
        // `spec.nodeSelector` selects several, and a peer carrying neither is
        // global. Attributing every peer to every node reported sessions that
        // do not exist and made `total_peers` peers × nodes.
        //
        // Calico may also write `nodeSelector` as one of its own selector
        // expressions (`has(rack)`), which this does not evaluate; such a peer
        // keeps the global reading rather than dropping rows for sessions that
        // may well exist.
        let targets: Vec<String> = match spec.get("node").and_then(|v| v.as_str()) {
            Some(node) if node_labels.contains_key(node) => vec![node.to_string()],
            // A peer pinned to a node this cluster does not have peers nowhere.
            Some(_) => Vec::new(),
            None => find_matching_nodes(spec.get("nodeSelector"), node_labels),
        };

        for node_name in &targets {
            bgp_nodes.insert(node_name.clone());
            neighbors.push(BgpNeighbor {
                node_name: node_name.clone(),
                peer_address: peer_ip.clone(),
                peer_asn: as_num,
                local_asn: 0,
                session_state: BgpSessionState::Configured,
                policy_name: p_name.clone(),
                policy_kind: "BGPPeer".to_string(),
                // MetalLB ships a `BGPPeer` too, so the kind alone does not
                // identify this row's CRD.
                policy_api_version: calico_bgp_peer_resource().api_version.clone(),
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
    BgpClusterSummary {
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
    }
}

// ---------------------------------------------------------------------------
// Selector Matching
// ---------------------------------------------------------------------------

/// The Service selectors carried by the cluster's `CiliumBGPAdvertisement`
/// objects, one entry per `advertisementType: Service` block. `None` is "no
/// advertisement was read at all", which is what a legacy `v2alpha1` cluster
/// looks like and tells us nothing; `Some(vec![])` is advertisements that
/// announce no Service — a `PodCIDR`-only cluster. A `None` entry inside the
/// list is a Service advertisement with no selector, which takes every
/// Service.
fn service_advertisement_selectors(adv_items: &[DynamicObject]) -> Option<Vec<Option<Value>>> {
    if adv_items.is_empty() {
        return None;
    }
    let mut selectors = Vec::new();
    for adv in adv_items {
        let Some(blocks) = adv
            .data
            .get("spec")
            .and_then(|s| s.get("advertisements"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        for block in blocks {
            let kind = block
                .get("advertisementType")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if kind.eq_ignore_ascii_case("service") {
                selectors.push(block.get("selector").cloned());
            }
        }
    }
    Some(selectors)
}

/// The LoadBalancer Services the cluster's advertisements actually select.
/// With no advertisement to read, every LoadBalancer Service is returned, as
/// before — an absent advertisement is not evidence either way.
fn advertised_lb_services<'a>(
    adv_items: &[DynamicObject],
    lb_services: &'a [LbService],
) -> Vec<&'a LbService> {
    let Some(selectors) = service_advertisement_selectors(adv_items) else {
        return lb_services.iter().collect();
    };
    lb_services
        .iter()
        .filter(|svc| {
            selectors.iter().any(|selector| match selector {
                None => true,
                Some(sel) => matches_label_selector(sel, &svc.labels),
            })
        })
        .collect()
}

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
            if matches_label_selector(sel, labels) {
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

/// A Kubernetes `LabelSelector` (`matchLabels` / `matchExpressions`) against
/// one object's labels. Nodes and Services are both selected this way.
fn matches_label_selector(selector: &Value, node_labels: &BTreeMap<String, String>) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A LoadBalancer Service fixture with no labels.
    fn lb(name: &str, namespace: &str, ip: &str, pool: Option<&str>) -> LbService {
        LbService {
            name: name.to_string(),
            namespace: namespace.to_string(),
            load_balancer_ip: ip.to_string(),
            ip_pool: pool.map(String::from),
            labels: BTreeMap::new(),
        }
    }

    /// The same, carrying one label an advertisement can select on.
    fn lb_labelled(name: &str, ip: &str, key: &str, value: &str) -> LbService {
        let mut svc = lb(name, "default", ip, None);
        svc.labels.insert(key.to_string(), value.to_string());
        svc
    }

    #[test]
    fn test_all_api_resources_and_timeout() {
        assert_eq!(
            cilium_bgp_peering_policy_resource().kind,
            "CiliumBGPPeeringPolicy"
        );
        assert_eq!(cilium_bgp_peering_policy_resource().version, "v2alpha1");

        assert_eq!(
            cilium_bgp_cluster_config_v2_resource().kind,
            "CiliumBGPClusterConfig"
        );
        assert_eq!(cilium_bgp_cluster_config_v2_resource().version, "v2");

        assert_eq!(
            cilium_bgp_cluster_config_v2alpha1_resource().kind,
            "CiliumBGPClusterConfig"
        );
        assert_eq!(
            cilium_bgp_cluster_config_v2alpha1_resource().version,
            "v2alpha1"
        );

        assert_eq!(
            cilium_bgp_peer_config_v2_resource().kind,
            "CiliumBGPPeerConfig"
        );
        assert_eq!(cilium_bgp_peer_config_v2_resource().version, "v2");

        assert_eq!(
            cilium_bgp_peer_config_v2alpha1_resource().kind,
            "CiliumBGPPeerConfig"
        );
        assert_eq!(
            cilium_bgp_peer_config_v2alpha1_resource().version,
            "v2alpha1"
        );

        assert_eq!(
            cilium_bgp_advertisement_v2_resource().kind,
            "CiliumBGPAdvertisement"
        );
        assert_eq!(cilium_bgp_advertisement_v2_resource().version, "v2");

        assert_eq!(
            cilium_bgp_advertisement_v2alpha1_resource().kind,
            "CiliumBGPAdvertisement"
        );
        assert_eq!(
            cilium_bgp_advertisement_v2alpha1_resource().version,
            "v2alpha1"
        );

        assert_eq!(
            cilium_bgp_node_config_v2_resource().kind,
            "CiliumBGPNodeConfig"
        );
        assert_eq!(cilium_bgp_node_config_v2_resource().version, "v2");

        assert_eq!(
            cilium_bgp_node_config_v2alpha1_resource().kind,
            "CiliumBGPNodeConfig"
        );
        assert_eq!(
            cilium_bgp_node_config_v2alpha1_resource().version,
            "v2alpha1"
        );

        assert_eq!(
            cilium_bgp_node_config_override_v2_resource().kind,
            "CiliumBGPNodeConfigOverride"
        );
        assert_eq!(cilium_bgp_node_config_override_v2_resource().version, "v2");

        assert_eq!(
            cilium_load_balancer_ip_pool_v2_resource().kind,
            "CiliumLoadBalancerIPPool"
        );
        assert_eq!(cilium_load_balancer_ip_pool_v2_resource().version, "v2");

        assert_eq!(
            cilium_load_balancer_ip_pool_v2alpha1_resource().kind,
            "CiliumLoadBalancerIPPool"
        );
        assert_eq!(
            cilium_load_balancer_ip_pool_v2alpha1_resource().version,
            "v2alpha1"
        );

        assert_eq!(cilium_node_resource().kind, "CiliumNode");
        assert_eq!(cilium_node_resource().version, "v2");

        assert_eq!(metallb_bgp_peer_resource().kind, "BGPPeer");
        assert_eq!(metallb_bgp_peer_resource().version, "v1beta2");

        assert_eq!(metallb_ip_pool_resource().kind, "IPAddressPool");
        assert_eq!(metallb_ip_pool_resource().version, "v1beta1");

        assert_eq!(calico_bgp_peer_resource().kind, "BGPPeer");
        assert_eq!(calico_bgp_peer_resource().version, "v1");

        assert_eq!(request_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn test_bgp_engine_type_display_and_equality() {
        assert_eq!(BgpEngineType::CiliumV2.to_string(), "Cilium BGP (v2)");
        assert_eq!(
            BgpEngineType::CiliumV2Alpha1.to_string(),
            "Cilium BGP (v2alpha1)"
        );
        assert_eq!(BgpEngineType::MetalLB.to_string(), "MetalLB BGP");
        assert_eq!(BgpEngineType::Calico.to_string(), "Calico BGP");
        assert_eq!(BgpEngineType::None.to_string(), "No BGP Engine Detected");

        let json = serde_json::to_string(&BgpEngineType::CiliumV2).unwrap();
        let parsed: BgpEngineType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, BgpEngineType::CiliumV2);
    }

    #[test]
    fn test_bgp_session_state_all_cases() {
        let cases = vec![
            ("Established", BgpSessionState::Established),
            ("established", BgpSessionState::Established),
            ("ESTABLISHED", BgpSessionState::Established),
            ("up", BgpSessionState::Established),
            ("UP", BgpSessionState::Established),
            ("Active", BgpSessionState::Active),
            ("active", BgpSessionState::Active),
            ("Connect", BgpSessionState::Connect),
            ("connect", BgpSessionState::Connect),
            ("Idle", BgpSessionState::Idle),
            ("idle", BgpSessionState::Idle),
            ("down", BgpSessionState::Idle),
            ("DOWN", BgpSessionState::Idle),
            ("OpenSent", BgpSessionState::OpenSent),
            ("opensent", BgpSessionState::OpenSent),
            ("OpenConfirm", BgpSessionState::OpenConfirm),
            ("openconfirm", BgpSessionState::OpenConfirm),
            ("Configured", BgpSessionState::Configured),
            ("configured", BgpSessionState::Configured),
            ("ready", BgpSessionState::Configured),
            ("READY", BgpSessionState::Configured),
            ("foobar", BgpSessionState::Unknown),
            ("", BgpSessionState::Unknown),
        ];

        for (input, expected) in cases {
            assert_eq!(BgpSessionState::parse(input), expected, "input: {}", input);
        }

        assert_eq!(BgpSessionState::Established.to_string(), "Established");
        assert_eq!(BgpSessionState::Active.to_string(), "Active");
        assert_eq!(BgpSessionState::Connect.to_string(), "Connect");
        assert_eq!(BgpSessionState::Idle.to_string(), "Idle");
        assert_eq!(BgpSessionState::OpenSent.to_string(), "OpenSent");
        assert_eq!(BgpSessionState::OpenConfirm.to_string(), "OpenConfirm");
        assert_eq!(BgpSessionState::Configured.to_string(), "Configured");
        assert_eq!(BgpSessionState::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn test_matches_label_selector_edges() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "topology.kubernetes.io/zone".to_string(),
            "us-east-1a".to_string(),
        );
        labels.insert("node-role.kubernetes.io/worker".to_string(), "".to_string());
        labels.insert("rack".to_string(), "rack-42".to_string());

        // Null and empty object match everything
        assert!(matches_label_selector(&Value::Null, &labels));
        assert!(matches_label_selector(&json!({}), &labels));
        assert!(matches_label_selector(&json!(42), &labels));

        // matchLabels success & fail
        assert!(matches_label_selector(
            &json!({"matchLabels": {"topology.kubernetes.io/zone": "us-east-1a"}}),
            &labels
        ));
        assert!(!matches_label_selector(
            &json!({"matchLabels": {"topology.kubernetes.io/zone": "us-west-1b"}}),
            &labels
        ));
        assert!(!matches_label_selector(
            &json!({"matchLabels": {"missing-label": "val"}}),
            &labels
        ));
        // matchLabels with non-string value is ignored
        assert!(matches_label_selector(
            &json!({"matchLabels": {"topology.kubernetes.io/zone": 123}}),
            &labels
        ));

        // matchExpressions: Exists
        assert!(matches_label_selector(
            &json!({"matchExpressions": [{"key": "rack", "operator": "Exists"}]}),
            &labels
        ));
        assert!(!matches_label_selector(
            &json!({"matchExpressions": [{"key": "nonexistent", "operator": "Exists"}]}),
            &labels
        ));

        // matchExpressions: DoesNotExist
        assert!(matches_label_selector(
            &json!({"matchExpressions": [{"key": "nonexistent", "operator": "DoesNotExist"}]}),
            &labels
        ));
        assert!(!matches_label_selector(
            &json!({"matchExpressions": [{"key": "rack", "operator": "DoesNotExist"}]}),
            &labels
        ));

        // matchExpressions: In
        assert!(matches_label_selector(
            &json!({"matchExpressions": [{"key": "rack", "operator": "In", "values": ["rack-41", "rack-42"]}]}),
            &labels
        ));
        assert!(!matches_label_selector(
            &json!({"matchExpressions": [{"key": "rack", "operator": "In", "values": ["rack-1", "rack-2"]}]}),
            &labels
        ));
        assert!(!matches_label_selector(
            &json!({"matchExpressions": [{"key": "missing", "operator": "In", "values": ["val"]}]}),
            &labels
        ));

        // matchExpressions: NotIn
        assert!(matches_label_selector(
            &json!({"matchExpressions": [{"key": "rack", "operator": "NotIn", "values": ["rack-1", "rack-2"]}]}),
            &labels
        ));
        assert!(!matches_label_selector(
            &json!({"matchExpressions": [{"key": "rack", "operator": "NotIn", "values": ["rack-42", "rack-43"]}]}),
            &labels
        ));
        assert!(matches_label_selector(
            &json!({"matchExpressions": [{"key": "missing", "operator": "NotIn", "values": ["val"]}]}),
            &labels
        ));

        // matchExpressions: invalid expression or unknown operator
        assert!(matches_label_selector(
            &json!({"matchExpressions": [{"no_key": "val"}]}),
            &labels
        ));
        assert!(matches_label_selector(
            &json!({"matchExpressions": [{"key": "rack", "operator": "UnknownOp", "values": ["rack-42"]}]}),
            &labels
        ));
    }

    #[test]
    fn test_find_matching_nodes() {
        let mut node_labels = HashMap::new();
        let mut l1 = BTreeMap::new();
        l1.insert("env".to_string(), "prod".to_string());
        l1.insert("zone".to_string(), "east".to_string());
        node_labels.insert("node-1".to_string(), l1);

        let mut l2 = BTreeMap::new();
        l2.insert("env".to_string(), "staging".to_string());
        l2.insert("zone".to_string(), "west".to_string());
        node_labels.insert("node-2".to_string(), l2);

        // None selector matches all
        let matched = find_matching_nodes(None, &node_labels);
        assert_eq!(matched.len(), 2);

        // Empty array selector matches all
        let empty_arr = json!([]);
        let matched = find_matching_nodes(Some(&empty_arr), &node_labels);
        assert_eq!(matched.len(), 2);

        // Empty object selector matches all
        let empty_obj = json!({});
        let matched = find_matching_nodes(Some(&empty_obj), &node_labels);
        assert_eq!(matched.len(), 2);

        // Single object selector
        let sel_obj = json!({"matchLabels": {"env": "prod"}});
        let matched = find_matching_nodes(Some(&sel_obj), &node_labels);
        assert_eq!(matched, vec!["node-1".to_string()]);

        // Array of selectors (OR logic)
        let sel_arr = json!([
            {"matchLabels": {"env": "prod"}},
            {"matchLabels": {"zone": "west"}}
        ]);
        let mut matched = find_matching_nodes(Some(&sel_arr), &node_labels);
        matched.sort();
        assert_eq!(matched, vec!["node-1".to_string(), "node-2".to_string()]);
    }

    #[test]
    fn test_parse_single_peer_status_variations() {
        // Standard camelCase
        let p1 = json!({
            "peerAddress": "192.168.1.1",
            "peerASN": 65001,
            "localASN": 65000,
            "sessionState": "Established",
            "uptime": "5d2h",
            "routesAdvertised": 15,
            "routesReceived": 25,
            "name": "peer-core-1"
        });
        let info1 = parse_single_peer_status(&p1, 0).expect("should parse");
        assert_eq!(info1.peer_address, "192.168.1.1");
        assert_eq!(info1.peer_asn, 65001);
        assert_eq!(info1.local_asn, 65000);
        assert_eq!(info1.session_state, BgpSessionState::Established);
        assert_eq!(info1.uptime.as_deref(), Some("5d2h"));
        assert_eq!(info1.routes_advertised, 15);
        assert_eq!(info1.routes_received, 25);
        assert_eq!(info1.peer_name, "peer-core-1");

        // Alternate snake_case & shortened keys
        let p2 = json!({
            "peer_address": "10.10.10.1",
            "peer_asn": 64512,
            "local_asn": 64500,
            "state": "Active",
            "established_time": "12m",
            "routes_advertised": 4,
            "routes_received": 8
        });
        let info2 = parse_single_peer_status(&p2, 0).expect("should parse");
        assert_eq!(info2.peer_address, "10.10.10.1");
        assert_eq!(info2.peer_asn, 64512);
        assert_eq!(info2.local_asn, 64500);
        assert_eq!(info2.session_state, BgpSessionState::Active);
        assert_eq!(info2.uptime.as_deref(), Some("12m"));
        assert_eq!(info2.routes_advertised, 4);
        assert_eq!(info2.routes_received, 8);

        // Fallback local ASN & alternate keys like "peer", "asn", "status", "lastChange", "advertised", "received"
        let p3 = json!({
            "peer": "172.16.0.2",
            "asn": 64550,
            "status": "Connect",
            "lastChange": "1h",
            "advertised": 10,
            "received": 20
        });
        let info3 = parse_single_peer_status(&p3, 64500).expect("should parse");
        assert_eq!(info3.peer_address, "172.16.0.2");
        assert_eq!(info3.peer_asn, 64550);
        assert_eq!(info3.local_asn, 64500);
        assert_eq!(info3.session_state, BgpSessionState::Connect);
        assert_eq!(info3.uptime.as_deref(), Some("1h"));
        assert_eq!(info3.routes_advertised, 10);
        assert_eq!(info3.routes_received, 20);

        // Missing peerAddress returns None
        let p4 = json!({
            "peerASN": 65000,
            "sessionState": "Established"
        });
        assert!(parse_single_peer_status(&p4, 0).is_none());
    }

    #[test]
    fn test_extract_live_bgp_peers_recursive() {
        let nested_status = json!({
            "bgp": [
                {
                    "peerAddress": "192.168.1.1",
                    "peerASN": 65001,
                    "sessionState": "Established"
                }
            ],
            "instances": [
                {
                    "localASN": 65100,
                    "neighbors": [
                        {
                            "peerAddress": "192.168.1.2",
                            "peerASN": 65002,
                            "sessionState": "Active"
                        }
                    ]
                }
            ]
        });

        let peers = extract_live_bgp_peers(&nested_status, 65000);
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].peer_address, "192.168.1.1");
        assert_eq!(peers[0].session_state, BgpSessionState::Established);
        assert_eq!(peers[1].peer_address, "192.168.1.2");
        assert_eq!(peers[1].local_asn, 65100);
        assert_eq!(peers[1].session_state, BgpSessionState::Active);

        // Empty value returns empty
        assert!(extract_live_bgp_peers(&json!("invalid"), 0).is_empty());
        assert!(extract_live_bgp_peers(&json!(42), 0).is_empty());
    }

    #[test]
    fn test_update_or_insert_neighbor_logic() {
        let mut neighbors = vec![BgpNeighbor {
            node_name: "node-1".to_string(),
            peer_address: "10.0.0.1".to_string(),
            peer_asn: 65001,
            local_asn: 65000,
            session_state: BgpSessionState::Configured,
            policy_name: "policy-a".to_string(),
            policy_kind: "CiliumBGPClusterConfig".to_string(),
            policy_api_version: String::new(),
            namespace: None,
            export_pod_cidr: true,
            hold_time_seconds: Some(90),
            keepalive_time_seconds: Some(30),
            connect_retry_seconds: Some(120),
            multihop_ttl: Some(64),
            graceful_restart: true,
            advertised_prefixes: vec!["10.244.0.0/24".to_string()],
            routes_count: 1,
            routes_received: 0,
            uptime_or_last_change: None,
        }];

        let mut node_pod_cidrs = HashMap::new();
        node_pod_cidrs.insert("node-1".to_string(), vec!["10.244.0.0/24".to_string()]);
        node_pod_cidrs.insert("node-2".to_string(), vec!["10.244.1.0/24".to_string()]);

        let lb_services = [lb("web-svc", "default", "1.2.3.4", Some("public-pool"))];

        // 1. Update existing neighbor (matched by node_name + peer_address)
        let live_update = LiveBgpPeerInfo {
            peer_address: "10.0.0.1".to_string(),
            peer_asn: 65001,
            local_asn: 65000,
            session_state: BgpSessionState::Established,
            uptime: Some("3d4h".to_string()),
            routes_advertised: 5,
            routes_received: 120,
            peer_name: "".to_string(),
        };

        update_or_insert_neighbor(
            &mut neighbors,
            "node-1",
            live_update,
            "default-pol",
            "CiliumBGPNodeConfig",
            true,
            &node_pod_cidrs,
            lb_services.len(),
        );

        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].session_state, BgpSessionState::Established);
        assert_eq!(neighbors[0].uptime_or_last_change.as_deref(), Some("3d4h"));
        assert_eq!(neighbors[0].routes_count, 5);
        assert_eq!(neighbors[0].routes_received, 120);

        // 2. Insert new neighbor (no match)
        let live_insert = LiveBgpPeerInfo {
            peer_address: "10.0.0.2".to_string(),
            peer_asn: 65002,
            local_asn: 65000,
            session_state: BgpSessionState::Active,
            uptime: Some("10m".to_string()),
            routes_advertised: 0,
            routes_received: 0,
            peer_name: "custom-peer-name".to_string(),
        };

        update_or_insert_neighbor(
            &mut neighbors,
            "node-2",
            live_insert,
            "default-pol",
            "CiliumBGPNodeConfig",
            true,
            &node_pod_cidrs,
            lb_services.len(),
        );

        assert_eq!(neighbors.len(), 2);
        let n2 = &neighbors[1];
        assert_eq!(n2.node_name, "node-2");
        assert_eq!(n2.peer_address, "10.0.0.2");
        assert_eq!(n2.peer_asn, 65002);
        assert_eq!(n2.session_state, BgpSessionState::Active);
        assert_eq!(n2.policy_name, "custom-peer-name");
        assert_eq!(n2.policy_kind, "CiliumBGPNodeConfig");
        assert!(n2.export_pod_cidr);
        // The VIP is not copied onto the neighbour, only counted.
        assert_eq!(
            n2.advertised_prefixes,
            vec!["PodCIDR: 10.244.1.0/24".to_string()]
        );
        assert_eq!(n2.routes_count, 2);

        // 3. Insert without export_pod_cidr
        let live_no_cidr = LiveBgpPeerInfo {
            peer_address: "10.0.0.3".to_string(),
            peer_asn: 65003,
            local_asn: 65000,
            session_state: BgpSessionState::Idle,
            uptime: None,
            routes_advertised: 0,
            routes_received: 0,
            peer_name: "".to_string(),
        };

        update_or_insert_neighbor(
            &mut neighbors,
            "node-2",
            live_no_cidr,
            "fallback-pol",
            "CiliumNode",
            false,
            &node_pod_cidrs,
            lb_services.len(),
        );

        assert_eq!(neighbors.len(), 3);
        let n3 = &neighbors[2];
        assert_eq!(n3.node_name, "node-2");
        assert_eq!(n3.policy_name, "fallback-pol");
        assert_eq!(n3.policy_kind, "CiliumNode");
        assert!(!n3.export_pod_cidr);
        assert!(n3.advertised_prefixes.is_empty());
        assert_eq!(n3.routes_count, 1);

        // 4. Update matching existing with empty node_name
        let mut empty_node_neighbor = vec![BgpNeighbor {
            node_name: "".to_string(),
            peer_address: "10.0.0.99".to_string(),
            peer_asn: 65099,
            local_asn: 0,
            session_state: BgpSessionState::Configured,
            policy_name: "p".to_string(),
            policy_kind: "k".to_string(),
            policy_api_version: String::new(),
            namespace: None,
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
        }];

        let live_fill_node = LiveBgpPeerInfo {
            peer_address: "10.0.0.99".to_string(),
            peer_asn: 65099,
            local_asn: 65000,
            session_state: BgpSessionState::Established,
            uptime: Some("1d".to_string()),
            routes_advertised: 10,
            routes_received: 50,
            peer_name: "".to_string(),
        };

        update_or_insert_neighbor(
            &mut empty_node_neighbor,
            "node-filled",
            live_fill_node,
            "p",
            "k",
            false,
            &node_pod_cidrs,
            lb_services.len(),
        );

        assert_eq!(empty_node_neighbor.len(), 1);
        assert_eq!(empty_node_neighbor[0].node_name, "node-filled");
        assert_eq!(
            empty_node_neighbor[0].session_state,
            BgpSessionState::Established
        );
        assert_eq!(empty_node_neighbor[0].local_asn, 65000);
        assert_eq!(empty_node_neighbor[0].routes_count, 10);
        assert_eq!(empty_node_neighbor[0].routes_received, 50);
    }

    #[test]
    fn test_cilium_peer_config_data_clone_and_debug() {
        let cfg = CiliumPeerConfigData {
            hold_time: Some(90),
            keepalive: Some(30),
            connect_retry: Some(60),
            multihop: Some(4),
            graceful_restart: true,
        };
        let cloned = cfg.clone();
        assert_eq!(cloned.hold_time, Some(90));
        assert_eq!(cloned.keepalive, Some(30));
        assert_eq!(cloned.connect_retry, Some(60));
        assert_eq!(cloned.multihop, Some(4));
        assert!(cloned.graceful_restart);
        assert!(!format!("{:?}", cfg).is_empty());
    }

    #[test]
    fn test_live_bgp_peer_info_clone_and_debug() {
        let info = LiveBgpPeerInfo {
            peer_address: "10.0.0.1".to_string(),
            peer_asn: 65000,
            local_asn: 65001,
            session_state: BgpSessionState::Established,
            uptime: Some("1h".to_string()),
            routes_advertised: 2,
            routes_received: 10,
            peer_name: "p1".to_string(),
        };
        let cloned = info.clone();
        assert_eq!(cloned.peer_address, "10.0.0.1");
        assert_eq!(cloned.routes_advertised, 2);
        assert!(!format!("{:?}", info).is_empty());
    }

    #[test]
    fn test_build_cilium_bgp_summary_v2_full() {
        let mut pool_obj1 =
            DynamicObject::new("pool-1", &cilium_load_balancer_ip_pool_v2_resource());
        pool_obj1.data = json!({
            "spec": {
                "cidrs": ["192.168.10.0/24"],
                "serviceSelector": {"matchLabels": {"app": "web"}},
                "disabled": false
            }
        });

        let mut pool_obj2 =
            DynamicObject::new("pool-2", &cilium_load_balancer_ip_pool_v2_resource());
        pool_obj2.data = json!({
            "spec": {
                "blocks": [{"cidr": "10.10.0.0/16"}],
                "disabled": true
            }
        });

        let mut peer_cfg = DynamicObject::new("peer-cfg-1", &cilium_bgp_peer_config_v2_resource());
        peer_cfg.data = json!({
            "spec": {
                "timers": {
                    "holdTimeSeconds": 90,
                    "keepAliveTimeSeconds": 30,
                    "connectRetryTimeSeconds": 60
                },
                "ebgpMultihop": 4,
                "gracefulRestart": {"enabled": true}
            }
        });

        let mut adv_obj = DynamicObject::new("adv-1", &cilium_bgp_advertisement_v2_resource());
        adv_obj.data = json!({
            "spec": {
                "advertisements": [
                    {"advertisementType": "PodCIDR"},
                    // Without this block the cluster announces no VIP, and
                    // the Services tab below would be empty.
                    {"advertisementType": "Service",
                     "service": {"addresses": ["LoadBalancerIP"]}}
                ]
            }
        });

        let mut cluster_cfg =
            DynamicObject::new("cluster-cfg-1", &cilium_bgp_cluster_config_v2_resource());
        cluster_cfg.data = json!({
            "spec": {
                "nodeSelector": {"matchLabels": {"role": "worker"}},
                "bgpInstances": [
                    {
                        "localASN": 65000,
                        "peers": [
                            {
                                "peerAddress": "172.16.1.1",
                                "peerASN": 65001,
                                "peerConfigRef": {"name": "peer-cfg-1"}
                            },
                            {
                                "peerAddress": "172.16.1.2",
                                "peerASN": 65002,
                                "timers": {
                                    "holdTimeSeconds": 60,
                                    "keepAliveTimeSeconds": 20,
                                    "connectRetryTimeSeconds": 40
                                }
                            }
                        ]
                    }
                ]
            }
        });

        let mut node_cfg = DynamicObject::new("worker-1", &cilium_bgp_node_config_v2_resource());
        node_cfg.data = json!({
            "status": {
                "bgp": [
                    {
                        "peerAddress": "172.16.1.1",
                        "sessionState": "Established",
                        "uptime": "2d4h",
                        "routesAdvertised": 10,
                        "routesReceived": 20
                    }
                ]
            }
        });

        let mut cnode = DynamicObject::new("worker-1", &cilium_node_resource());
        cnode.data = json!({
            "status": {
                "bgp": [
                    {
                        "peerAddress": "172.16.1.2",
                        "sessionState": "Active"
                    }
                ]
            }
        });

        let mut node_labels = HashMap::new();
        let mut w1_labels = BTreeMap::new();
        w1_labels.insert("role".to_string(), "worker".to_string());
        node_labels.insert("worker-1".to_string(), w1_labels);

        let mut node_pod_cidrs = HashMap::new();
        node_pod_cidrs.insert("worker-1".to_string(), vec!["10.244.1.0/24".to_string()]);

        let lb_services = vec![lb(
            "ingress-svc",
            "cilium-system",
            "192.168.10.50",
            Some("pool-1"),
        )];

        let summary = build_cilium_bgp_summary(
            vec![pool_obj1, pool_obj2],
            vec![peer_cfg],
            vec![adv_obj],
            vec![cluster_cfg],
            vec![node_cfg],
            vec![],
            vec![cnode],
            &node_labels,
            &node_pod_cidrs,
            &lb_services,
        );

        assert_eq!(summary.engine, BgpEngineType::CiliumV2);
        assert_eq!(summary.ip_pools.len(), 2);
        assert_eq!(summary.ip_pools[0].name, "pool-1");
        assert!(!summary.ip_pools[0].disabled);
        assert_eq!(summary.ip_pools[1].name, "pool-2");
        assert!(summary.ip_pools[1].disabled);

        assert_eq!(summary.peers.len(), 2);
        assert_eq!(summary.established_peers, 1);
        assert_eq!(summary.degraded_peers, 1);

        let p1 = &summary.peers[0];
        assert_eq!(p1.peer_address, "172.16.1.1");
        assert_eq!(p1.session_state, BgpSessionState::Established);
        assert_eq!(p1.routes_count, 10);
        assert_eq!(p1.routes_received, 20);

        let p2 = &summary.peers[1];
        assert_eq!(p2.peer_address, "172.16.1.2");
        assert_eq!(p2.session_state, BgpSessionState::Active);

        assert_eq!(summary.advertised_services.len(), 1);
        assert_eq!(summary.advertised_services[0].service_name, "ingress-svc");
    }

    #[test]
    fn test_build_cilium_bgp_summary_v2alpha1_legacy() {
        let mut pol_obj = DynamicObject::new("peering-pol", &cilium_bgp_peering_policy_resource());
        pol_obj.data = json!({
            "spec": {
                "nodeSelectors": [{"matchLabels": {"zone": "east"}}],
                "virtualRouters": [
                    {
                        "localASN": 64512,
                        "exportPodCIDR": true,
                        "neighbors": [
                            {
                                "peerAddress": "10.1.1.1",
                                "peerASN": 64513,
                                "holdTimeSeconds": 90,
                                "keepAliveTimeSeconds": 30,
                                "connectRetryTimeSeconds": 60,
                                "eBGPMultihopTTL": 3,
                                "gracefulRestart": {"enabled": true}
                            }
                        ]
                    }
                ]
            }
        });

        let mut node_labels = HashMap::new();
        let mut e_labels = BTreeMap::new();
        e_labels.insert("zone".to_string(), "east".to_string());
        node_labels.insert("node-east".to_string(), e_labels);

        let mut node_pod_cidrs = HashMap::new();
        node_pod_cidrs.insert("node-east".to_string(), vec!["10.244.0.0/24".to_string()]);

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![pol_obj],
            vec![],
            &node_labels,
            &node_pod_cidrs,
            &[],
        );

        assert_eq!(summary.engine, BgpEngineType::CiliumV2Alpha1);
        assert_eq!(summary.peers.len(), 1);
        let peer = &summary.peers[0];
        assert_eq!(peer.node_name, "node-east");
        assert_eq!(peer.peer_address, "10.1.1.1");
        assert_eq!(peer.peer_asn, 64513);
        assert_eq!(peer.local_asn, 64512);
        assert_eq!(peer.hold_time_seconds, Some(90));
        assert_eq!(peer.keepalive_time_seconds, Some(30));
        assert_eq!(peer.connect_retry_seconds, Some(60));
        assert_eq!(peer.multihop_ttl, Some(3));
        assert!(peer.graceful_restart);
        assert!(peer.export_pod_cidr);
        assert_eq!(peer.routes_count, 1);
    }

    #[test]
    fn test_build_cilium_bgp_summary_empty_returns_none_engine() {
        let node_labels = HashMap::new();
        let node_pod_cidrs = HashMap::new();
        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &node_pod_cidrs,
            &[],
        );

        assert_eq!(summary.engine, BgpEngineType::None);
        assert!(summary.peers.is_empty());
        assert!(summary.ip_pools.is_empty());
    }

    #[test]
    fn test_build_metallb_bgp_summary_full() {
        let mut peer_obj = DynamicObject::new("metallb-peer-1", &metallb_bgp_peer_resource());
        peer_obj.metadata.namespace = Some("metallb-system".to_string());
        peer_obj.data = json!({
            "spec": {
                "peerAddress": "192.168.100.1",
                "peerASN": 64512,
                "myASN": 64511,
                "holdTime": 90,
                "nodeSelectors": [{"matchLabels": {"bgp": "true"}}]
            }
        });

        let mut pool_obj = DynamicObject::new("metallb-pool-1", &metallb_ip_pool_resource());
        pool_obj.metadata.namespace = Some("metallb-system".to_string());
        pool_obj.data = json!({
            "spec": {
                "addresses": ["192.168.100.200-192.168.100.250"]
            }
        });

        let mut node_labels = HashMap::new();
        let mut l1 = BTreeMap::new();
        l1.insert("bgp".to_string(), "true".to_string());
        node_labels.insert("node-1".to_string(), l1);

        let mut l2 = BTreeMap::new();
        l2.insert("bgp".to_string(), "false".to_string());
        node_labels.insert("node-2".to_string(), l2);

        let lb_services = vec![lb(
            "nginx-lb",
            "default",
            "192.168.100.201",
            Some("metallb-pool-1"),
        )];

        let summary =
            build_metallb_bgp_summary(vec![peer_obj], vec![pool_obj], &node_labels, &lb_services);

        assert_eq!(summary.engine, BgpEngineType::MetalLB);
        assert_eq!(summary.bgp_nodes, 1);
        assert_eq!(summary.total_peers, 1);
        assert_eq!(summary.ip_pools.len(), 1);
        assert_eq!(summary.ip_pools[0].name, "metallb-pool-1");
        assert_eq!(
            summary.ip_pools[0].namespace.as_deref(),
            Some("metallb-system")
        );
        assert_eq!(
            summary.ip_pools[0].cidrs,
            vec!["192.168.100.200-192.168.100.250".to_string()]
        );

        let p = &summary.peers[0];
        assert_eq!(p.node_name, "node-1");
        assert_eq!(p.peer_address, "192.168.100.1");
        assert_eq!(p.peer_asn, 64512);
        assert_eq!(p.local_asn, 64511);
        assert_eq!(p.policy_kind, "BGPPeer");
        assert_eq!(p.namespace.as_deref(), Some("metallb-system"));
        assert_eq!(p.hold_time_seconds, Some(90));
        assert_eq!(p.routes_count, 1);

        assert_eq!(summary.advertised_services.len(), 1);
        assert_eq!(summary.advertised_services[0].service_name, "nginx-lb");
    }

    #[test]
    fn test_build_calico_bgp_summary_full() {
        let mut peer_obj = DynamicObject::new("calico-global-peer", &calico_bgp_peer_resource());
        peer_obj.data = json!({
            "spec": {
                "peerIP": "10.0.1.1",
                "asNumber": 65005
            }
        });

        let mut node_labels = HashMap::new();
        node_labels.insert("calico-worker-1".to_string(), BTreeMap::new());

        let summary = build_calico_bgp_summary(vec![peer_obj], &node_labels);

        assert_eq!(summary.engine, BgpEngineType::Calico);
        assert_eq!(summary.bgp_nodes, 1);
        assert_eq!(summary.total_peers, 1);
        let p = &summary.peers[0];
        assert_eq!(p.node_name, "calico-worker-1");
        assert_eq!(p.peer_address, "10.0.1.1");
        assert_eq!(p.peer_asn, 65005);
        assert_eq!(p.policy_kind, "BGPPeer");
    }

    #[test]
    fn test_build_cilium_bgp_summary_alternate_keys_and_edge_cases() {
        // Missing spec objects are skipped cleanly
        let empty_cluster_cfg =
            DynamicObject::new("bad-cluster", &cilium_bgp_cluster_config_v2_resource());
        let empty_policy = DynamicObject::new("bad-policy", &cilium_bgp_peering_policy_resource());

        // Cluster config with alternative key names: `instances`, `neighbors`, `address`, `peerIP`, `localAsn`, `peerAsn`
        let mut cluster_cfg_alt =
            DynamicObject::new("alt-cluster", &cilium_bgp_cluster_config_v2_resource());
        cluster_cfg_alt.data = json!({
            "spec": {
                "instances": [
                    {
                        "localAsn": 64512,
                        "neighbors": [
                            {
                                "peerIP": "10.20.30.40",
                                "peerAsn": 64599
                            },
                            {
                                "address": "10.20.30.41",
                                "peerASN": 64598
                            },
                            {
                                "peerAddress": "" // empty address is skipped
                            }
                        ]
                    }
                ]
            }
        });

        // Advertisement with non-podcidr advertisementType sets export_pod_cidr_default to false
        let mut adv_svc = DynamicObject::new("adv-svc", &cilium_bgp_advertisement_v2_resource());
        adv_svc.data = json!({
            "spec": {
                "advertisements": [
                    {"advertisementType": "Service"}
                ]
            }
        });

        let mut node_labels = HashMap::new();
        node_labels.insert("node-any".to_string(), BTreeMap::new());

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![adv_svc],
            vec![empty_cluster_cfg, cluster_cfg_alt],
            vec![],
            vec![empty_policy],
            vec![],
            &node_labels,
            &HashMap::new(),
            &[],
        );

        assert_eq!(summary.engine, BgpEngineType::CiliumV2);
        assert_eq!(summary.peers.len(), 2);
        assert_eq!(summary.peers[0].peer_address, "10.20.30.40");
        assert_eq!(summary.peers[0].peer_asn, 64599);
        assert_eq!(summary.peers[0].local_asn, 64512);
        assert!(!summary.peers[0].export_pod_cidr);

        assert_eq!(summary.peers[1].peer_address, "10.20.30.41");
        assert_eq!(summary.peers[1].peer_asn, 64598);
    }

    #[test]
    fn test_build_metallb_and_calico_edge_cases() {
        // MetalLB with missing spec or empty peer address
        let empty_peer = DynamicObject::new("bad-peer", &metallb_bgp_peer_resource());
        let mut peer_with_ip = DynamicObject::new("ip-peer", &metallb_bgp_peer_resource());
        peer_with_ip.data = json!({
            "spec": {
                "peerIP": "192.168.1.1",
                "peerAsn": 65001,
                "localAsn": 65000
            }
        });

        let mut node_labels = HashMap::new();
        node_labels.insert("node-1".to_string(), BTreeMap::new());

        let summary =
            build_metallb_bgp_summary(vec![empty_peer, peer_with_ip], vec![], &node_labels, &[]);
        assert_eq!(summary.engine, BgpEngineType::MetalLB);
        assert_eq!(summary.peers.len(), 1);
        assert_eq!(summary.peers[0].peer_address, "192.168.1.1");
        assert_eq!(summary.peers[0].peer_asn, 65001);
        assert_eq!(summary.peers[0].local_asn, 65000);

        // Calico with missing spec
        let empty_calico = DynamicObject::new("bad-calico", &calico_bgp_peer_resource());
        let calico_summary = build_calico_bgp_summary(vec![empty_calico], &node_labels);
        assert_eq!(calico_summary.engine, BgpEngineType::Calico);
        assert_eq!(calico_summary.peers.len(), 0);
    }

    #[test]
    fn a_bgppeer_row_carries_the_crd_it_came_from() {
        // `BGPPeer` is shipped by both MetalLB and Calico, so the kind alone
        // cannot tell a drill-down which CRD to fetch.
        let mut node_labels = HashMap::new();
        node_labels.insert("node-1".to_string(), BTreeMap::new());

        let mut metallb_peer = DynamicObject::new("metallb-peer", &metallb_bgp_peer_resource());
        metallb_peer.metadata.namespace = Some("metallb-system".to_string());
        metallb_peer.data = json!({"spec": {"peerAddress": "10.0.0.1", "peerASN": 64512}});
        let metallb = build_metallb_bgp_summary(vec![metallb_peer], vec![], &node_labels, &[]);
        assert_eq!(metallb.peers[0].policy_kind, "BGPPeer");
        assert_eq!(metallb.peers[0].policy_api_version, "metallb.io/v1beta2");
        assert_eq!(
            metallb.peers[0].namespace.as_deref(),
            Some("metallb-system")
        );

        let mut calico_peer = DynamicObject::new("calico-peer", &calico_bgp_peer_resource());
        calico_peer.data = json!({"spec": {"peerIP": "10.0.0.2", "asNumber": 64512}});
        let calico = build_calico_bgp_summary(vec![calico_peer], &node_labels);
        assert_eq!(calico.peers[0].policy_kind, "BGPPeer");
        assert_eq!(
            calico.peers[0].policy_api_version,
            "crd.projectcalico.org/v1"
        );
        assert_ne!(
            calico.peers[0].policy_api_version,
            metallb.peers[0].policy_api_version
        );

        // The legacy Cilium policy is pinned to v2alpha1.
        let mut pol = DynamicObject::new("legacy-pol", &cilium_bgp_peering_policy_resource());
        pol.data = json!({
            "spec": {"virtualRouters": [{
                "localASN": 64512,
                "neighbors": [{"peerAddress": "10.1.1.1", "peerASN": 64513}]
            }]}
        });
        let legacy = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![pol],
            vec![],
            &node_labels,
            &HashMap::new(),
            &[],
        );
        assert_eq!(
            legacy.peers[0].policy_api_version, "cilium.io/v2alpha1",
            "the legacy policy only exists under v2alpha1"
        );
    }

    #[test]
    fn a_neighbour_counts_the_advertised_vips_rather_than_copying_them() {
        // `advertised_services` carries each VIP once, with its announcing
        // nodes and peers. A copy per neighbour grows as nodes × peers ×
        // services and says nothing new.
        let mut node_labels = HashMap::new();
        node_labels.insert("node-1".to_string(), BTreeMap::new());
        node_labels.insert("node-2".to_string(), BTreeMap::new());
        let mut node_pod_cidrs = HashMap::new();
        node_pod_cidrs.insert("node-1".to_string(), vec!["10.244.0.0/24".to_string()]);
        node_pod_cidrs.insert("node-2".to_string(), vec!["10.244.1.0/24".to_string()]);

        let lb_services = vec![
            lb("web", "default", "1.2.3.4", None),
            lb("api", "default", "1.2.3.5", None),
        ];

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![],
            vec![one_node_cluster_config()],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &node_pod_cidrs,
            &lb_services,
        );

        assert_eq!(summary.peers.len(), 2, "one peer per matching node");
        for peer in &summary.peers {
            assert!(
                peer.advertised_prefixes.iter().all(|p| !p.contains("VIP:")),
                "no VIP copies on a neighbour, got {:?}",
                peer.advertised_prefixes
            );
            // One PodCIDR plus the two advertised VIPs.
            assert_eq!(peer.routes_count, 3);
        }
        assert_eq!(summary.advertised_services.len(), 2);

        // MetalLB neighbours carry none either.
        let mut metallb_peer = DynamicObject::new("peer-1", &metallb_bgp_peer_resource());
        metallb_peer.data = json!({"spec": {"peerAddress": "10.0.0.1", "peerASN": 64512}});
        let metallb =
            build_metallb_bgp_summary(vec![metallb_peer], vec![], &node_labels, &lb_services);
        for peer in &metallb.peers {
            assert!(peer.advertised_prefixes.is_empty());
            assert_eq!(peer.routes_count, 2);
        }
        assert_eq!(metallb.advertised_services.len(), 2);
    }

    #[test]
    fn a_calico_peer_is_reported_only_on_the_nodes_it_targets() {
        let mut node_labels = HashMap::new();
        node_labels.insert(
            "rack1-host1".to_string(),
            BTreeMap::from([("rack".to_string(), "rack1".to_string())]),
        );
        node_labels.insert(
            "rack2-host1".to_string(),
            BTreeMap::from([("rack".to_string(), "rack2".to_string())]),
        );

        // `spec.node` names one node.
        let mut pinned = DynamicObject::new("pinned", &calico_bgp_peer_resource());
        pinned.data = json!({
            "spec": {"node": "rack1-host1", "peerIP": "10.0.0.1", "asNumber": 64512}
        });
        let summary = build_calico_bgp_summary(vec![pinned], &node_labels);
        let nodes: Vec<&str> = summary.peers.iter().map(|p| p.node_name.as_str()).collect();
        assert_eq!(nodes, ["rack1-host1"]);
        assert_eq!(summary.total_peers, 1);
        assert_eq!(summary.bgp_nodes, 1);

        // `spec.nodeSelector` selects several.
        let mut selected = DynamicObject::new("selected", &calico_bgp_peer_resource());
        selected.data = json!({
            "spec": {
                "nodeSelector": {"matchLabels": {"rack": "rack2"}},
                "peerIP": "10.0.0.2",
                "asNumber": 64512
            }
        });
        let summary = build_calico_bgp_summary(vec![selected], &node_labels);
        let nodes: Vec<&str> = summary.peers.iter().map(|p| p.node_name.as_str()).collect();
        assert_eq!(nodes, ["rack2-host1"]);

        // Neither: the peer is global.
        let mut global = DynamicObject::new("global", &calico_bgp_peer_resource());
        global.data = json!({"spec": {"peerIP": "10.0.0.3", "asNumber": 64512}});
        let summary = build_calico_bgp_summary(vec![global], &node_labels);
        assert_eq!(summary.total_peers, 2);
        assert_eq!(summary.bgp_nodes, 2);

        // A node this cluster does not have is reported nowhere, rather than
        // everywhere.
        let mut stale = DynamicObject::new("stale", &calico_bgp_peer_resource());
        stale.data = json!({
            "spec": {"node": "decommissioned", "peerIP": "10.0.0.4", "asNumber": 64512}
        });
        let summary = build_calico_bgp_summary(vec![stale], &node_labels);
        assert_eq!(summary.total_peers, 0);
    }

    #[test]
    fn a_legacy_singular_node_selector_limits_the_peer_to_matching_nodes() {
        let mut pol = DynamicObject::new("legacy-pol", &cilium_bgp_peering_policy_resource());
        // The CRD's own spelling: `spec.nodeSelector`, singular.
        pol.data = json!({
            "spec": {
                "nodeSelector": {"matchLabels": {"zone": "east"}},
                "virtualRouters": [{
                    "localASN": 64512,
                    "neighbors": [{"peerAddress": "10.1.1.1", "peerASN": 64513}]
                }]
            }
        });

        let mut node_labels = HashMap::new();
        node_labels.insert(
            "east-1".to_string(),
            BTreeMap::from([("zone".to_string(), "east".to_string())]),
        );
        node_labels.insert(
            "west-1".to_string(),
            BTreeMap::from([("zone".to_string(), "west".to_string())]),
        );

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![pol],
            vec![],
            &node_labels,
            &HashMap::new(),
            &[],
        );

        let nodes: Vec<&str> = summary.peers.iter().map(|p| p.node_name.as_str()).collect();
        assert_eq!(
            nodes,
            ["east-1"],
            "a policy scoped to one zone must not peer the whole cluster"
        );
        assert_eq!(summary.bgp_nodes, 1);
        assert_eq!(summary.total_peers, 1);
    }

    /// A Cilium cluster config peering one node, so the summary has an engine
    /// and the Services tab is reached.
    fn one_node_cluster_config() -> DynamicObject {
        let mut cluster_cfg =
            DynamicObject::new("cluster-cfg", &cilium_bgp_cluster_config_v2_resource());
        cluster_cfg.data = json!({
            "spec": {
                "bgpInstances": [{
                    "localASN": 65000,
                    "peers": [{"peerAddress": "172.16.1.1", "peerASN": 65001}]
                }]
            }
        });
        cluster_cfg
    }

    fn advertisement(name: &str, blocks: Value) -> DynamicObject {
        let mut adv = DynamicObject::new(name, &cilium_bgp_advertisement_v2_resource());
        adv.data = json!({ "spec": { "advertisements": blocks } });
        adv
    }

    fn one_node_labels() -> HashMap<String, BTreeMap<String, String>> {
        let mut node_labels = HashMap::new();
        node_labels.insert("node-1".to_string(), BTreeMap::new());
        node_labels
    }

    #[test]
    fn an_advertisement_carrying_only_pod_cidr_advertises_no_service() {
        let node_labels = one_node_labels();
        let lb_services = vec![lb("web", "default", "1.2.3.4", None)];

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![advertisement(
                "adv-podcidr",
                json!([{"advertisementType": "PodCIDR"}]),
            )],
            vec![one_node_cluster_config()],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );

        assert_eq!(summary.peers.len(), 1, "the peer is still configured");
        assert!(
            summary.advertised_services.is_empty(),
            "a PodCIDR-only advertisement announces no VIP, got {:?}",
            summary.advertised_services
        );
    }

    #[test]
    fn an_advertisement_selector_excluding_a_service_yields_no_row_for_it() {
        let node_labels = one_node_labels();
        let lb_services = vec![
            lb_labelled("public-lb", "1.2.3.4", "tier", "public"),
            lb_labelled("internal-lb", "10.0.0.9", "tier", "internal"),
        ];

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![advertisement(
                "adv-service",
                json!([{
                    "advertisementType": "Service",
                    "service": {"addresses": ["LoadBalancerIP"]},
                    "selector": {"matchLabels": {"tier": "public"}}
                }]),
            )],
            vec![one_node_cluster_config()],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );

        let names: Vec<&str> = summary
            .advertised_services
            .iter()
            .map(|s| s.service_name.as_str())
            .collect();
        assert_eq!(names, ["public-lb"]);
    }

    #[test]
    fn a_service_advertisement_without_a_selector_takes_every_service() {
        let node_labels = one_node_labels();
        let lb_services = vec![
            lb_labelled("public-lb", "1.2.3.4", "tier", "public"),
            lb_labelled("internal-lb", "10.0.0.9", "tier", "internal"),
        ];

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![advertisement(
                "adv-service",
                json!([
                    {"advertisementType": "PodCIDR"},
                    {"advertisementType": "Service",
                     "service": {"addresses": ["LoadBalancerIP"]}}
                ]),
            )],
            vec![one_node_cluster_config()],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );

        assert_eq!(summary.advertised_services.len(), 2);
    }

    #[test]
    fn a_cluster_with_no_advertisement_to_read_still_lists_its_services() {
        // Nothing was read about what is advertised, so nothing is claimed
        // either way: the legacy v2alpha1 shape keeps its Services tab.
        let node_labels = one_node_labels();
        let lb_services = vec![lb("web", "default", "1.2.3.4", None)];

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![],
            vec![one_node_cluster_config()],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );

        assert_eq!(summary.advertised_services.len(), 1);
    }

    /// A cluster that serves Nodes and Services, and answers every CRD list
    /// with `crd_status`.
    fn cluster_answering_crds_with(crd_status: u16) -> kube::Client {
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let path = request.uri().path().to_owned();
            async move {
                let (status, body) = if path == "/api/v1/nodes" {
                    (
                        200,
                        json!({"apiVersion":"v1","kind":"NodeList","metadata":{},"items":[
                            {"apiVersion":"v1","kind":"Node",
                             "metadata":{"name":"node-1","labels":{"role":"worker"}},
                             "spec":{"podCIDR":"10.244.0.0/24"}}]}),
                    )
                } else if path == "/api/v1/services" {
                    (
                        200,
                        json!({"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[]}),
                    )
                } else {
                    let reason = if crd_status == 404 {
                        "NotFound"
                    } else {
                        "Forbidden"
                    };
                    (
                        crd_status,
                        json!({"apiVersion":"v1","kind":"Status","status":"Failure",
                            "code":crd_status,"reason":reason,"message":"rejected"}),
                    )
                };
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        kube::Client::new(service, "default")
    }

    #[tokio::test]
    async fn a_forbidden_crd_list_reports_discovery_failure_not_an_absent_engine() {
        let summary = fetch_bgp_summary(&cluster_answering_crds_with(403))
            .await
            .expect("nodes and services were readable");

        assert_eq!(summary.engine, BgpEngineType::None);
        assert_eq!(summary.total_nodes, 1);
        let err = summary
            .error
            .expect("a refused CRD list must not read as an unconfigured cluster");
        assert!(
            err.starts_with("BGP discovery failed: "),
            "error should name the failure, got {err}"
        );
        assert!(
            err.contains("CiliumLoadBalancerIPPool"),
            "error should name the lookup that failed, got {err}"
        );
    }

    #[tokio::test]
    async fn a_cluster_without_the_bgp_crds_reports_no_engine_and_no_error() {
        // 404 is the cluster answering that it serves no such resource.
        let summary = fetch_bgp_summary(&cluster_answering_crds_with(404))
            .await
            .expect("nodes and services were readable");

        assert_eq!(summary.engine, BgpEngineType::None);
        assert_eq!(summary.error, None);
    }
}
