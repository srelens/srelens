//! The `k8s.listNodes` capability (cluster-scoped).

use std::collections::HashMap;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{Node, Pod};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNodesIn {
    pub context: String,
}

/// One entry of `spec.taints` — the shape the Nodes list's badge tooltip and
/// the optional Taints column read. `value` is empty for the common valueless
/// taint (`node-role.kubernetes.io/control-plane:NoSchedule`), which is a real
/// value and not a missing one, so it is a `String` and not an `Option`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct NodeTaint {
    pub key: String,
    pub value: String,
    /// `NoSchedule`, `PreferNoSchedule` or `NoExecute`.
    pub effect: String,
    /// Set by Kubernetes only for `NoExecute` taints.
    #[serde(rename = "timeAdded", skip_serializing_if = "Option::is_none")]
    pub time_added: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct NodeSummary {
    pub name: String,
    /// Readiness derived from the `Ready` condition: "Ready", "NotReady", or "Unknown".
    pub status: String,
    /// Whether the node is cordoned (`spec.unschedulable`) — shown as "SchedulingDisabled".
    pub unschedulable: bool,
    /// Number of taints on the node, excluding the auto-added unschedulable taint.
    pub taints: u32,
    /// The taints `taints` counts, in the order the API server reports them.
    /// Deliberately the *same* filtered set as the count rather than the whole
    /// of `spec.taints`: the list's badge shows the count and its tooltip shows
    /// this, and a badge reading "2" over a tooltip listing three lines is a
    /// worse answer than leaving the cordon taint — already spelled out by the
    /// SchedulingDisabled badge beside it — to the detail page, which reads the
    /// live object and lists every taint.
    #[serde(rename = "taintDetails")]
    pub taint_details: Vec<NodeTaint>,
    pub version: String,
    pub roles: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// `status.allocatable.cpu`, converted to millicores — the unit metrics-server uses.
    #[serde(rename = "allocatableCpuMillicores")]
    pub allocatable_cpu_millicores: i64,
    /// `status.allocatable.memory`, converted to MiB — the unit metrics-server uses.
    #[serde(rename = "allocatableMemoryMiB")]
    pub allocatable_memory_mib: i64,
    /// `status.allocatable.pods`.
    #[serde(rename = "allocatablePods")]
    pub allocatable_pods: i64,
    /// The node's machine type, read from its `node.kubernetes.io/instance-type`
    /// label, falling back to the deprecated `beta.kubernetes.io/instance-type`
    /// when the modern one is absent. Empty when the node carries neither —
    /// e.g. on kind, whose nodes are containers rather than cloud machines —
    /// not a guessed or placeholder value.
    #[serde(rename = "instanceType")]
    pub instance_type: String,
    #[serde(rename = "internalIp", skip_serializing_if = "Option::is_none")]
    pub internal_ip: Option<String>,
    #[serde(rename = "externalIp", skip_serializing_if = "Option::is_none")]
    pub external_ip: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNodesOut {
    pub nodes: Vec<NodeSummary>,
}

pub fn summarise(node: Node) -> NodeSummary {
    let name = node.metadata.name.clone().unwrap_or_default();
    let status = node
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .map(|c| {
            if c.status == "True" {
                "Ready"
            } else {
                "NotReady"
            }
        })
        .unwrap_or("Unknown")
        .to_string();
    let version = node
        .status
        .as_ref()
        .and_then(|s| s.node_info.as_ref())
        .map(|i| i.kubelet_version.clone())
        .unwrap_or_default();
    let roles = node
        .metadata
        .labels
        .as_ref()
        .map(|labels| {
            let roles: Vec<String> = labels
                .keys()
                .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
                .filter(|r| !r.is_empty())
                .map(String::from)
                .collect();
            if roles.is_empty() {
                "<none>".to_string()
            } else {
                roles.join(",")
            }
        })
        .unwrap_or_else(|| "<none>".to_string());
    let spec = node.spec.as_ref();
    let unschedulable = spec.and_then(|s| s.unschedulable).unwrap_or(false);
    // Count taints, ignoring the taint Kubernetes adds automatically when a node
    // is cordoned — that state is already conveyed by `unschedulable`.
    let taint_details: Vec<NodeTaint> = spec
        .and_then(|s| s.taints.as_ref())
        .map(|taints| {
            taints
                .iter()
                .filter(|taint| taint.key != "node.kubernetes.io/unschedulable")
                .map(|taint| NodeTaint {
                    key: taint.key.clone(),
                    value: taint.value.clone().unwrap_or_default(),
                    effect: taint.effect.clone(),
                    time_added: taint.time_added.as_ref().map(|t| t.0.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();
    let taints = taint_details.len() as u32;
    // A node that reports no allocatable at all reports zero, not a guess —
    // the consumer downstream (packages/core/src/lib/k8sCapacity.ts) is what
    // turns a zero denominator into "no reading".
    let allocatable = node.status.as_ref().and_then(|s| s.allocatable.as_ref());
    let allocatable_cpu_millicores = allocatable
        .and_then(|a| a.get("cpu"))
        .map(|q| crate::metrics::cpu_millicores(&q.0))
        .unwrap_or(0);
    let allocatable_memory_mib = allocatable
        .and_then(|a| a.get("memory"))
        .map(|q| crate::metrics::mem_mib(&q.0))
        .unwrap_or(0);
    let allocatable_pods = allocatable
        .and_then(|a| a.get("pods"))
        .map(|q| q.0.trim().parse::<f64>().unwrap_or(0.0) as i64)
        .unwrap_or(0);
    // Modern label preferred; deprecated one is a fallback for older clusters.
    // Neither present reports empty, not "unknown" — an empty column cell is
    // the truthful answer for a node (e.g. on kind) that has no machine type.
    let instance_type = node
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| {
            labels
                .get("node.kubernetes.io/instance-type")
                .or_else(|| labels.get("beta.kubernetes.io/instance-type"))
        })
        .cloned()
        .unwrap_or_default();
    let addresses = node.status.as_ref().and_then(|s| s.addresses.as_ref());
    let internal_ip = addresses.and_then(|addrs| {
        addrs
            .iter()
            .find(|a| a.type_ == "InternalIP")
            .map(|a| a.address.clone())
    });
    let external_ip = addresses.and_then(|addrs| {
        addrs
            .iter()
            .find(|a| a.type_ == "ExternalIP")
            .map(|a| a.address.clone())
    });

    NodeSummary {
        name,
        status,
        unschedulable,
        taints,
        taint_details,
        version,
        roles,
        created: crate::creation_rfc3339(node.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(node.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(node.metadata.creation_timestamp.as_ref()),
        allocatable_cpu_millicores,
        allocatable_memory_mib,
        allocatable_pods,
        instance_type,
        internal_ip,
        external_ip,
    }
}

/// `k8s.listNodes` — list cluster nodes.
pub fn list_nodes_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListNodesIn, ListNodesOut, _, _>(
        "k8s.listNodes",
        "list the nodes of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListNodesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Node> = Api::all(client);
                let list =
                    tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                        .await
                        .map_err(|_| CapabilityError::Handler("list nodes timed out".into()))?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListNodesOut {
                    nodes: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NodeCapacityOverviewIn {
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct NodeCapacityItem {
    pub name: String,
    pub status: String,
    pub unschedulable: bool,
    pub roles: String,
    #[serde(rename = "allocatableCpuMillicores")]
    pub allocatable_cpu_millicores: i64,
    #[serde(rename = "requestedCpuMillicores")]
    pub requested_cpu_millicores: i64,
    #[serde(rename = "remainingCpuMillicores")]
    pub remaining_cpu_millicores: i64,
    #[serde(rename = "cpuUsagePercent")]
    pub cpu_usage_percent: f64,
    #[serde(rename = "allocatableMemoryMiB")]
    pub allocatable_memory_mib: i64,
    #[serde(rename = "requestedMemoryMiB")]
    pub requested_memory_mib: i64,
    #[serde(rename = "remainingMemoryMiB")]
    pub remaining_memory_mib: i64,
    #[serde(rename = "memoryUsagePercent")]
    pub memory_usage_percent: f64,
    #[serde(rename = "podCount")]
    pub pod_count: usize,
    #[serde(rename = "vmCount")]
    pub vm_count: usize,
    #[serde(rename = "allocatablePods")]
    pub allocatable_pods: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NodeCapacityOverviewOut {
    pub nodes: Vec<NodeCapacityItem>,
}

#[derive(Default)]
struct NodeUsageAcc {
    requested_cpu_millicores: i64,
    requested_memory_mib: i64,
    pod_count: usize,
    vm_count: usize,
}

/// Calculate aggregated capacity and allocated requests per node from raw Node and Pod lists.
pub fn calculate_node_capacities(nodes: Vec<Node>, pods: Vec<Pod>) -> Vec<NodeCapacityItem> {
    let mut usage_by_node: HashMap<String, NodeUsageAcc> = HashMap::new();
    for pod in pods {
        let phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("");
        if phase == "Succeeded" || phase == "Failed" {
            continue;
        }
        let node_name = match pod.spec.as_ref().and_then(|s| s.node_name.as_deref()) {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };
        let acc = usage_by_node.entry(node_name.to_string()).or_default();
        acc.pod_count += 1;
        let is_vm = pod
            .metadata
            .labels
            .as_ref()
            .map(|l| {
                l.contains_key("kubevirt.io/domain")
                    || l.get("kubevirt.io").map(|v| v.as_str()) == Some("virt-launcher")
            })
            .unwrap_or(false);
        if is_vm {
            acc.vm_count += 1;
        }
        if let Some(spec) = &pod.spec {
            for c in &spec.containers {
                if let Some(res) = &c.resources {
                    if let Some(reqs) = &res.requests {
                        if let Some(cpu) = reqs.get("cpu") {
                            acc.requested_cpu_millicores += crate::metrics::cpu_millicores(&cpu.0);
                        }
                        if let Some(mem) = reqs.get("memory") {
                            acc.requested_memory_mib += crate::metrics::mem_mib(&mem.0);
                        }
                    }
                }
            }
        }
    }

    nodes
        .into_iter()
        .map(|node| {
            let s = summarise(node);
            let usage = usage_by_node.remove(&s.name).unwrap_or_default();
            let remaining_cpu =
                (s.allocatable_cpu_millicores - usage.requested_cpu_millicores).max(0);
            let cpu_pct = if s.allocatable_cpu_millicores > 0 {
                ((usage.requested_cpu_millicores as f64 / s.allocatable_cpu_millicores as f64)
                    * 100.0)
                    .round()
            } else {
                0.0
            };
            let remaining_mem = (s.allocatable_memory_mib - usage.requested_memory_mib).max(0);
            let mem_pct = if s.allocatable_memory_mib > 0 {
                ((usage.requested_memory_mib as f64 / s.allocatable_memory_mib as f64) * 100.0)
                    .round()
            } else {
                0.0
            };
            NodeCapacityItem {
                name: s.name,
                status: s.status,
                unschedulable: s.unschedulable,
                roles: s.roles,
                allocatable_cpu_millicores: s.allocatable_cpu_millicores,
                requested_cpu_millicores: usage.requested_cpu_millicores,
                remaining_cpu_millicores: remaining_cpu,
                cpu_usage_percent: cpu_pct,
                allocatable_memory_mib: s.allocatable_memory_mib,
                requested_memory_mib: usage.requested_memory_mib,
                remaining_memory_mib: remaining_mem,
                memory_usage_percent: mem_pct,
                pod_count: usage.pod_count,
                vm_count: usage.vm_count,
                allocatable_pods: s.allocatable_pods,
            }
        })
        .collect()
}

/// `k8s.nodeCapacityOverview` — cluster-wide node capacity, requested allocations, remaining headroom, and VM counts.
pub fn node_capacity_overview_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<NodeCapacityOverviewIn, NodeCapacityOverviewOut, _, _>(
        "k8s.nodeCapacityOverview",
        "cluster-wide node capacity overview: reports allocatable, requested, and remaining CPU (millicores) and Memory (MiB), usage percentages, and pod / VM counts for every node. Fast single-call alternative to listing all pods per node for capacity planning and VM/workload migration.",
        Annotations::READ_ONLY,
        move |input: NodeCapacityOverviewIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let node_api: Api<Node> = Api::all(client.clone());
                let pod_api: Api<Pod> = Api::all(client);
                let node_lp = ListParams::default();
                let pod_lp = ListParams::default();
                let (nodes_res, pods_res) = tokio::join!(
                    tokio::time::timeout(request_timeout(), node_api.list(&node_lp)),
                    tokio::time::timeout(request_timeout(), pod_api.list(&pod_lp)),
                );
                let node_list = nodes_res
                    .map_err(|_| CapabilityError::Handler("list nodes timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let pod_list = pods_res
                    .map_err(|_| CapabilityError::Handler("list pods timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let nodes = calculate_node_capacities(node_list.items, pod_list.items);
                Ok(NodeCapacityOverviewOut { nodes })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{NodeCondition, NodeStatus, NodeSystemInfo};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_nodes_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listNodes");
    }

    #[test]
    fn node_capacity_overview_capability_has_expected_id() {
        let cap = node_capacity_overview_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.nodeCapacityOverview");
    }

    #[test]
    fn test_calculate_node_capacities_aggregates_pods_and_vms() {
        use k8s_openapi::api::core::v1::{Container, PodSpec, PodStatus, ResourceRequirements};
        use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

        let mut node = node_with_allocatable("4", "16Gi", "110");
        node.metadata.name = Some("node-1".into());

        let mut reqs = BTreeMap::new();
        reqs.insert("cpu".to_string(), Quantity("1000m".into()));
        reqs.insert("memory".to_string(), Quantity("4Gi".into()));

        let mut vm_labels = BTreeMap::new();
        vm_labels.insert("kubevirt.io".to_string(), "virt-launcher".to_string());

        let vm_pod = Pod {
            metadata: kube::core::ObjectMeta {
                name: Some("virt-launcher-test-vm".into()),
                labels: Some(vm_labels),
                ..Default::default()
            },
            spec: Some(PodSpec {
                node_name: Some("node-1".into()),
                containers: vec![Container {
                    name: "compute".into(),
                    resources: Some(ResourceRequirements {
                        requests: Some(reqs),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".into()),
                ..Default::default()
            }),
        };

        let res = calculate_node_capacities(vec![node], vec![vm_pod]);
        assert_eq!(res.len(), 1);
        let n = &res[0];
        assert_eq!(n.name, "node-1");
        assert_eq!(n.allocatable_cpu_millicores, 4000);
        assert_eq!(n.requested_cpu_millicores, 1000);
        assert_eq!(n.remaining_cpu_millicores, 3000);
        assert_eq!(n.cpu_usage_percent, 25.0);
        assert_eq!(n.allocatable_memory_mib, 16384);
        assert_eq!(n.requested_memory_mib, 4096);
        assert_eq!(n.remaining_memory_mib, 12288);
        assert_eq!(n.memory_usage_percent, 25.0);
        assert_eq!(n.pod_count, 1);
        assert_eq!(n.vm_count, 1);
    }

    #[test]
    fn summarises_ready_version_and_roles() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "node-role.kubernetes.io/control-plane".to_string(),
            "".to_string(),
        );
        let node = Node {
            metadata: kube::core::ObjectMeta {
                name: Some("cp-1".into()),
                labels: Some(labels),
                ..Default::default()
            },
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: "True".into(),
                    ..Default::default()
                }]),
                node_info: Some(NodeSystemInfo {
                    kubelet_version: "v1.35.0".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(node);
        assert_eq!(s.status, "Ready");
        assert_eq!(s.version, "v1.35.0");
        assert_eq!(s.roles, "control-plane");
        assert!(!s.unschedulable);
        assert_eq!(s.taints, 0);
    }

    fn node_with_allocatable(cpu: &str, memory: &str, pods: &str) -> Node {
        use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
        let mut allocatable = BTreeMap::new();
        allocatable.insert("cpu".to_string(), Quantity(cpu.to_string()));
        allocatable.insert("memory".to_string(), Quantity(memory.to_string()));
        allocatable.insert("pods".to_string(), Quantity(pods.to_string()));
        Node {
            metadata: kube::core::ObjectMeta {
                name: Some("n1".into()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                allocatable: Some(allocatable),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn node_with_no_status() -> Node {
        Node {
            metadata: kube::core::ObjectMeta {
                name: Some("n1".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn reads_allocatable_in_the_units_the_metrics_use() {
        let node = node_with_allocatable("3800m", "16344820Ki", "110");
        let s = summarise(node);
        assert_eq!(s.allocatable_cpu_millicores, 3800);
        assert_eq!(s.allocatable_memory_mib, 15961);
        assert_eq!(s.allocatable_pods, 110);
    }

    #[test]
    fn reads_a_whole_core_as_millicores() {
        assert_eq!(
            summarise(node_with_allocatable("4", "0", "0")).allocatable_cpu_millicores,
            4000
        );
    }

    #[test]
    fn a_node_that_reports_no_allocatable_reports_zero_not_a_guess() {
        assert_eq!(
            summarise(node_with_no_status()).allocatable_cpu_millicores,
            0
        );
        assert_eq!(summarise(node_with_no_status()).allocatable_memory_mib, 0);
        assert_eq!(summarise(node_with_no_status()).allocatable_pods, 0);
    }

    fn node_with_labels(labels: BTreeMap<String, String>) -> Node {
        Node {
            metadata: kube::core::ObjectMeta {
                name: Some("n1".into()),
                labels: Some(labels),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn reads_the_modern_instance_type_label() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "node.kubernetes.io/instance-type".to_string(),
            "c3-standard-4".to_string(),
        );
        let s = summarise(node_with_labels(labels));
        assert_eq!(s.instance_type, "c3-standard-4");
    }

    #[test]
    fn falls_back_to_the_deprecated_beta_instance_type_label() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "beta.kubernetes.io/instance-type".to_string(),
            "n2-standard-8".to_string(),
        );
        let s = summarise(node_with_labels(labels));
        assert_eq!(s.instance_type, "n2-standard-8");
    }

    #[test]
    fn prefers_the_modern_label_when_both_are_present() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "node.kubernetes.io/instance-type".to_string(),
            "t2d-spot".to_string(),
        );
        labels.insert(
            "beta.kubernetes.io/instance-type".to_string(),
            "n2-standard-8".to_string(),
        );
        let s = summarise(node_with_labels(labels));
        assert_eq!(s.instance_type, "t2d-spot");
    }

    #[test]
    fn a_node_with_neither_instance_type_label_reports_empty_not_unknown() {
        let s = summarise(node_with_labels(BTreeMap::new()));
        assert_eq!(s.instance_type, "");

        let s = summarise(node_with_no_status());
        assert_eq!(s.instance_type, "");
    }

    #[test]
    fn reports_cordoned_and_taints_excluding_the_unschedulable_taint() {
        use k8s_openapi::api::core::v1::{NodeSpec, Taint};
        let node = Node {
            metadata: kube::core::ObjectMeta {
                name: Some("worker-1".into()),
                ..Default::default()
            },
            spec: Some(NodeSpec {
                unschedulable: Some(true),
                taints: Some(vec![
                    Taint {
                        key: "dedicated".into(),
                        effect: "NoSchedule".into(),
                        ..Default::default()
                    },
                    // Auto-added when cordoned — must not be counted as a taint.
                    Taint {
                        key: "node.kubernetes.io/unschedulable".into(),
                        effect: "NoSchedule".into(),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: "True".into(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(node);
        assert_eq!(s.status, "Ready");
        assert!(s.unschedulable);
        assert_eq!(s.taints, 1);
        // The detail list is the same filtered set the count reports, so the
        // badge's number and its tooltip can never disagree.
        assert_eq!(s.taint_details.len(), 1);
        assert_eq!(s.taint_details[0].key, "dedicated");
    }

    /// A node with an empty `spec.taints` and a node with no `spec` at all are
    /// the same answer — zero, and an empty list rather than a null the
    /// frontend would have to guard.
    #[test]
    fn a_node_with_no_taints_reports_zero_and_an_empty_list() {
        let no_spec = Node {
            metadata: kube::core::ObjectMeta {
                name: Some("clean-1".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let s = summarise(no_spec);
        assert_eq!(s.taints, 0);
        assert!(s.taint_details.is_empty());

        let empty_taints = Node {
            metadata: kube::core::ObjectMeta {
                name: Some("clean-2".into()),
                ..Default::default()
            },
            spec: Some(k8s_openapi::api::core::v1::NodeSpec {
                taints: Some(vec![]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(empty_taints);
        assert_eq!(s.taints, 0);
        assert!(s.taint_details.is_empty());
    }

    /// The single-taint case the issue calls out as the benign one: a fresh
    /// control-plane node. Its taint carries no value, which is a real value
    /// (the empty string) and not a missing one.
    #[test]
    fn a_control_plane_nodes_one_taint_is_carried_whole() {
        use k8s_openapi::api::core::v1::{NodeSpec, Taint};
        let node = Node {
            metadata: kube::core::ObjectMeta {
                name: Some("cp-1".into()),
                ..Default::default()
            },
            spec: Some(NodeSpec {
                taints: Some(vec![Taint {
                    key: "node-role.kubernetes.io/control-plane".into(),
                    effect: "NoSchedule".into(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(node);
        assert_eq!(s.taints, 1);
        assert_eq!(s.taint_details.len(), 1);
        assert_eq!(s.taint_details[0].value, "");
        assert_eq!(s.taint_details[0].effect, "NoSchedule");
        assert_eq!(s.taint_details[0].time_added, None);
    }

    /// N taints across all three effects, with `timeAdded` — which Kubernetes
    /// sets only for `NoExecute` — carried through as RFC 3339 for the detail
    /// page. Order is the API server's, unchanged.
    #[test]
    fn carries_every_effect_and_the_time_a_no_execute_taint_was_added() {
        use k8s_openapi::api::core::v1::{NodeSpec, Taint};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
        let added: Time = serde_json::from_str("\"2026-09-02T08:15:00Z\"").unwrap();
        let node = Node {
            metadata: kube::core::ObjectMeta {
                name: Some("worker-9".into()),
                ..Default::default()
            },
            spec: Some(NodeSpec {
                taints: Some(vec![
                    Taint {
                        key: "node.kubernetes.io/memory-pressure".into(),
                        effect: "NoSchedule".into(),
                        ..Default::default()
                    },
                    Taint {
                        key: "spot".into(),
                        value: Some("true".into()),
                        effect: "PreferNoSchedule".into(),
                        ..Default::default()
                    },
                    Taint {
                        key: "team".into(),
                        value: Some("payments".into()),
                        effect: "NoExecute".into(),
                        time_added: Some(added),
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(node);
        assert_eq!(s.taints, 3);
        let effects: Vec<&str> = s.taint_details.iter().map(|t| t.effect.as_str()).collect();
        assert_eq!(effects, ["NoSchedule", "PreferNoSchedule", "NoExecute"]);
        assert_eq!(s.taint_details[1].value, "true");
        assert_eq!(
            s.taint_details[2].time_added.as_deref(),
            Some("2026-09-02T08:15:00Z")
        );
    }

    #[test]
    fn extracts_node_internal_and_external_ips() {
        use k8s_openapi::api::core::v1::{NodeAddress, NodeStatus};
        let node = Node {
            metadata: kube::core::ObjectMeta {
                name: Some("worker-1".into()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                addresses: Some(vec![
                    NodeAddress {
                        type_: "InternalIP".into(),
                        address: "10.0.1.20".into(),
                    },
                    NodeAddress {
                        type_: "ExternalIP".into(),
                        address: "198.51.100.5".into(),
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(node);
        assert_eq!(s.internal_ip.as_deref(), Some("10.0.1.20"));
        assert_eq!(s.external_ip.as_deref(), Some("198.51.100.5"));
    }
}
