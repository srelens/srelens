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

/// What identifies a Service across the summary: its namespace and name.
type ServiceKey = (String, String);

impl LbService {
    fn key(&self) -> ServiceKey {
        (self.namespace.clone(), self.name.clone())
    }
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

/// What the API server said when asked to list one CRD version.
///
/// Both variants are the server answering. `NotServed` is the `404` that
/// means this cluster has no such resource installed, kept apart from an
/// empty `Served` list because the two say different things about a sibling
/// version: a version that is not served tells us nothing about one that is.
#[derive(Debug)]
enum Listed {
    Served(Vec<DynamicObject>),
    NotServed,
}

impl Listed {
    fn into_items(self) -> Vec<DynamicObject> {
        match self {
            Listed::Served(items) => items,
            Listed::NotServed => Vec::new(),
        }
    }
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
async fn list_one_resource(client: &kube::Client, res: &ApiResource) -> Result<Listed, String> {
    let api: Api<DynamicObject> = Api::all_with(client.clone(), res);
    match tokio::time::timeout(request_timeout(), api.list(&ListParams::default())).await {
        Ok(Ok(list)) => Ok(Listed::Served(list.items)),
        // 404 is this cluster saying it serves no such resource.
        Ok(Err(kube::Error::Api(e))) if e.code == 404 => Ok(Listed::NotServed),
        Ok(Err(e)) => Err(format!("list {}: {}", res.kind, e)),
        Err(_) => Err(format!("list {} timed out", res.kind)),
    }
}

/// List a CRD that exists under two API versions, preferring whichever
/// version has objects. One version answering `404` while the other serves
/// objects is the normal shape of a cluster that has upgraded.
///
/// The lookup fails when no *served* version could be read. A refused `v2`
/// beside a `v2alpha1` the cluster does not serve is a refusal, not an
/// absence: the only version that could have answered did not. Only when
/// neither version is served, or a served version answered with no objects,
/// is the result an empty list.
async fn list_dynamic_resource(
    client: &kube::Client,
    res1: &ApiResource,
    res2: &ApiResource,
) -> Result<Vec<DynamicObject>, String> {
    let first = list_one_resource(client, res1).await;
    if matches!(&first, Ok(Listed::Served(items)) if !items.is_empty()) {
        return first.map(Listed::into_items);
    }
    let second = list_one_resource(client, res2).await;
    match (first, second) {
        (_, Ok(Listed::Served(items))) if !items.is_empty() => Ok(items),
        // A served version answered with no objects: the CRD is installed and
        // empty, whatever the other version said.
        (Ok(Listed::Served(items)), _) => Ok(items),
        (_, Ok(Listed::Served(items))) => Ok(items),
        // Neither version is served: the cluster has no such CRD.
        (Ok(Listed::NotServed), Ok(Listed::NotServed)) => Ok(Vec::new()),
        // The one version that is served refused us.
        (Err(e), Ok(Listed::NotServed)) | (Ok(Listed::NotServed), Err(e)) => Err(e),
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

    // An engine is detected from the objects the cluster serves, not from
    // how many peers they resolve to. A MetalLB `BGPPeer` whose node selector
    // matches no node, or a Calico peer pinned to a node that has since gone,
    // is still that engine, configured to peer nowhere — dropping it for the
    // next engine would end in "no BGP engine", which is a different and
    // wrong claim. Each engine's builder only names itself when it read
    // objects.
    fn detected(summary: &BgpClusterSummary) -> bool {
        summary.engine != BgpEngineType::None
    }

    // 3. Try Cilium BGP Control Plane (v2 / v2alpha1)
    match discover_cilium_bgp(client, &node_labels_map, &node_pod_cidrs, &lb_services).await {
        Ok(cilium_summary) => {
            if detected(&cilium_summary) {
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
            if detected(&metallb_summary) {
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
            if detected(&calico_summary) {
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
    /// `spec.families[].advertisements`: one label selector per address
    /// family, each choosing the `CiliumBGPAdvertisement` objects that reach
    /// sessions using this config. A family without one advertises nothing,
    /// so an empty list here is a config that selects no advertisement.
    advertisement_selectors: Vec<Value>,
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

    // Kept as a lookup rather than flattened to a list: a refused or timed-out
    // advertisement list and a successful empty one both hold no objects, but
    // they say opposite things about a peer whose config selects
    // advertisements by label (see `AdvertisementLookup`).
    let adv_lookup = match list_dynamic_resource(
        client,
        &cilium_bgp_advertisement_v2_resource(),
        &cilium_bgp_advertisement_v2alpha1_resource(),
    )
    .await
    {
        Ok(items) => AdvertisementLookup::Read(items),
        Err(e) => {
            // Recorded through `take`, which owns the failures list here.
            let _ = take(Err(e));
            AdvertisementLookup::Failed
        }
    };

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

    let policy_items = take(
        list_one_resource(client, &cilium_bgp_peering_policy_resource())
            .await
            .map(Listed::into_items),
    );
    let cnode_items = take(
        list_one_resource(client, &cilium_node_resource())
            .await
            .map(Listed::into_items),
    );

    let mut summary = build_cilium_bgp_summary_from(
        pool_items,
        peer_cfg_items,
        adv_lookup,
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

/// What the `CiliumBGPAdvertisement` list came back with.
///
/// `Read(vec![])` is the API server answering that there are no
/// advertisements; a peer whose config selects advertisements by label then
/// genuinely selects none. `Failed` is the list being refused or timing out,
/// which says nothing about what the peer announces — the same peer keeps the
/// union fallback, as a peer with no resolvable config does, and the failure
/// itself travels on the summary's `error`.
#[derive(Debug)]
pub(crate) enum AdvertisementLookup {
    Read(Vec<DynamicObject>),
    Failed,
}

/// [`build_cilium_bgp_summary_from`] for an advertisement list that was read.
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
    build_cilium_bgp_summary_from(
        pool_items,
        peer_cfg_items,
        AdvertisementLookup::Read(adv_items),
        cluster_cfg_items,
        node_cfg_items,
        policy_items,
        cnode_items,
        node_labels,
        node_pod_cidrs,
        lb_services,
    )
}

// Ten arguments, like the wrapper above and its siblings: one per CRD list.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_cilium_bgp_summary_from(
    pool_items: Vec<DynamicObject>,
    peer_cfg_items: Vec<DynamicObject>,
    adv_lookup: AdvertisementLookup,
    cluster_cfg_items: Vec<DynamicObject>,
    node_cfg_items: Vec<DynamicObject>,
    policy_items: Vec<DynamicObject>,
    cnode_items: Vec<DynamicObject>,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
    node_pod_cidrs: &HashMap<String, Vec<String>>,
    lb_services: &[LbService],
) -> BgpClusterSummary {
    // Whether the advertisements were read at all decides, below, whether a
    // resolved peer config may be trusted to select from them.
    let (adv_items, adv_read) = match adv_lookup {
        AdvertisementLookup::Read(items) => (items, true),
        AdvertisementLookup::Failed => (Vec::new(), false),
    };
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
            if let Some(families) = spec.get("families").and_then(|v| v.as_array()) {
                cfg.advertisement_selectors = families
                    .iter()
                    .filter_map(|family| family.get("advertisements").cloned())
                    .collect();
            }
        }
        peer_configs.insert(name, cfg);
    }

    // C. Load CiliumBGPAdvertisement (v2 / v2alpha1)
    //
    // The advertisements say what the cluster announces. Only the Services
    // they select are advertised: an advertisement carrying just `PodCIDR`
    // announces no VIP at all, and one with a Service selector announces only
    // the Services that selector matches.
    //
    // An advertisement reaches a session only through the peer's
    // `CiliumBGPPeerConfig`, whose `spec.families[].advertisements` selects
    // advertisements by label. One that no peer config selects announces
    // nothing, so what each peer advertises is resolved through its config
    // below. The union over every advertisement — what the cluster *could*
    // announce — stands in only for a peer whose config cannot be resolved:
    // no `peerConfigRef`, or one naming a config that was not read — and for
    // every peer when the advertisement list itself could not be read, since
    // a config selecting from a list we never saw selects nothing we can
    // name. For such a peer the union is the pre-existing reading, kept
    // because the alternative is to claim the peer announces nothing.
    let all_advs: Vec<&DynamicObject> = adv_items.iter().collect();
    let advertised_by_any = if adv_items.is_empty() {
        // Nothing was read about what is advertised — the legacy v2alpha1
        // shape — which is not evidence either way.
        None
    } else {
        service_advertisement_selectors(&all_advs)
    };
    let advertised: Vec<&LbService> = select_lb_services(advertised_by_any.as_deref(), lb_services);
    let mut export_pod_cidr_default = true;
    if !adv_items.is_empty() {
        is_cilium_v2 = true;
        export_pod_cidr_default = advertises_pod_cidr(&all_advs);
    }

    // Which sessions carry each advertised Service, kept beside `neighbors`
    // (one entry per row, in push order) until the rows are correlated.
    let mut neighbor_services: Vec<Vec<ServiceKey>> = Vec::new();
    let keys_of = |services: &[&LbService]| -> Vec<ServiceKey> {
        services.iter().map(|svc| svc.key()).collect()
    };

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

                        let resolved_cfg = peer_configs.get(peer_cfg_name);
                        let peer_cfg = resolved_cfg.cloned().unwrap_or_else(|| {
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

                        // What this session announces: the advertisements its
                        // config selects, when both the config and the
                        // advertisements were read (section C). A successful
                        // empty advertisement list leaves a resolved peer
                        // silent; a failed lookup does not.
                        let selection = resolved_cfg.filter(|_| adv_read).map(|cfg| {
                            let selected: Vec<&DynamicObject> = all_advs
                                .iter()
                                .copied()
                                .filter(|adv| {
                                    let labels = adv.metadata.labels.clone().unwrap_or_default();
                                    cfg.advertisement_selectors
                                        .iter()
                                        .any(|sel| matches_label_selector(sel, &labels))
                                })
                                .collect();
                            let services = select_lb_services(
                                service_advertisement_selectors(&selected).as_deref(),
                                lb_services,
                            );
                            (services, advertises_pod_cidr(&selected))
                        });
                        let (peer_advertised, export_pod_cidr): (&[&LbService], bool) =
                            match &selection {
                                Some((services, pod_cidr)) => (services.as_slice(), *pod_cidr),
                                None => (advertised.as_slice(), export_pod_cidr_default),
                            };

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
                            // The advertised VIPs are counted, not copied:
                            // `advertised_services` already carries each one
                            // once, with its announcing nodes and peers. A
                            // copy per neighbour grows as nodes × peers ×
                            // services.
                            let routes_count = prefixes.len() + peer_advertised.len();

                            neighbor_services.push(keys_of(peer_advertised));
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
                                export_pod_cidr,
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
                let inserted = update_or_insert_neighbor(
                    &mut neighbors,
                    &node_name,
                    live,
                    &node_name,
                    "CiliumBGPNodeConfig",
                    export_pod_cidr_default,
                    node_pod_cidrs,
                    advertised.len(),
                );
                if inserted {
                    // A session no cluster config declared: nothing ties it
                    // to a peer config, so the union stands in (section C).
                    neighbor_services.push(keys_of(&advertised));
                }
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
                                neighbor_services.push(keys_of(&advertised));
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
                let inserted = update_or_insert_neighbor(
                    &mut neighbors,
                    &n_name,
                    live,
                    &n_name,
                    "CiliumNode",
                    export_pod_cidr_default,
                    node_pod_cidrs,
                    advertised.len(),
                );
                if inserted {
                    neighbor_services.push(keys_of(&advertised));
                }
            }
        }
    }

    // Correlate Advertised Services: one row per Service some session
    // announces, naming the nodes and peers that carry it rather than every
    // node and peer in the cluster.
    debug_assert_eq!(neighbor_services.len(), neighbors.len());
    let mut announcers: BTreeMap<ServiceKey, (BTreeSet<String>, BTreeSet<String>)> =
        BTreeMap::new();
    for (neighbor, services) in neighbors.iter().zip(&neighbor_services) {
        for key in services {
            let (nodes, peers) = announcers.entry(key.clone()).or_default();
            nodes.insert(neighbor.node_name.clone());
            peers.insert(format!("{}:{}", neighbor.peer_address, neighbor.peer_asn));
        }
    }
    let mut advertised_services: Vec<BgpAdvertisedService> = Vec::new();
    for svc in lb_services {
        let Some((nodes, peers)) = announcers.get(&svc.key()) else {
            continue;
        };
        advertised_services.push(BgpAdvertisedService {
            service_name: svc.name.clone(),
            namespace: svc.namespace.clone(),
            load_balancer_ip: svc.load_balancer_ip.clone(),
            ip_pool: svc.ip_pool.clone(),
            announcing_nodes: nodes.iter().cloned().collect(),
            peers: peers.iter().cloned().collect(),
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
) -> bool {
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
        false
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
        true
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
        Ok(listed) => {
            let items = listed.into_items();
            if items.is_empty() {
                return Ok(BgpClusterSummary::default());
            }
            items
        }
        Err(e) => {
            // The cluster may run MetalLB and simply not let us look.
            return Err(e);
        }
    };

    let pool_items = match list_one_resource(client, &metallb_ip_pool_resource()).await {
        Ok(listed) => listed.into_items(),
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
    let peer_items = list_one_resource(client, &calico_bgp_peer_resource())
        .await?
        .into_items();
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
        // On the wire `nodeSelector` is a Calico selector expression — a
        // string such as `rack == 'rack2'` or `has(rack)` — evaluated against
        // the node's labels, which in a Kubernetes-backed Calico are the Node's
        // own. A hand-written object in the Kubernetes `LabelSelector` shape is
        // accepted too. An expression this parser does not understand keeps
        // the global reading rather than dropping rows for sessions that may
        // well exist.
        let targets: Vec<String> = match spec.get("node").and_then(|v| v.as_str()) {
            Some(node) if node_labels.contains_key(node) => vec![node.to_string()],
            // A peer pinned to a node this cluster does not have peers nowhere.
            Some(_) => Vec::new(),
            None => match spec.get("nodeSelector") {
                Some(Value::String(expr)) => calico_selected_nodes(expr, node_labels),
                other => find_matching_nodes(other, node_labels),
            },
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
// Calico selector expressions
// ---------------------------------------------------------------------------

/// The nodes a Calico selector expression picks out, sorted by name. An
/// expression that does not parse selects every node — the global reading
/// the caller would have used had the field been absent.
fn calico_selected_nodes(
    expr: &str,
    node_labels: &HashMap<String, BTreeMap<String, String>>,
) -> Vec<String> {
    let Some(selector) = CalicoSelector::parse(expr) else {
        return node_labels.keys().cloned().collect();
    };
    let mut out: Vec<String> = node_labels
        .iter()
        .filter(|(_, labels)| selector.matches(labels))
        .map(|(name, _)| name.clone())
        .collect();
    out.sort();
    out
}

/// A parsed Calico selector expression.
///
/// The grammar is Calico's own (`all()`, `has(k)`, `k == 'v'`, `k != 'v'`,
/// `k in {'a', 'b'}`, `k not in {…}`, `k contains 'v'`, `k starts with 'v'`,
/// `k ends with 'v'`, joined with `&&`, `||`, `!` and parentheses). Label
/// values are quoted with `'` or `"`; keys are bare and may carry `.`, `/`,
/// `-` and `_`. Anything else fails to parse rather than being guessed at.
#[derive(Debug, Clone, PartialEq)]
enum CalicoSelector {
    All,
    Has(String),
    Eq(String, String),
    Ne(String, String),
    In(String, Vec<String>),
    NotIn(String, Vec<String>),
    Contains(String, String),
    StartsWith(String, String),
    EndsWith(String, String),
    Not(Box<CalicoSelector>),
    And(Box<CalicoSelector>, Box<CalicoSelector>),
    Or(Box<CalicoSelector>, Box<CalicoSelector>),
}

impl CalicoSelector {
    fn parse(expr: &str) -> Option<Self> {
        let mut p = CalicoParser { src: expr, pos: 0 };
        let sel = p.or_expr()?;
        p.skip_ws();
        if p.pos == p.src.len() {
            Some(sel)
        } else {
            None
        }
    }

    fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        match self {
            CalicoSelector::All => true,
            CalicoSelector::Has(k) => labels.contains_key(k),
            CalicoSelector::Eq(k, v) => labels.get(k) == Some(v),
            CalicoSelector::Ne(k, v) => labels.get(k) != Some(v),
            CalicoSelector::In(k, vs) => labels.get(k).is_some_and(|v| vs.contains(v)),
            CalicoSelector::NotIn(k, vs) => !labels.get(k).is_some_and(|v| vs.contains(v)),
            CalicoSelector::Contains(k, v) => labels.get(k).is_some_and(|l| l.contains(v)),
            CalicoSelector::StartsWith(k, v) => labels.get(k).is_some_and(|l| l.starts_with(v)),
            CalicoSelector::EndsWith(k, v) => labels.get(k).is_some_and(|l| l.ends_with(v)),
            CalicoSelector::Not(inner) => !inner.matches(labels),
            CalicoSelector::And(a, b) => a.matches(labels) && b.matches(labels),
            CalicoSelector::Or(a, b) => a.matches(labels) || b.matches(labels),
        }
    }
}

/// A recursive-descent parser over one selector expression. Every method
/// answers `None` for input it does not understand, and the whole parse
/// fails with it.
struct CalicoParser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> CalicoParser<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn skip_ws(&mut self) {
        let trimmed = self.rest().trim_start();
        self.pos = self.src.len() - trimmed.len();
    }

    fn eat(&mut self, token: &str) -> bool {
        self.skip_ws();
        if self.rest().starts_with(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    /// A keyword: the token followed by something that cannot continue an
    /// identifier, so `in` does not match the start of `internal`.
    fn eat_keyword(&mut self, word: &str) -> bool {
        self.skip_ws();
        let rest = self.rest();
        if rest.starts_with(word)
            && !rest[word.len()..]
                .chars()
                .next()
                .is_some_and(is_calico_key_char)
        {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn or_expr(&mut self) -> Option<CalicoSelector> {
        let mut left = self.and_expr()?;
        while self.eat("||") {
            let right = self.and_expr()?;
            left = CalicoSelector::Or(Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn and_expr(&mut self) -> Option<CalicoSelector> {
        let mut left = self.not_expr()?;
        while self.eat("&&") {
            let right = self.not_expr()?;
            left = CalicoSelector::And(Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn not_expr(&mut self) -> Option<CalicoSelector> {
        if self.eat("!") {
            return Some(CalicoSelector::Not(Box::new(self.not_expr()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Option<CalicoSelector> {
        if self.eat("(") {
            let inner = self.or_expr()?;
            return self.eat(")").then_some(inner);
        }
        if self.eat_keyword("all") {
            return (self.eat("(") && self.eat(")")).then_some(CalicoSelector::All);
        }
        if self.eat_keyword("has") {
            if !self.eat("(") {
                return None;
            }
            let key = self.key()?;
            return self.eat(")").then_some(CalicoSelector::Has(key));
        }
        let key = self.key()?;
        if self.eat("==") {
            return Some(CalicoSelector::Eq(key, self.value()?));
        }
        if self.eat("!=") {
            return Some(CalicoSelector::Ne(key, self.value()?));
        }
        if self.eat_keyword("not") {
            return self
                .eat_keyword("in")
                .then(|| self.set())
                .flatten()
                .map(|set| CalicoSelector::NotIn(key, set));
        }
        if self.eat_keyword("in") {
            return Some(CalicoSelector::In(key, self.set()?));
        }
        if self.eat_keyword("contains") {
            return Some(CalicoSelector::Contains(key, self.value()?));
        }
        if self.eat_keyword("starts") {
            return self
                .eat_keyword("with")
                .then(|| self.value())
                .flatten()
                .map(|v| CalicoSelector::StartsWith(key, v));
        }
        if self.eat_keyword("ends") {
            return self
                .eat_keyword("with")
                .then(|| self.value())
                .flatten()
                .map(|v| CalicoSelector::EndsWith(key, v));
        }
        None
    }

    fn key(&mut self) -> Option<String> {
        self.skip_ws();
        let rest = self.rest();
        let len = rest
            .char_indices()
            .find(|(_, c)| !is_calico_key_char(*c))
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        if len == 0 {
            return None;
        }
        self.pos += len;
        Some(rest[..len].to_string())
    }

    fn value(&mut self) -> Option<String> {
        self.skip_ws();
        let mut chars = self.rest().char_indices();
        let (_, quote) = chars.next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        let (end, _) = chars.find(|(_, c)| *c == quote)?;
        let value = self.rest()[1..end].to_string();
        self.pos += end + 1;
        Some(value)
    }

    fn set(&mut self) -> Option<Vec<String>> {
        if !self.eat("{") {
            return None;
        }
        let mut values = Vec::new();
        if self.eat("}") {
            return Some(values);
        }
        loop {
            values.push(self.value()?);
            if self.eat(",") {
                continue;
            }
            return self.eat("}").then_some(values);
        }
    }
}

fn is_calico_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')
}

// ---------------------------------------------------------------------------
// Selector Matching
// ---------------------------------------------------------------------------

/// The Service selectors carried by a set of `CiliumBGPAdvertisement`
/// objects: one entry per `advertisementType: Service` block whose
/// `service.addresses` includes `LoadBalancerIP`. A block announcing only
/// `ClusterIP` or `ExternalIP` addresses exports no VIP and contributes
/// nothing. A `None` entry is a Service block with no selector, which takes
/// every Service; `Some(vec![])` is advertisements that announce no VIP — a
/// `PodCIDR`-only set.
///
/// The result is `None` when an advertisement cannot be read: `spec.
/// advertisements` is not an array, or a Service block does not say which
/// addresses it announces. Cilium requires both, so such an object is not a
/// valid empty advertisement, and reading it as one would narrow the Services
/// tab on the strength of a manifest we did not understand.
fn service_advertisement_selectors(advs: &[&DynamicObject]) -> Option<Vec<Option<Value>>> {
    let mut selectors = Vec::new();
    for adv in advs {
        let blocks = adv
            .data
            .get("spec")
            .and_then(|s| s.get("advertisements"))
            .and_then(|v| v.as_array())?;
        for block in blocks {
            let kind = block
                .get("advertisementType")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !kind.eq_ignore_ascii_case("service") {
                continue;
            }
            let addresses = block
                .get("service")
                .and_then(|s| s.get("addresses"))
                .and_then(|v| v.as_array())?;
            let announces_lb_ip = addresses
                .iter()
                .filter_map(|a| a.as_str())
                .any(|a| a.eq_ignore_ascii_case("LoadBalancerIP"));
            if announces_lb_ip {
                selectors.push(block.get("selector").cloned());
            }
        }
    }
    Some(selectors)
}

/// Whether a set of advertisements announces PodCIDRs. An advertisement that
/// cannot be read keeps the long-standing assumption that it does.
fn advertises_pod_cidr(advs: &[&DynamicObject]) -> bool {
    advs.iter().any(|adv| {
        match adv
            .data
            .get("spec")
            .and_then(|s| s.get("advertisements"))
            .and_then(|v| v.as_array())
        {
            Some(blocks) => blocks.iter().any(|block| {
                block
                    .get("advertisementType")
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| {
                        t.eq_ignore_ascii_case("podcidr") || t.eq_ignore_ascii_case("pod")
                    })
            }),
            None => true,
        }
    })
}

/// The LoadBalancer Services a set of selectors picks out. `None` — nothing
/// readable was said about what is advertised — returns every Service, as
/// before: an absent advertisement is not evidence either way.
fn select_lb_services<'a>(
    selectors: Option<&[Option<Value>]>,
    lb_services: &'a [LbService],
) -> Vec<&'a LbService> {
    let Some(selectors) = selectors else {
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
            advertisement_selectors: Vec::new(),
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
                "gracefulRestart": {"enabled": true},
                // The family selects the advertisement below by label; an
                // advertisement no peer config selects reaches no session.
                "families": [{
                    "afi": "ipv4", "safi": "unicast",
                    "advertisements": {"matchLabels": {"advertise": "bgp"}}
                }]
            }
        });

        let mut adv_obj = DynamicObject::new("adv-1", &cilium_bgp_advertisement_v2_resource());
        adv_obj.metadata.labels = Some(BTreeMap::from([(
            "advertise".to_string(),
            "bgp".to_string(),
        )]));
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

    /// A fake API server answering each request path with `answer(path)`.
    fn cluster_answering<F>(answer: F) -> kube::Client
    where
        F: Fn(&str) -> (u16, Value) + Send + Sync + 'static,
    {
        let answer = std::sync::Arc::new(answer);
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let path = request.uri().path().to_owned();
            let answer = answer.clone();
            async move {
                let (status, body) = answer(&path);
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

    fn one_worker_node_list() -> Value {
        json!({"apiVersion":"v1","kind":"NodeList","metadata":{},"items":[
            {"apiVersion":"v1","kind":"Node",
             "metadata":{"name":"node-1","labels":{"role":"worker"}},
             "spec":{"podCIDR":"10.244.0.0/24"}}]})
    }

    fn empty_list(kind: &str) -> Value {
        json!({"apiVersion":"v1","kind":kind,"metadata":{},"items":[]})
    }

    fn status(code: u16) -> (u16, Value) {
        let reason = match code {
            404 => "NotFound",
            403 => "Forbidden",
            _ => "Failure",
        };
        (
            code,
            json!({"apiVersion":"v1","kind":"Status","status":"Failure",
                "code":code,"reason":reason,"message":"rejected"}),
        )
    }

    /// A cluster that serves Nodes and Services, and answers every CRD list
    /// with `crd_status`.
    fn cluster_answering_crds_with(crd_status: u16) -> kube::Client {
        cluster_answering(move |path| match path {
            "/api/v1/nodes" => (200, one_worker_node_list()),
            "/api/v1/services" => (200, empty_list("ServiceList")),
            _ => status(crd_status),
        })
    }

    #[tokio::test]
    async fn a_refused_served_version_beside_an_unserved_one_is_a_discovery_failure() {
        // The cluster installs the Cilium CRDs at v2alpha1 only, and RBAC
        // denies listing them: v2 answers 404, v2alpha1 answers 403. The one
        // version that could have answered refused, so nothing has been
        // learned about the cluster's BGP.
        let summary = fetch_bgp_summary(&cluster_answering(|path| match path {
            "/api/v1/nodes" => (200, one_worker_node_list()),
            "/api/v1/services" => (200, empty_list("ServiceList")),
            p if p.starts_with("/apis/cilium.io/v2alpha1/") => status(403),
            _ => status(404),
        }))
        .await
        .expect("nodes and services were readable");

        assert_eq!(summary.engine, BgpEngineType::None);
        let err = summary
            .error
            .expect("a 404 on the unserved version must not hide the refusal on the served one");
        assert!(
            err.contains("CiliumLoadBalancerIPPool") && err.contains("403"),
            "error should name the refused lookup, got {err}"
        );

        // And the mirror image: v2 refused, v2alpha1 not served.
        let summary = fetch_bgp_summary(&cluster_answering(|path| match path {
            "/api/v1/nodes" => (200, one_worker_node_list()),
            "/api/v1/services" => (200, empty_list("ServiceList")),
            p if p.starts_with("/apis/cilium.io/v2/") => status(403),
            _ => status(404),
        }))
        .await
        .expect("nodes and services were readable");
        assert!(summary.error.is_some(), "the refused v2 must surface");
    }

    #[tokio::test]
    async fn a_metallb_peer_that_matches_no_node_still_reports_metallb() {
        // The engine is decided by the objects the cluster serves, not by how
        // many peers they resolve to: a BGPPeer whose node selector matches
        // nothing is MetalLB configured to peer nowhere, not "no engine".
        let summary = fetch_bgp_summary(&cluster_answering(|path| match path {
            "/api/v1/nodes" => (200, one_worker_node_list()),
            "/api/v1/services" => (200, empty_list("ServiceList")),
            "/apis/metallb.io/v1beta2/bgppeers" => (
                200,
                json!({"apiVersion":"metallb.io/v1beta2","kind":"BGPPeerList","metadata":{},
                    "items":[{"apiVersion":"metallb.io/v1beta2","kind":"BGPPeer",
                        "metadata":{"name":"rack-9","namespace":"metallb-system"},
                        "spec":{"peerAddress":"10.0.0.1","peerASN":64512,"myASN":64512,
                                "nodeSelectors":[{"matchLabels":{"rack":"nowhere"}}]}}]}),
            ),
            "/apis/metallb.io/v1beta1/ipaddresspools" => (200, empty_list("IPAddressPoolList")),
            _ => status(404),
        }))
        .await
        .expect("nodes and services were readable");

        assert_eq!(summary.engine, BgpEngineType::MetalLB);
        assert_eq!(summary.total_peers, 0);
        assert_eq!(summary.total_nodes, 1);
        assert_eq!(summary.error, None);
    }

    #[test]
    fn a_successful_empty_advertisement_list_leaves_a_resolved_peer_silent() {
        // The API server answered: there are no CiliumBGPAdvertisement objects.
        // A peer whose config selects advertisements by label selects none.
        let node_labels = one_node_labels();
        let lb_services = vec![lb("web", "default", "1.2.3.4", None)];

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![peer_config_selecting("cfg-public", ("advertise", "public"))],
            vec![],
            vec![cluster_config_with_peer_config("cfg-public")],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );

        assert!(
            summary.advertised_services.is_empty(),
            "got {:?}",
            summary.advertised_services
        );
        assert_eq!(summary.peers[0].routes_count, 0);
        assert!(!summary.peers[0].export_pod_cidr);
    }

    #[test]
    fn a_failed_advertisement_lookup_keeps_the_union_for_a_resolved_peer() {
        // The list was refused: nothing is known about what is advertised,
        // and a resolved config must not turn that into "announces nothing".
        let node_labels = one_node_labels();
        let lb_services = vec![lb("web", "default", "1.2.3.4", None)];
        let mut node_pod_cidrs = HashMap::new();
        node_pod_cidrs.insert("node-1".to_string(), vec!["10.244.0.0/24".to_string()]);

        let summary = build_cilium_bgp_summary_from(
            vec![],
            vec![peer_config_selecting("cfg-public", ("advertise", "public"))],
            AdvertisementLookup::Failed,
            vec![cluster_config_with_peer_config("cfg-public")],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &node_pod_cidrs,
            &lb_services,
        );

        assert_eq!(summary.advertised_services.len(), 1, "the union stands in");
        assert!(
            summary.peers[0].export_pod_cidr,
            "the PodCIDR default stands in"
        );
        assert_eq!(summary.peers[0].routes_count, 2, "one PodCIDR, one VIP");
    }

    #[tokio::test]
    async fn a_refused_advertisement_list_is_reported_and_does_not_silence_the_peers() {
        // End to end: cluster config and peer config are readable, the
        // advertisement list is refused. The header warns, and the Services
        // tab keeps the union rather than going empty.
        let summary = fetch_bgp_summary(&cluster_answering(|path| match path {
            "/api/v1/nodes" => (200, one_worker_node_list()),
            "/api/v1/services" => (
                200,
                json!({"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[
                    {"apiVersion":"v1","kind":"Service",
                     "metadata":{"name":"web","namespace":"default"},
                     "spec":{"type":"LoadBalancer"},
                     "status":{"loadBalancer":{"ingress":[{"ip":"1.2.3.4"}]}}}]}),
            ),
            "/apis/cilium.io/v2/ciliumbgpclusterconfigs" => (
                200,
                json!({"apiVersion":"cilium.io/v2","kind":"CiliumBGPClusterConfigList",
                    "metadata":{},"items":[{"apiVersion":"cilium.io/v2",
                    "kind":"CiliumBGPClusterConfig","metadata":{"name":"cluster-cfg"},
                    "spec":{"bgpInstances":[{"localASN":65000,"peers":[{
                        "peerAddress":"172.16.1.1","peerASN":65001,
                        "peerConfigRef":{"name":"cfg-public"}}]}]}}]}),
            ),
            "/apis/cilium.io/v2/ciliumbgppeerconfigs" => (
                200,
                json!({"apiVersion":"cilium.io/v2","kind":"CiliumBGPPeerConfigList",
                    "metadata":{},"items":[{"apiVersion":"cilium.io/v2",
                    "kind":"CiliumBGPPeerConfig","metadata":{"name":"cfg-public"},
                    "spec":{"families":[{"afi":"ipv4","safi":"unicast",
                        "advertisements":{"matchLabels":{"advertise":"public"}}}]}}]}),
            ),
            "/apis/cilium.io/v2/ciliumbgpadvertisements" => status(403),
            _ => status(404),
        }))
        .await
        .expect("nodes and services were readable");

        assert_eq!(summary.engine, BgpEngineType::CiliumV2);
        let err = summary
            .error
            .as_deref()
            .expect("the refused advertisement list must be reported");
        assert!(err.contains("CiliumBGPAdvertisement"), "{err}");
        assert_eq!(summary.peers.len(), 1);
        assert_eq!(
            summary.advertised_services.len(),
            1,
            "a refused list must not empty the Services tab"
        );
        assert!(summary.peers[0].export_pod_cidr);
    }

    fn labelled_advertisement(name: &str, label: (&str, &str), blocks: Value) -> DynamicObject {
        let mut adv = advertisement(name, blocks);
        adv.metadata.labels = Some(BTreeMap::from([(label.0.to_string(), label.1.to_string())]));
        adv
    }

    fn peer_config_selecting(name: &str, label: (&str, &str)) -> DynamicObject {
        let mut cfg = DynamicObject::new(name, &cilium_bgp_peer_config_v2_resource());
        cfg.data = json!({
            "spec": {
                "families": [{
                    "afi": "ipv4", "safi": "unicast",
                    "advertisements": {"matchLabels": {label.0: label.1}}
                }]
            }
        });
        cfg
    }

    fn cluster_config_with_peer_config(peer_cfg: &str) -> DynamicObject {
        let mut cluster_cfg =
            DynamicObject::new("cluster-cfg", &cilium_bgp_cluster_config_v2_resource());
        cluster_cfg.data = json!({
            "spec": {
                "bgpInstances": [{
                    "localASN": 65000,
                    "peers": [{
                        "peerAddress": "172.16.1.1",
                        "peerASN": 65001,
                        "peerConfigRef": {"name": peer_cfg}
                    }]
                }]
            }
        });
        cluster_cfg
    }

    #[test]
    fn an_advertisement_no_peer_config_selects_announces_nothing() {
        // Two Service advertisements exist; the peer's config selects only
        // the one labelled `advertise=public`. The other reaches no session,
        // so its Services are not advertised and do not count as routes.
        let node_labels = one_node_labels();
        let lb_services = vec![
            lb_labelled("public-lb", "1.2.3.4", "tier", "public"),
            lb_labelled("internal-lb", "10.0.0.9", "tier", "internal"),
        ];
        let service_block = |tier: &str| {
            json!([{
                "advertisementType": "Service",
                "service": {"addresses": ["LoadBalancerIP"]},
                "selector": {"matchLabels": {"tier": tier}}
            }])
        };

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![peer_config_selecting("cfg-public", ("advertise", "public"))],
            vec![
                labelled_advertisement(
                    "adv-public",
                    ("advertise", "public"),
                    service_block("public"),
                ),
                labelled_advertisement(
                    "adv-internal",
                    ("advertise", "internal"),
                    service_block("internal"),
                ),
            ],
            vec![cluster_config_with_peer_config("cfg-public")],
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
        assert_eq!(
            names,
            ["public-lb"],
            "only the selected advertisement announces"
        );
        assert_eq!(summary.peers.len(), 1);
        assert_eq!(summary.peers[0].routes_count, 1, "one VIP, no PodCIDR");
        assert!(!summary.peers[0].export_pod_cidr);
        let row = &summary.advertised_services[0];
        assert_eq!(row.announcing_nodes, ["node-1"]);
        assert_eq!(row.peers, ["172.16.1.1:65001"]);
    }

    #[test]
    fn a_peer_whose_config_selects_no_advertisement_announces_nothing() {
        let node_labels = one_node_labels();
        let lb_services = vec![lb("web", "default", "1.2.3.4", None)];
        // A config with a family that names no advertisements selector.
        let mut cfg = DynamicObject::new("cfg-quiet", &cilium_bgp_peer_config_v2_resource());
        cfg.data = json!({"spec": {"families": [{"afi": "ipv4", "safi": "unicast"}]}});

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![cfg],
            vec![labelled_advertisement(
                "adv",
                ("advertise", "all"),
                json!([{"advertisementType": "Service",
                        "service": {"addresses": ["LoadBalancerIP"]}}]),
            )],
            vec![cluster_config_with_peer_config("cfg-quiet")],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );

        assert!(summary.advertised_services.is_empty());
        assert_eq!(summary.peers[0].routes_count, 0);
    }

    #[test]
    fn a_service_block_announcing_no_load_balancer_ip_advertises_no_vip() {
        // `service.addresses` may name only ClusterIP or ExternalIP; such a
        // block exports no LoadBalancer VIP.
        let node_labels = one_node_labels();
        let lb_services = vec![lb("web", "default", "1.2.3.4", None)];

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![advertisement(
                "adv-clusterip",
                json!([{"advertisementType": "Service",
                        "service": {"addresses": ["ClusterIP", "ExternalIP"]}}]),
            )],
            vec![one_node_cluster_config()],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );

        assert!(
            summary.advertised_services.is_empty(),
            "got {:?}",
            summary.advertised_services
        );
        assert_eq!(summary.peers[0].routes_count, 0);
    }

    #[test]
    fn an_unreadable_advertisement_does_not_narrow_the_services() {
        // Cilium requires `spec.advertisements`; an object without it is not
        // a valid empty advertisement, so nothing is claimed from it.
        let node_labels = one_node_labels();
        let lb_services = vec![lb("web", "default", "1.2.3.4", None)];
        let mut malformed = DynamicObject::new("adv-odd", &cilium_bgp_advertisement_v2_resource());
        malformed.data = json!({"spec": {}});

        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![malformed],
            vec![one_node_cluster_config()],
            vec![],
            vec![],
            vec![],
            &node_labels,
            &HashMap::new(),
            &lb_services,
        );
        assert_eq!(summary.advertised_services.len(), 1);

        // Likewise a Service block that does not say which addresses it
        // announces.
        let summary = build_cilium_bgp_summary(
            vec![],
            vec![],
            vec![advertisement(
                "adv-no-addresses",
                json!([{"advertisementType": "Service"}]),
            )],
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

    #[test]
    fn a_calico_selector_expression_scopes_the_peer_to_the_nodes_it_names() {
        let mut node_labels = HashMap::new();
        node_labels.insert(
            "rack1-host1".to_string(),
            BTreeMap::from([("rack".to_string(), "rack1".to_string())]),
        );
        node_labels.insert(
            "rack2-host1".to_string(),
            BTreeMap::from([("rack".to_string(), "rack2".to_string())]),
        );
        node_labels.insert("unracked".to_string(), BTreeMap::new());

        let nodes_for = |expr: &str| -> Vec<String> {
            let mut peer = DynamicObject::new("peer", &calico_bgp_peer_resource());
            peer.data = json!({
                "spec": {"nodeSelector": expr, "peerIP": "10.0.0.1", "asNumber": 64512}
            });
            build_calico_bgp_summary(vec![peer], &node_labels)
                .peers
                .iter()
                .map(|p| p.node_name.clone())
                .collect()
        };

        assert_eq!(nodes_for("rack == 'rack2'"), ["rack2-host1"]);
        assert_eq!(nodes_for("rack != \"rack2\""), ["rack1-host1", "unracked"]);
        assert_eq!(nodes_for("has(rack)"), ["rack1-host1", "rack2-host1"]);
        assert_eq!(nodes_for("!has(rack)"), ["unracked"]);
        assert_eq!(
            nodes_for("rack in {'rack1', 'rack2'}"),
            ["rack1-host1", "rack2-host1"]
        );
        assert_eq!(
            nodes_for("rack not in {'rack1'}"),
            ["rack2-host1", "unracked"]
        );
        assert_eq!(
            nodes_for("has(rack) && rack starts with 'rack2' || !has(rack)"),
            ["rack2-host1", "unracked"]
        );
        assert_eq!(nodes_for("all()").len(), 3, "all() is every node");
        // An expression this parser does not understand keeps the global
        // reading rather than dropping sessions that may exist.
        assert_eq!(nodes_for("rack === 'rack2'").len(), 3);
        assert_eq!(
            nodes_for("internal == 'x'"),
            Vec::<String>::new(),
            "`in` is not `internal`"
        );
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
