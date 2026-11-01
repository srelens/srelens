//! Compute and estimated cost for one cluster, from what the Kubernetes API
//! already says: node allocatable, what pods request, and (optionally) live
//! usage from metrics-server.
//!
//! The cost here is an ESTIMATE and is meant to be read as one. It prices the
//! CPU and memory the nodes offer (what you pay for) and the CPU and memory
//! pods request (what is reserved), at per-hour rates the caller supplies. It
//! knows nothing about the instance type's real price, discounts, spot
//! pricing, network or storage. A cost tool that does (Kubecost, OpenCost) is
//! preferred when one is present; see [`crate::opencost`].

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Node, Pod};

use crate::metrics::{cpu_millicores, mem_mib, NodeMetric};
use crate::why_pending::calculate_pod_resource_requests;

/// Hours in an average month, the usual cloud-billing convention.
pub const HOURS_PER_MONTH: f64 = 730.0;

/// Price of one unit of compute for one hour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Per CPU core-hour.
    pub cpu_core_hour: f64,
    /// Per GiB-hour of memory.
    pub mem_gib_hour: f64,
}

impl Default for Rates {
    /// Placeholder rates in the range public clouds charge for general-purpose
    /// compute. They are a starting point to be replaced with the user's own.
    fn default() -> Self {
        Self { cpu_core_hour: 0.031611, mem_gib_hour: 0.004237 }
    }
}

