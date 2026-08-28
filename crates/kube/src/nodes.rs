//! The `k8s.listNodes` capability (cluster-scoped).

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::Node;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNodesIn {
    pub context: String,
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
    pub version: String,
    pub roles: String,
    pub age: String,
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
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNodesOut {
    pub nodes: Vec<NodeSummary>,
}

fn summarise(node: Node) -> NodeSummary {
    let name = node.metadata.name.clone().unwrap_or_default();
    let status = node
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .map(|c| if c.status == "True" { "Ready" } else { "NotReady" })
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
    let taints = spec
        .and_then(|s| s.taints.as_ref())
        .map(|taints| {
            taints
                .iter()
                .filter(|taint| taint.key != "node.kubernetes.io/unschedulable")
                .count() as u32
        })
        .unwrap_or(0);
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
    NodeSummary {
        name,
        status,
        unschedulable,
        taints,
        version,
        roles,
        age: crate::humanize_age(node.metadata.creation_timestamp.as_ref()),
        allocatable_cpu_millicores,
        allocatable_memory_mib,
        allocatable_pods,
        instance_type,
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
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
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
    fn summarises_ready_version_and_roles() {
        let mut labels = BTreeMap::new();
        labels.insert("node-role.kubernetes.io/control-plane".to_string(), "".to_string());
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
        assert_eq!(summarise(node_with_allocatable("4", "0", "0")).allocatable_cpu_millicores, 4000);
    }

    #[test]
    fn a_node_that_reports_no_allocatable_reports_zero_not_a_guess() {
        assert_eq!(summarise(node_with_no_status()).allocatable_cpu_millicores, 0);
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
    }
}
