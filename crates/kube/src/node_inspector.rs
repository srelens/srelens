use std::time::Duration;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{api::ListParams, Api, Client};
use serde::{Deserialize, Serialize};

/// Comprehensive inspection details for a single Kubernetes Node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInspectorDetails {
    pub name: String,
    pub status: String,
    pub unschedulable: bool,
    pub roles: String,
    pub instance_type: String,
    pub zone: Option<String>,
    pub region: Option<String>,
    pub nodepool: Option<String>,
    pub internal_ip: Option<String>,
    pub external_ip: Option<String>,
    pub os_image: String,
    pub kernel_version: String,
    pub container_runtime: String,
    pub kubelet_version: String,
    pub architecture: String,
    pub created_at: String,

    // Capacity vs Allocatable vs Requests
    pub cpu_capacity_millicores: i64,
    pub cpu_allocatable_millicores: i64,
    pub cpu_requests_millicores: i64,

    pub mem_capacity_mib: i64,
    pub mem_allocatable_mib: i64,
    pub mem_requests_mib: i64,

    pub pods_capacity: i64,
    pub pods_allocatable: i64,
    pub pods_count: usize,

    // GPU & Accelerator Details
    pub has_gpu: bool,
    pub gpu_model: Option<String>,
    pub gpu_driver_version: Option<String>,
    pub gpu_cuda_version: Option<String>,
    pub gpu_capacity_count: i64,
    pub gpu_allocatable_count: i64,
    pub gpu_requests_count: i64,
    pub gpu_memory_total_mib: Option<i64>,
    pub gpu_memory_requests_mib: i64,

    pub conditions: Vec<NodeConditionInfo>,
    pub taints: Vec<NodeTaintInfo>,
    pub pods: Vec<NodePodItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeConditionInfo {
    pub type_: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeTaintInfo {
    pub key: String,
    pub value: Option<String>,
    pub effect: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePodItem {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub ready_containers: String,
    pub restarts: i64,
    pub age: String,
    pub cpu_requests_millicores: i64,
    pub mem_requests_mib: i64,
    pub gpu_requests: i64,
    pub gpu_mem_requests_mib: i64,
    pub pod_ip: String,
}

/// Query and inspect a live node and all pods scheduled on it.
pub async fn inspect_node(client: Client, node_name: &str) -> Result<NodeInspectorDetails, String> {
    let node_api: Api<Node> = Api::all(client.clone());
    let node = tokio::time::timeout(Duration::from_secs(10), node_api.get(node_name))
        .await
        .map_err(|_| format!("Timed out fetching node '{}'", node_name))?
        .map_err(|e| format!("Failed to fetch node '{}': {}", node_name, e))?;

    let pod_api: Api<Pod> = Api::all(client);
    let lp = ListParams::default().fields(&format!("spec.nodeName={}", node_name));
    let pod_list = tokio::time::timeout(Duration::from_secs(10), pod_api.list(&lp))
        .await
        .map_err(|_| format!("Timed out listing pods on node '{}'", node_name))?
        .map_err(|e| format!("Failed to list pods on node '{}': {}", node_name, e))?;

    Ok(parse_node_details(&node, &pod_list.items))
}

/// Pure parser extracting `NodeInspectorDetails` from raw Kubernetes `Node` and `Pod` resources.
pub fn parse_node_details(node: &Node, pods: &[Pod]) -> NodeInspectorDetails {
    let name = node.metadata.name.clone().unwrap_or_default();

    // 1. Status & Readiness
    let status = node
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .map(|c| if c.status == "True" { "Ready" } else { "NotReady" })
        .unwrap_or("Unknown")
        .to_string();

    let unschedulable = node.spec.as_ref().and_then(|s| s.unschedulable).unwrap_or(false);

    // 2. Roles
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

    // 3. Labels & Topology metadata
    let labels = node.metadata.labels.as_ref();
    let instance_type = labels
        .and_then(|l| {
            l.get("node.kubernetes.io/instance-type")
                .or_else(|| l.get("beta.kubernetes.io/instance-type"))
        })
        .cloned()
        .unwrap_or_else(|| "-".to_string());

    let zone = labels
        .and_then(|l| {
            l.get("topology.kubernetes.io/zone")
                .or_else(|| l.get("failure-domain.beta.kubernetes.io/zone"))
        })
        .cloned();

    let region = labels
        .and_then(|l| {
            l.get("topology.kubernetes.io/region")
                .or_else(|| l.get("failure-domain.beta.kubernetes.io/region"))
        })
        .cloned();

    let nodepool = labels
        .and_then(|l| {
            l.get("cloud.google.com/gke-nodepool")
                .or_else(|| l.get("karpenter.sh/nodepool"))
                .or_else(|| l.get("eks.amazonaws.com/nodegroup"))
                .or_else(|| l.get("node.kubernetes.io/nodepool"))
        })
        .cloned();

    // 4. Addresses
    let addresses = node.status.as_ref().and_then(|s| s.addresses.as_ref());
    let internal_ip = addresses.and_then(|addrs| {
        addrs.iter().find(|a| a.type_ == "InternalIP").map(|a| a.address.clone())
    });
    let external_ip = addresses.and_then(|addrs| {
        addrs.iter().find(|a| a.type_ == "ExternalIP").map(|a| a.address.clone())
    });

    // 5. System Info
    let node_info = node.status.as_ref().and_then(|s| s.node_info.as_ref());
    let os_image = node_info.map(|i| i.os_image.clone()).unwrap_or_else(|| "-".to_string());
    let kernel_version = node_info.map(|i| i.kernel_version.clone()).unwrap_or_else(|| "-".to_string());
    let container_runtime = node_info.map(|i| i.container_runtime_version.clone()).unwrap_or_else(|| "-".to_string());
    let kubelet_version = node_info.map(|i| i.kubelet_version.clone()).unwrap_or_else(|| "-".to_string());
    let architecture = node_info.map(|i| i.architecture.clone()).unwrap_or_else(|| "-".to_string());
    let created_at = crate::creation_timestamp_iso(node.metadata.creation_timestamp.as_ref());

    // 6. CPU, Memory, Pods Capacity & Allocatable
    let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
    let allocatable = node.status.as_ref().and_then(|s| s.allocatable.as_ref());

    let cpu_capacity_millicores = capacity
        .and_then(|c| c.get("cpu"))
        .map(|q| crate::metrics::cpu_millicores(&q.0))
        .unwrap_or(0);
    let cpu_allocatable_millicores = allocatable
        .and_then(|a| a.get("cpu"))
        .map(|q| crate::metrics::cpu_millicores(&q.0))
        .unwrap_or(0);

    let mem_capacity_mib = capacity
        .and_then(|c| c.get("memory"))
        .map(|q| crate::metrics::mem_mib(&q.0))
        .unwrap_or(0);
    let mem_allocatable_mib = allocatable
        .and_then(|a| a.get("memory"))
        .map(|q| crate::metrics::mem_mib(&q.0))
        .unwrap_or(0);

    let pods_capacity = capacity
        .and_then(|c| c.get("pods"))
        .and_then(|q| q.0.trim().parse::<f64>().ok())
        .map(|n| n as i64)
        .unwrap_or(0);
    let pods_allocatable = allocatable
        .and_then(|a| a.get("pods"))
        .and_then(|q| q.0.trim().parse::<f64>().ok())
        .map(|n| n as i64)
        .unwrap_or(0);

    // 7. GPU & Accelerator Detection
    let mut gpu_capacity_count: i64 = 0;
    let mut gpu_allocatable_count: i64 = 0;

    // Discrete GPU keys
    let discrete_gpu_keys = [
        "nvidia.com/gpu",
        "amd.com/gpu",
        "google.com/tpu",
    ];

    for key in &discrete_gpu_keys {
        if let Some(cap) = capacity.and_then(|c| c.get(*key)) {
            if let Ok(count) = cap.0.trim().parse::<i64>() {
                gpu_capacity_count = gpu_capacity_count.max(count);
            }
        }
        if let Some(alloc) = allocatable.and_then(|a| a.get(*key)) {
            if let Ok(count) = alloc.0.trim().parse::<i64>() {
                gpu_allocatable_count = gpu_allocatable_count.max(count);
            }
        }
    }

    // Check MIG slices if no standard GPU was found
    if gpu_capacity_count == 0 {
        if let Some(cap) = capacity {
            for (k, v) in cap {
                if k.starts_with("nvidia.com/mig-") {
                    if let Ok(count) = v.0.trim().parse::<i64>() {
                        gpu_capacity_count += count;
                    }
                }
            }
        }
        if let Some(alloc) = allocatable {
            for (k, v) in alloc {
                if k.starts_with("nvidia.com/mig-") {
                    if let Ok(count) = v.0.trim().parse::<i64>() {
                        gpu_allocatable_count += count;
                    }
                }
            }
        }
    }

    // GPU Model & Driver Info
    let gpu_model = labels.and_then(|l| {
        l.get("nvidia.com/gpu.product")
            .or_else(|| l.get("gpu.trivago.com/model"))
            .or_else(|| l.get("nvidia.com/gpu.machine"))
            .or_else(|| l.get("nvidia.com/gpu.family"))
            .cloned()
            .map(|m| m.replace('-', " "))
    });

    let gpu_driver_version = labels.and_then(|l| {
        l.get("nvidia.com/cuda.driver-version")
            .or_else(|| l.get("nvidia.com/driver-version"))
            .cloned()
    });

    let gpu_cuda_version = labels.and_then(|l| {
        l.get("nvidia.com/cuda.runtime.version")
            .or_else(|| l.get("nvidia.com/cuda.version"))
            .cloned()
    });

    let mut gpu_memory_total_mib = labels.and_then(|l| {
        l.get("nvidia.com/gpu.memory")
            .and_then(|m| m.parse::<i64>().ok())
    });

    // Check capacity / allocatable for "nvidia.com/gpumem" (HAMi virtual memory)
    if gpu_memory_total_mib.is_none() {
        if let Some(mem_cap) = capacity.and_then(|c| c.get("nvidia.com/gpumem")) {
            if let Ok(val) = mem_cap.0.trim().parse::<i64>() {
                gpu_memory_total_mib = Some(val);
            }
        }
    }

    // If still none, fallback based on well-known GPU models
    if gpu_memory_total_mib.is_none() {
        if let Some(model) = &gpu_model {
            let m_lower = model.to_lowercase();
            if m_lower.contains("t4") {
                gpu_memory_total_mib = Some(15360); // 15 GiB
            } else if m_lower.contains("a100") {
                gpu_memory_total_mib = Some(81920); // 80 GiB
            } else if m_lower.contains("h100") {
                gpu_memory_total_mib = Some(81920); // 80 GiB
            } else if m_lower.contains("a10") || m_lower.contains("l40") {
                gpu_memory_total_mib = Some(24576); // 24 GiB
            } else if m_lower.contains("v100") {
                gpu_memory_total_mib = Some(16384); // 16 GiB
            }
        }
    }

    let has_gpu = gpu_capacity_count > 0
        || gpu_model.is_some()
        || labels.map(|l| l.contains_key("nvidia.com/gpu.present") || l.contains_key("trivago.com/gpu") || l.contains_key("gpu.trivago.com/model")).unwrap_or(false);

    // 8. Conditions
    let conditions = node
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conds| {
            conds
                .iter()
                .map(|c| NodeConditionInfo {
                    type_: c.type_.clone(),
                    status: c.status.clone(),
                    reason: c.reason.clone(),
                    message: c.message.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    // 9. Taints
    let taints = node
        .spec
        .as_ref()
        .and_then(|s| s.taints.as_ref())
        .map(|ts| {
            ts.iter()
                .map(|t| NodeTaintInfo {
                    key: t.key.clone(),
                    value: t.value.clone(),
                    effect: t.effect.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    // 10. Process Pods running on this node
    let mut pod_items = Vec::new();
    let mut cpu_requests_millicores = 0;
    let mut mem_requests_mib = 0;
    let mut gpu_requests_count = 0;
    let mut gpu_memory_requests_mib = 0;

    for pod in pods {
        let p_name = pod.metadata.name.clone().unwrap_or_default();
        let p_ns = pod.metadata.namespace.clone().unwrap_or_else(|| "default".to_string());
        let p_phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let age = crate::humanize_age(pod.metadata.creation_timestamp.as_ref());

        // Container ready count
        let container_statuses = pod.status.as_ref().and_then(|s| s.container_statuses.as_ref());
        let ready_count = container_statuses
            .map(|cs| cs.iter().filter(|c| c.ready).count())
            .unwrap_or(0);
        let total_containers = pod
            .spec
            .as_ref()
            .map(|s| s.containers.len())
            .unwrap_or(0);
        let ready_containers = format!("{}/{}", ready_count, total_containers);

        let restarts: i64 = container_statuses
            .map(|cs| cs.iter().map(|c| c.restart_count as i64).sum())
            .unwrap_or(0);

        // Requests computation
        let mut pod_cpu_req = 0;
        let mut pod_mem_req = 0;
        let mut pod_gpu_req = 0;
        let mut pod_gpu_mem_req = 0;

        if let Some(spec) = &pod.spec {
            for c in &spec.containers {
                if let Some(res) = &c.resources {
                    if let Some(reqs) = &res.requests {
                        if let Some(cpu) = reqs.get("cpu") {
                            pod_cpu_req += crate::metrics::cpu_millicores(&cpu.0);
                        }
                        if let Some(mem) = reqs.get("memory") {
                            pod_mem_req += crate::metrics::mem_mib(&mem.0);
                        }
                        for key in &discrete_gpu_keys {
                            if let Some(gpu) = reqs.get(*key) {
                                if let Ok(val) = gpu.0.trim().parse::<i64>() {
                                    pod_gpu_req += val;
                                }
                            }
                        }
                        if let Some(gpumem) = reqs.get("nvidia.com/gpumem") {
                            if let Ok(val) = gpumem.0.trim().parse::<i64>() {
                                pod_gpu_mem_req += val;
                            }
                        }
                        for (k, v) in reqs {
                            if k.starts_with("nvidia.com/mig-") {
                                if let Ok(val) = v.0.trim().parse::<i64>() {
                                    pod_gpu_req += val;
                                }
                            }
                        }
                    }
                    // Fallback to limits if requests not specified
                    if pod_gpu_req == 0 && pod_gpu_mem_req == 0 {
                        if let Some(limits) = &res.limits {
                            for key in &discrete_gpu_keys {
                                if let Some(gpu) = limits.get(*key) {
                                    if let Ok(val) = gpu.0.trim().parse::<i64>() {
                                        pod_gpu_req += val;
                                    }
                                }
                            }
                            if let Some(gpumem) = limits.get("nvidia.com/gpumem") {
                                if let Ok(val) = gpumem.0.trim().parse::<i64>() {
                                    pod_gpu_mem_req += val;
                                }
                            }
                            for (k, v) in limits {
                                if k.starts_with("nvidia.com/mig-") {
                                    if let Ok(val) = v.0.trim().parse::<i64>() {
                                        pod_gpu_req += val;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // If pod requested GPU VRAM (HAMi) but not discrete GPU count, count it as 1 slice
        if pod_gpu_mem_req > 0 && pod_gpu_req == 0 {
            pod_gpu_req = 1;
        }

        let p_ip = pod
            .status
            .as_ref()
            .and_then(|s| s.pod_ip.clone())
            .unwrap_or_default();

        cpu_requests_millicores += pod_cpu_req;
        mem_requests_mib += pod_mem_req;
        gpu_requests_count += pod_gpu_req;
        gpu_memory_requests_mib += pod_gpu_mem_req;

        pod_items.push(NodePodItem {
            name: p_name,
            namespace: p_ns,
            phase: p_phase,
            ready_containers,
            restarts,
            age,
            cpu_requests_millicores: pod_cpu_req,
            mem_requests_mib: pod_mem_req,
            gpu_requests: pod_gpu_req,
            gpu_mem_requests_mib: pod_gpu_mem_req,
            pod_ip: p_ip,
        });
    }

    // Sort pods: GPU/VRAM consumers first, then by CPU requests descending
    pod_items.sort_by(|a, b| {
        let a_has_gpu = a.gpu_requests > 0 || a.gpu_mem_requests_mib > 0;
        let b_has_gpu = b.gpu_requests > 0 || b.gpu_mem_requests_mib > 0;
        b_has_gpu
            .cmp(&a_has_gpu)
            .then_with(|| b.gpu_mem_requests_mib.cmp(&a.gpu_mem_requests_mib))
            .then_with(|| b.gpu_requests.cmp(&a.gpu_requests))
            .then_with(|| b.cpu_requests_millicores.cmp(&a.cpu_requests_millicores))
            .then_with(|| a.name.cmp(&b.name))
    });

    let pods_count = pod_items.len();

    NodeInspectorDetails {
        name,
        status,
        unschedulable,
        roles,
        instance_type,
        zone,
        region,
        nodepool,
        internal_ip,
        external_ip,
        os_image,
        kernel_version,
        container_runtime,
        kubelet_version,
        architecture,
        created_at,
        cpu_capacity_millicores,
        cpu_allocatable_millicores,
        cpu_requests_millicores,
        mem_capacity_mib,
        mem_allocatable_mib,
        mem_requests_mib,
        pods_capacity,
        pods_allocatable,
        pods_count,
        has_gpu,
        gpu_model,
        gpu_driver_version,
        gpu_cuda_version,
        gpu_capacity_count,
        gpu_allocatable_count,
        gpu_requests_count,
        gpu_memory_total_mib,
        gpu_memory_requests_mib,
        conditions,
        taints,
        pods: pod_items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;

    #[test]
    fn test_parse_node_details_with_gpu_and_pods() {
        let mut labels = BTreeMap::new();
        labels.insert("node.kubernetes.io/instance-type".to_string(), "g4dn.xlarge".to_string());
        labels.insert("topology.kubernetes.io/zone".to_string(), "eu-west-1a".to_string());
        labels.insert("nvidia.com/gpu.product".to_string(), "Tesla-T4".to_string());
        labels.insert("nvidia.com/cuda.driver-version".to_string(), "535.129".to_string());
        labels.insert("nvidia.com/cuda.runtime.version".to_string(), "12.2".to_string());

        let mut allocatable = BTreeMap::new();
        allocatable.insert("cpu".to_string(), Quantity("4".to_string()));
        allocatable.insert("memory".to_string(), Quantity("16Gi".to_string()));
        allocatable.insert("pods".to_string(), Quantity("110".to_string()));
        allocatable.insert("nvidia.com/gpu".to_string(), Quantity("1".to_string()));

        let node = Node {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("gpu-worker-1".to_string()),
                labels: Some(labels),
                ..Default::default()
            },
            status: Some(k8s_openapi::api::core::v1::NodeStatus {
                allocatable: Some(allocatable.clone()),
                capacity: Some(allocatable),
                conditions: Some(vec![k8s_openapi::api::core::v1::NodeCondition {
                    type_: "Ready".to_string(),
                    status: "True".to_string(),
                    reason: Some("KubeletReady".to_string()),
                    message: Some("kubelet is posting ready status".to_string()),
                    ..Default::default()
                }]),
                node_info: Some(k8s_openapi::api::core::v1::NodeSystemInfo {
                    kubelet_version: "v1.30.2".to_string(),
                    os_image: "Ubuntu 22.04 LTS".to_string(),
                    kernel_version: "5.15.0-generic".to_string(),
                    container_runtime_version: "containerd://1.7.1".to_string(),
                    architecture: "amd64".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            spec: Some(k8s_openapi::api::core::v1::NodeSpec {
                unschedulable: Some(false),
                taints: Some(vec![k8s_openapi::api::core::v1::Taint {
                    key: "nvidia.com/gpu".to_string(),
                    value: Some("present".to_string()),
                    effect: "NoSchedule".to_string(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        };

        let mut pod_requests = BTreeMap::new();
        pod_requests.insert("cpu".to_string(), Quantity("500m".to_string()));
        pod_requests.insert("memory".to_string(), Quantity("2Gi".to_string()));
        pod_requests.insert("nvidia.com/gpu".to_string(), Quantity("1".to_string()));

        let pod1 = Pod {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some("llm-inference-1".to_string()),
                namespace: Some("ai-workloads".to_string()),
                ..Default::default()
            },
            spec: Some(k8s_openapi::api::core::v1::PodSpec {
                containers: vec![k8s_openapi::api::core::v1::Container {
                    name: "vllm".to_string(),
                    resources: Some(k8s_openapi::api::core::v1::ResourceRequirements {
                        requests: Some(pod_requests),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(k8s_openapi::api::core::v1::PodStatus {
                phase: Some("Running".to_string()),
                container_statuses: Some(vec![k8s_openapi::api::core::v1::ContainerStatus {
                    name: "vllm".to_string(),
                    ready: true,
                    restart_count: 0,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        };

        let details = parse_node_details(&node, &[pod1]);

        assert_eq!(details.name, "gpu-worker-1");
        assert_eq!(details.status, "Ready");
        assert!(!details.unschedulable);
        assert_eq!(details.instance_type, "g4dn.xlarge");
        assert_eq!(details.zone.as_deref(), Some("eu-west-1a"));
        assert!(details.has_gpu);
        assert_eq!(details.gpu_model.as_deref(), Some("Tesla T4"));
        assert_eq!(details.gpu_driver_version.as_deref(), Some("535.129"));
        assert_eq!(details.gpu_cuda_version.as_deref(), Some("12.2"));
        assert_eq!(details.gpu_capacity_count, 1);
        assert_eq!(details.gpu_requests_count, 1);
        assert_eq!(details.cpu_allocatable_millicores, 4000);
        assert_eq!(details.cpu_requests_millicores, 500);
        assert_eq!(details.mem_allocatable_mib, 16384);
        assert_eq!(details.mem_requests_mib, 2048);
        assert_eq!(details.pods_count, 1);
        assert_eq!(details.pods[0].name, "llm-inference-1");
        assert_eq!(details.pods[0].gpu_requests, 1);
        assert_eq!(details.taints.len(), 1);
        assert_eq!(details.taints[0].key, "nvidia.com/gpu");
    }
}