impl Rates {
    /// Hourly price of `cpu_millicores` and `mem_mib`.
    pub fn hourly(&self, cpu_millicores: i64, mem_mib: i64) -> f64 {
        (cpu_millicores.max(0) as f64 / 1000.0) * self.cpu_core_hour
            + (mem_mib.max(0) as f64 / 1024.0) * self.mem_gib_hour
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeCompute {
    pub name: String,
    pub instance_type: Option<String>,
    pub zone: Option<String>,
    pub allocatable_cpu_m: i64,
    pub allocatable_mem_mib: i64,
    pub requested_cpu_m: i64,
    pub requested_mem_mib: i64,
    /// Live usage; `None` when metrics-server did not report this node.
    pub usage_cpu_m: Option<i64>,
    pub usage_mem_mib: Option<i64>,
    pub pods: usize,
    /// What the node's allocatable compute costs per hour at the rates.
    pub hourly: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamespaceCompute {
    pub namespace: String,
    pub pods: usize,
    pub requested_cpu_m: i64,
    pub requested_mem_mib: i64,
    /// The cost of what the namespace's pods request, per hour.
    pub hourly: f64,
}

impl NamespaceCompute {
    pub fn monthly(&self) -> f64 {
        self.hourly * HOURS_PER_MONTH
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClusterCompute {
    pub rates: Rates,
    pub nodes: Vec<NodeCompute>,
    /// Largest requester first.
    pub namespaces: Vec<NamespaceCompute>,
    pub pods: usize,
    pub allocatable_cpu_m: i64,
    pub allocatable_mem_mib: i64,
    pub requested_cpu_m: i64,
    pub requested_mem_mib: i64,
    /// Summed over the nodes that reported; `None` when none did.
    pub usage_cpu_m: Option<i64>,
    pub usage_mem_mib: Option<i64>,
    /// What the nodes cost per hour.
    pub cluster_hourly: f64,
    /// What the pods' requests cost per hour (the part of the cluster that is
    /// spoken for).
    pub requested_hourly: f64,
    /// The node or pod list hit the list limit, so these are partial.
    pub truncated: bool,
    /// Why live usage is missing, when it is (metrics-server absent or slow).
    pub usage_note: Option<String>,
    /// Real spend from Kubecost or OpenCost, when one is installed and answered.
    pub provider: Option<crate::opencost::ProviderCosts>,
    /// Why `provider` is missing when one IS installed (it did not answer).
    /// `None` with no provider means none was found.
    pub provider_note: Option<String>,
}

impl ClusterCompute {
    pub fn cluster_monthly(&self) -> f64 {
        self.cluster_hourly * HOURS_PER_MONTH
    }

    pub fn requested_monthly(&self) -> f64 {
        self.requested_hourly * HOURS_PER_MONTH
    }

    /// Paid for but not requested by any pod. Never negative: a cluster whose
    /// pods request more than the nodes offer has no idle share.
    pub fn idle_hourly(&self) -> f64 {
        (self.cluster_hourly - self.requested_hourly).max(0.0)
    }

    pub fn idle_monthly(&self) -> f64 {
        self.idle_hourly() * HOURS_PER_MONTH
    }

    /// Share of the nodes' cost that pods have requested, 0-100.
    pub fn requested_percent(&self) -> f64 {
        if self.cluster_hourly <= 0.0 {
            return 0.0;
        }
        (self.requested_hourly / self.cluster_hourly * 100.0).min(100.0)
    }
}

fn label<'a>(node: &'a Node, keys: &[&str]) -> Option<&'a str> {
    let labels = node.metadata.labels.as_ref()?;
    keys.iter().find_map(|k| labels.get(*k).map(String::as_str))
}

fn allocatable(node: &Node) -> (i64, i64) {
    let alloc = node.status.as_ref().and_then(|s| s.allocatable.as_ref());
    let get = |key: &str| alloc.and_then(|a| a.get(key)).map(|q| q.0.as_str());
    (
        get("cpu").map(cpu_millicores).unwrap_or(0),
        get("memory").map(mem_mib).unwrap_or(0),
    )
}

/// Pods that hold resources on a node: placed, and not finished.
fn is_running_on_a_node(pod: &Pod) -> bool {
    let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref()).unwrap_or("");
    let placed = pod
        .spec
        .as_ref()
        .and_then(|s| s.node_name.as_deref())
        .is_some_and(|n| !n.is_empty());
    placed && phase != "Succeeded" && phase != "Failed"
}

/// Summarise `nodes` and `pods`. `usage` is metrics-server's node metrics, if
/// it answered.
pub fn summarize(
    nodes: &[Node],
    pods: &[Pod],
    usage: Option<&[NodeMetric]>,
    rates: Rates,
) -> ClusterCompute {
    let usage_by_node: BTreeMap<&str, &NodeMetric> = usage
        .unwrap_or(&[])
        .iter()
        .map(|m| (m.name.as_str(), m))
        .collect();

    let mut by_node: BTreeMap<String, (i64, i64, usize)> = BTreeMap::new();
    let mut by_namespace: BTreeMap<String, (i64, i64, usize)> = BTreeMap::new();
    for pod in pods.iter().filter(|p| is_running_on_a_node(p)) {
        let req = calculate_pod_resource_requests(pod.spec.as_ref());
        let node = pod.spec.as_ref().and_then(|s| s.node_name.clone()).unwrap_or_default();
        let ns = pod.metadata.namespace.clone().unwrap_or_else(|| "default".to_string());
        for entry in [by_node.entry(node).or_default(), by_namespace.entry(ns).or_default()] {
            entry.0 += req.cpu_millicores;
            entry.1 += req.memory_mib;
            entry.2 += 1;
        }
    }

    let mut node_rows: Vec<NodeCompute> = nodes
        .iter()
        .map(|n| {
            let name = n.metadata.name.clone().unwrap_or_default();
            let (cpu, mem) = allocatable(n);
            let (req_cpu, req_mem, pod_count) = by_node.get(&name).copied().unwrap_or_default();
            let live = usage_by_node.get(name.as_str());
            NodeCompute {
                instance_type: label(
                    n,
                    &["node.kubernetes.io/instance-type", "beta.kubernetes.io/instance-type"],
                )
                .map(String::from),
                zone: label(n, &["topology.kubernetes.io/zone", "failure-domain.beta.kubernetes.io/zone"])
                    .map(String::from),
                allocatable_cpu_m: cpu,
                allocatable_mem_mib: mem,
                requested_cpu_m: req_cpu,
                requested_mem_mib: req_mem,
                usage_cpu_m: live.map(|m| m.cpu_millicores),
                usage_mem_mib: live.map(|m| m.memory_mib),
                pods: pod_count,
                hourly: rates.hourly(cpu, mem),
                name,
            }
        })
        .collect();
    node_rows.sort_by(|a, b| a.name.cmp(&b.name));

    let mut namespace_rows: Vec<NamespaceCompute> = by_namespace
        .into_iter()
        .map(|(namespace, (cpu, mem, pods))| NamespaceCompute {
            hourly: rates.hourly(cpu, mem),
            namespace,
            pods,
            requested_cpu_m: cpu,
            requested_mem_mib: mem,
        })
        .collect();
    namespace_rows.sort_by(|a, b| {
        b.hourly
            .partial_cmp(&a.hourly)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.namespace.cmp(&b.namespace))
    });

    let reported: Vec<&NodeCompute> = node_rows.iter().filter(|n| n.usage_cpu_m.is_some()).collect();
    let requested_cpu_m: i64 = namespace_rows.iter().map(|n| n.requested_cpu_m).sum();
    let requested_mem_mib: i64 = namespace_rows.iter().map(|n| n.requested_mem_mib).sum();

    ClusterCompute {
        rates,
        pods: namespace_rows.iter().map(|n| n.pods).sum(),
        allocatable_cpu_m: node_rows.iter().map(|n| n.allocatable_cpu_m).sum(),
        allocatable_mem_mib: node_rows.iter().map(|n| n.allocatable_mem_mib).sum(),
        requested_cpu_m,
        requested_mem_mib,
        usage_cpu_m: (!reported.is_empty()).then(|| reported.iter().filter_map(|n| n.usage_cpu_m).sum()),
        usage_mem_mib: (!reported.is_empty()).then(|| reported.iter().filter_map(|n| n.usage_mem_mib).sum()),
        cluster_hourly: node_rows.iter().map(|n| n.hourly).sum(),
        requested_hourly: rates.hourly(requested_cpu_m, requested_mem_mib),
        truncated: false,
        usage_note: None,
        provider: None,
        provider_note: None,
        nodes: node_rows,
        namespaces: namespace_rows,
    }
}

/// What a failed metrics-server read means, in a sentence a person can act on.
/// The raw error is a debug dump of an `ApiError` and says nothing about the
/// cause.
pub fn friendly_usage_note(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("404")
        || lower.contains("not found")
        || lower.contains("notfound")
        || lower.contains("could not find")
    {
        return "metrics-server is not installed in this cluster (the metrics.k8s.io API was not found)"
            .to_string();
    }
    if lower.contains("forbidden") || lower.contains("403") {
        return "metrics-server is installed but you are not allowed to read node metrics".to_string();
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return "metrics-server timed out answering".to_string();
    }
    let first_line = raw.lines().next().unwrap_or(raw);
    let short: String = first_line.chars().take(120).collect();
    format!("metrics-server returned an error: {short}")
}

/// Read the nodes, pods and (if metrics-server answers) node usage of
/// `context` and summarise them.
pub async fn fetch_compute(
    cache: std::sync::Arc<crate::client_cache::ClientCache>,
    context: &str,
    rates: Rates,
) -> Result<ClusterCompute, String> {
    use kube::api::ListParams;
    use kube::Api;

    let client = cache.get(context).await.map_err(|e| e.to_string())?;
    let node_api: Api<Node> = Api::all(client.clone());
    let pod_api: Api<Pod> = Api::all(client.clone());
    let (nodes, nodes_cut) = crate::list_cap::list_capped(&node_api, ListParams::default())
        .await
        .map_err(|e| format!("Failed to list nodes: {e}"))?;
    let (pods, pods_cut) = crate::list_cap::list_capped(&pod_api, ListParams::default())
        .await
        .map_err(|e| format!("Failed to list pods: {e}"))?;

    let (usage, usage_note) = match crate::metrics::fetch_node_metrics(cache, context).await {
        Ok(m) => (Some(m), None),
        Err(e) => (None, Some(friendly_usage_note(&e))),
    };
    // Real spend is a bonus: its absence or failure never fails the dashboard.
    let (provider, provider_note) = match crate::opencost::fetch_costs(&client).await {
        Ok(found) => (found, None),
        Err(why) => (None, Some(why)),
    };
    let mut summary = summarize(&nodes, &pods, usage.as_deref(), rates);
    summary.truncated = nodes_cut || pods_cut;
    summary.usage_note = usage_note;
    summary.provider = provider;
    summary.provider_note = provider_note;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const R: Rates = Rates { cpu_core_hour: 0.04, mem_gib_hour: 0.005 };

    fn node(name: &str, cpu: &str, mem: &str, extra_labels: serde_json::Value) -> Node {
        serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Node",
            "metadata": {"name": name, "labels": extra_labels},
            "status": {"allocatable": {"cpu": cpu, "memory": mem}}
        }))
        .unwrap()
    }

    fn pod(ns: &str, name: &str, node: Option<&str>, phase: &str, cpu: &str, mem: &str) -> Pod {
        let mut spec = json!({"containers": [{"name": "c", "image": "i",
            "resources": {"requests": {"cpu": cpu, "memory": mem}}}]});
        if let Some(n) = node {
            spec["nodeName"] = json!(n);
        }
        serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": name, "namespace": ns},
            "spec": spec, "status": {"phase": phase}
        }))
        .unwrap()
    }

    fn metric(name: &str, cpu: i64, mem: i64) -> NodeMetric {
        NodeMetric { name: name.into(), cpu_millicores: cpu, memory_mib: mem }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn rates_price_cores_and_gib_per_hour() {
        // 2 cores * 0.04 + 4 GiB * 0.005
        assert!(close(R.hourly(2000, 4096), 0.08 + 0.02));
        assert!(close(R.hourly(0, 0), 0.0));
        assert!(close(R.hourly(-5, -5), 0.0), "a negative request is nothing, not a credit");
    }

    #[test]
    fn the_cluster_is_priced_by_what_its_nodes_offer() {
        let nodes = [node("a", "4", "16Gi", json!({})), node("b", "2", "8Gi", json!({}))];
        let c = summarize(&nodes, &[], None, R);
        assert_eq!((c.allocatable_cpu_m, c.allocatable_mem_mib), (6000, 24 * 1024));
        // 6 cores * 0.04 + 24 GiB * 0.005
        assert!(close(c.cluster_hourly, 0.24 + 0.12));
        assert!(close(c.cluster_monthly(), (0.24 + 0.12) * HOURS_PER_MONTH));
    }

    #[test]
    fn namespaces_are_priced_by_what_their_pods_request_largest_first() {
        let nodes = [node("a", "8", "32Gi", json!({}))];
        let pods = [
            pod("small", "p1", Some("a"), "Running", "100m", "128Mi"),
            pod("big", "p2", Some("a"), "Running", "2", "4Gi"),
            pod("big", "p3", Some("a"), "Running", "1", "2Gi"),
        ];
        let c = summarize(&nodes, &pods, None, R);
        assert_eq!(c.namespaces[0].namespace, "big");
        assert_eq!((c.namespaces[0].pods, c.namespaces[0].requested_cpu_m), (2, 3000));
        assert_eq!(c.namespaces[0].requested_mem_mib, 6 * 1024);
        assert!(close(c.namespaces[0].hourly, 0.12 + 0.03));
        assert_eq!(c.namespaces[1].namespace, "small");
        assert_eq!(c.pods, 3);
    }

    #[test]
    fn idle_is_what_is_paid_for_and_not_requested() {
        let nodes = [node("a", "4", "16Gi", json!({}))];
        let pods = [pod("ns", "p", Some("a"), "Running", "1", "4Gi")];
        let c = summarize(&nodes, &pods, None, R);
        assert!(close(c.requested_hourly, 0.04 + 0.02));
        assert!(close(c.idle_hourly(), (0.16 + 0.08) - (0.04 + 0.02)));
        assert!(close(c.requested_percent(), (0.06 / 0.24) * 100.0));
        assert!(close(c.idle_monthly(), c.idle_hourly() * HOURS_PER_MONTH));
    }

    #[test]
    fn a_cluster_requested_past_its_capacity_has_no_negative_idle() {
        let nodes = [node("a", "1", "1Gi", json!({}))];
        let pods = [pod("ns", "p", Some("a"), "Running", "4", "8Gi")];
        let c = summarize(&nodes, &pods, None, R);
        assert_eq!(c.idle_hourly(), 0.0);
        assert!(c.requested_percent() <= 100.0);
    }

    #[test]
    fn finished_and_unplaced_pods_hold_nothing() {
        let nodes = [node("a", "4", "16Gi", json!({}))];
        let pods = [
            pod("ns", "done", Some("a"), "Succeeded", "1", "1Gi"),
            pod("ns", "failed", Some("a"), "Failed", "1", "1Gi"),
            pod("ns", "pending", None, "Pending", "1", "1Gi"),
            pod("ns", "live", Some("a"), "Running", "500m", "512Mi"),
        ];
        let c = summarize(&nodes, &pods, None, R);
        assert_eq!(c.pods, 1);
        assert_eq!((c.requested_cpu_m, c.requested_mem_mib), (500, 512));
    }

    #[test]
    fn per_node_requests_pod_counts_and_labels_are_kept() {
        let nodes = [
            node(
                "a",
                "4",
                "16Gi",
                json!({"node.kubernetes.io/instance-type": "m5.xlarge", "topology.kubernetes.io/zone": "us-east-1a"}),
            ),
            node("b", "4", "16Gi", json!({})),
        ];
        let pods = [
            pod("x", "p1", Some("a"), "Running", "1", "1Gi"),
            pod("x", "p2", Some("a"), "Running", "1", "1Gi"),
            pod("x", "p3", Some("b"), "Running", "1", "1Gi"),
        ];
        let c = summarize(&nodes, &pods, None, R);
        assert_eq!(c.nodes[0].name, "a");
        assert_eq!((c.nodes[0].pods, c.nodes[0].requested_cpu_m), (2, 2000));
        assert_eq!(c.nodes[0].instance_type.as_deref(), Some("m5.xlarge"));
        assert_eq!(c.nodes[0].zone.as_deref(), Some("us-east-1a"));
        assert_eq!(c.nodes[1].instance_type, None);
        assert_eq!(c.nodes[1].pods, 1);
    }

    #[test]
    fn live_usage_is_attached_by_node_and_summed_over_those_that_reported() {
        let nodes = [node("a", "4", "16Gi", json!({})), node("b", "4", "16Gi", json!({}))];
        let usage = [metric("a", 1500, 6000)];
        let c = summarize(&nodes, &[], Some(&usage), R);
        assert_eq!((c.nodes[0].usage_cpu_m, c.nodes[0].usage_mem_mib), (Some(1500), Some(6000)));
        assert_eq!(c.nodes[1].usage_cpu_m, None, "b did not report, so it is unknown, not zero");
        assert_eq!((c.usage_cpu_m, c.usage_mem_mib), (Some(1500), Some(6000)));
    }

    #[test]
    fn without_metrics_the_usage_is_unknown_not_zero() {
        let nodes = [node("a", "4", "16Gi", json!({}))];
        let c = summarize(&nodes, &[], None, R);
        assert_eq!((c.usage_cpu_m, c.usage_mem_mib), (None, None));
        assert_eq!(c.nodes[0].usage_cpu_m, None);
    }

    #[test]
    fn an_empty_cluster_is_all_zeros_and_never_nan() {
        let c = summarize(&[], &[], None, R);
        assert_eq!((c.pods, c.cluster_hourly, c.idle_hourly(), c.requested_percent()), (0, 0.0, 0.0, 0.0));
        assert!(c.nodes.is_empty() && c.namespaces.is_empty());
    }

    #[test]
    fn a_node_with_no_allocatable_is_priced_at_nothing() {
        let bare: Node = serde_json::from_value(json!({"apiVersion": "v1", "kind": "Node", "metadata": {"name": "x"}})).unwrap();
        let c = summarize(&[bare], &[], None, R);
        assert_eq!((c.nodes[0].allocatable_cpu_m, c.cluster_hourly), (0, 0.0));
    }

    #[test]
    fn a_missing_metrics_api_is_explained_in_plain_words() {
        // The three shapes a cluster without metrics-server answers with.
        for raw in [
            "ApiError: 404 page not found : Failed to parse error data (Status { status: Some(Failure), code: 404, message: \"404 page not found\\n\" })",
            "the server could not find the requested resource",
            "NotFound",
        ] {
            let note = friendly_usage_note(raw);
            assert!(note.contains("not installed"), "{raw} -> {note}");
            assert!(!note.contains("ApiError") && !note.contains("Status {"), "{note}");
        }
    }

    #[test]
    fn other_metrics_failures_say_what_kind_of_failure_it_was() {
        assert!(friendly_usage_note("node metrics timed out").contains("timed out"));
        assert!(friendly_usage_note("nodes.metrics.k8s.io is forbidden: User cannot list").contains("not allowed"));
        let odd = friendly_usage_note("something unexpected happened");
        assert!(odd.contains("metrics-server") && odd.contains("something unexpected"));
    }

    #[test]
    fn a_long_unrecognised_error_is_shortened() {
        let long = "x".repeat(500);
        assert!(friendly_usage_note(&long).len() < 250);
    }

    #[test]
    fn a_summary_starts_complete_and_with_no_usage_note() {
        let c = summarize(&[], &[], None, R);
        assert!(!c.truncated);
        assert_eq!(c.usage_note, None);
        assert!(c.provider.is_none() && c.provider_note.is_none());
    }

    #[test]
    fn the_default_rates_are_positive_placeholders() {
        let r = Rates::default();
        assert!(r.cpu_core_hour > 0.0 && r.mem_gib_hour > 0.0);
    }
}
