use std::collections::HashMap;
use std::time::Duration;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{Api, Client};
use kube::api::ListParams;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuPodItem {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub gpu_requests: i64,
    pub vram_requests_mib: i64,
    pub ready_containers: String,
    pub restarts: i64,
    pub age: String,
    pub containers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuNodeInfo {
    pub name: String,
    pub status: String,
    pub unschedulable: bool,
    pub roles: String,
    pub instance_type: String,
    pub gpu_model: Option<String>,
    pub gpu_driver_version: Option<String>,
    pub gpu_cuda_version: Option<String>,
    pub gpu_capacity: i64,
    pub gpu_allocatable: i64,
    pub gpu_requests: i64,
    pub vram_per_gpu_mib: Option<i64>,
    pub vram_capacity_total_mib: Option<i64>,
    pub vram_requests_total_mib: i64,
    pub pods: Vec<GpuPodItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuClusterInfo {
    pub nodes: Vec<GpuNodeInfo>,
    pub total_gpu_nodes: usize,
    pub total_gpus: i64,
    pub total_allocated_gpus: i64,
    pub total_vram_mib: i64,
    pub total_allocated_vram_mib: i64,
    pub total_gpu_pods: usize,
}

/// Fetch cluster nodes and pods to compile comprehensive GPU and VRAM allocation info.
pub async fn fetch_gpu_info(client: Client) -> Result<GpuClusterInfo, String> {
    let node_api: Api<Node> = Api::all(client.clone());
    let node_list = tokio::time::timeout(Duration::from_secs(12), node_api.list(&ListParams::default()))
        .await
        .map_err(|_| "Timed out fetching cluster nodes".to_string())?
        .map_err(|e| format!("Failed to list cluster nodes: {}", e))?;

    let pod_api: Api<Pod> = Api::all(client);
    let pod_list = tokio::time::timeout(Duration::from_secs(15), pod_api.list(&ListParams::default()))
        .await
        .map_err(|_| "Timed out fetching cluster pods".to_string())?
        .map_err(|e| format!("Failed to list cluster pods: {}", e))?;

    Ok(parse_gpu_cluster_info(&node_list.items, &pod_list.items))
}

/// Pure parser extracting `GpuClusterInfo` from raw Kubernetes `Node` and `Pod` resources.
pub fn parse_gpu_cluster_info(nodes: &[Node], pods: &[Pod]) -> GpuClusterInfo {
    // 1. Group pods by nodeName
    let mut pods_by_node: HashMap<String, Vec<&Pod>> = HashMap::new();
    for pod in pods {
        if let Some(spec) = &pod.spec {
            if let Some(node_name) = &spec.node_name {
                pods_by_node.entry(node_name.clone()).or_default().push(pod);
            }
        }
    }

    let discrete_gpu_keys = [
        "nvidia.com/gpu",
        "amd.com/gpu",
        "google.com/tpu",
    ];

    let mut gpu_nodes = Vec::new();
    let mut cluster_total_gpus = 0;
    let mut cluster_allocated_gpus = 0;
    let mut cluster_total_vram_mib = 0;
    let mut cluster_allocated_vram_mib = 0;
    let mut cluster_total_gpu_pods = 0;

    for node in nodes {
        let name = node.metadata.name.clone().unwrap_or_default();
        let labels = node.metadata.labels.as_ref();
        let capacity = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let allocatable = node.status.as_ref().and_then(|s| s.allocatable.as_ref());

        // Detect GPU capacity & allocatable
        let mut gpu_capacity: i64 = 0;
        let mut gpu_allocatable: i64 = 0;

        for key in &discrete_gpu_keys {
            if let Some(cap) = capacity.and_then(|c| c.get(*key)) {
                if let Ok(count) = cap.0.trim().parse::<i64>() {
                    gpu_capacity = gpu_capacity.max(count);
                }
            }
            if let Some(alloc) = allocatable.and_then(|a| a.get(*key)) {
                if let Ok(count) = alloc.0.trim().parse::<i64>() {
                    gpu_allocatable = gpu_allocatable.max(count);
                }
            }
        }

        // Check MIG slices if no standard discrete GPU
        if gpu_capacity == 0 {
            if let Some(cap) = capacity {
                for (k, v) in cap {
                    if k.starts_with("nvidia.com/mig-") {
                        if let Ok(count) = v.0.trim().parse::<i64>() {
                            gpu_capacity += count;
                        }
                    }
                }
            }
            if let Some(alloc) = allocatable {
                for (k, v) in alloc {
                    if k.starts_with("nvidia.com/mig-") {
                        if let Ok(count) = v.0.trim().parse::<i64>() {
                            gpu_allocatable += count;
                        }
                    }
                }
            }
        }

        // Check HAMi virtual GPU count if present
        if gpu_capacity == 0 {
            if let Some(cap) = capacity.and_then(|c| c.get("nvidia.com/vgpu")) {
                if let Ok(count) = cap.0.trim().parse::<i64>() {
                    gpu_capacity = gpu_capacity.max(count);
                }
            }
            if let Some(alloc) = allocatable.and_then(|a| a.get("nvidia.com/vgpu")) {
                if let Ok(count) = alloc.0.trim().parse::<i64>() {
                    gpu_allocatable = gpu_allocatable.max(count);
                }
            }
        }

        // Model & hardware labels
        let gpu_model = labels.and_then(|l| {
            l.get("nvidia.com/gpu.product")
                .or_else(|| l.get("gpu.trivago.com/model"))
                .or_else(|| l.get("nvidia.com/gpu.machine"))
                .or_else(|| l.get("nvidia.com/gpu.family"))
                .cloned()
                .map(|m| m.replace('-', " "))
        });

        let has_gpu = gpu_capacity > 0
            || gpu_model.is_some()
            || labels.map(|l| {
                l.contains_key("nvidia.com/gpu.present")
                    || l.contains_key("trivago.com/gpu")
                    || l.contains_key("gpu.trivago.com/model")
                    || l.contains_key("feature.node.kubernetes.io/pci-10de.present")
            }).unwrap_or(false);

        if !has_gpu {
            continue;
        }

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

        // Determine VRAM per GPU
        let mut vram_per_gpu_mib: Option<i64> = labels.and_then(|l| {
            l.get("nvidia.com/gpu.memory")
                .and_then(|m| m.parse::<i64>().ok())
        });

        if vram_per_gpu_mib.is_none() {
            if let Some(mem_cap) = capacity.and_then(|c| c.get("nvidia.com/gpumem")) {
                if let Ok(val) = mem_cap.0.trim().parse::<i64>() {
                    let total_gpus = gpu_capacity.max(1);
                    vram_per_gpu_mib = Some(val / total_gpus);
                }
            }
        }

        if vram_per_gpu_mib.is_none() {
            if let Some(model) = &gpu_model {
                let m = model.to_lowercase();
                if m.contains("t4") {
                    vram_per_gpu_mib = Some(15360); // 15 GiB
                } else if m.contains("a100") {
                    if m.contains("40gb") || m.contains("40g") {
                        vram_per_gpu_mib = Some(40960); // 40 GiB
                    } else {
                        vram_per_gpu_mib = Some(81920); // 80 GiB
                    }
                } else if m.contains("h100") {
                    vram_per_gpu_mib = Some(81920); // 80 GiB
                } else if m.contains("h200") {
                    vram_per_gpu_mib = Some(144384); // 141 GiB
                } else if m.contains("b200") {
                    vram_per_gpu_mib = Some(196608); // 192 GiB
                } else if m.contains("l40") {
                    vram_per_gpu_mib = Some(49152); // 48 GiB
                } else if m.contains("l4") {
                    vram_per_gpu_mib = Some(24576); // 24 GiB
                } else if m.contains("a10") || m.contains("a30") {
                    vram_per_gpu_mib = Some(24576); // 24 GiB
                } else if m.contains("v100") {
                    if m.contains("32gb") || m.contains("32g") {
                        vram_per_gpu_mib = Some(32768);
                    } else {
                        vram_per_gpu_mib = Some(16384); // 16 GiB
                    }
                } else if m.contains("rtx 4090") || m.contains("rtx 3090") {
                    vram_per_gpu_mib = Some(24576); // 24 GiB
                } else if m.contains("a40") || m.contains("a6000") {
                    vram_per_gpu_mib = Some(49152); // 48 GiB
                }
            }
        }

        // Status & roles
        let status = node
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
            .map(|c| if c.status == "True" { "Ready" } else { "NotReady" })
            .unwrap_or("Unknown")
            .to_string();

        let unschedulable = node.spec.as_ref().and_then(|s| s.unschedulable).unwrap_or(false);

        let roles = labels
            .map(|lbls| {
                let r: Vec<String> = lbls
                    .keys()
                    .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if r.is_empty() {
                    "<none>".to_string()
                } else {
                    r.join(",")
                }
            })
            .unwrap_or_else(|| "<none>".to_string());

        let instance_type = labels
            .and_then(|l| {
                l.get("node.kubernetes.io/instance-type")
                    .or_else(|| l.get("beta.kubernetes.io/instance-type"))
            })
            .cloned()
            .unwrap_or_else(|| "-".to_string());

        // Process Pods on this node requesting GPU / VRAM
        let mut gpu_pod_items = Vec::new();
        let mut node_gpu_reqs: i64 = 0;
        let mut node_vram_reqs_mib: i64 = 0;

        if let Some(node_pods) = pods_by_node.get(&name) {
            for pod in node_pods {
                let p_name = pod.metadata.name.clone().unwrap_or_default();
                let p_ns = pod.metadata.namespace.clone().unwrap_or_else(|| "default".to_string());
                let p_phase = pod
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.clone())
                    .unwrap_or_else(|| "Unknown".to_string());

                let age = crate::humanize_age(pod.metadata.creation_timestamp.as_ref());
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

                let mut pod_gpus: i64 = 0;
                let mut pod_vram_mib: i64 = 0;
                let mut requesting_containers = Vec::new();

                if let Some(spec) = &pod.spec {
                    for c in &spec.containers {
                        let mut c_gpu = 0;
                        let mut c_vram = 0;

                        if let Some(res) = &c.resources {
                            // Check requests first, then fallback to limits
                            let reqs_or_limits = res.requests.as_ref().or(res.limits.as_ref());
                            if let Some(res_map) = reqs_or_limits {
                                for key in &discrete_gpu_keys {
                                    if let Some(q) = res_map.get(*key) {
                                        if let Ok(val) = q.0.trim().parse::<i64>() {
                                            c_gpu += val;
                                        }
                                    }
                                }
                                // HAMi / virtual GPU memory in MiB
                                if let Some(q) = res_map.get("nvidia.com/gpumem")
                                    .or_else(|| res_map.get("nvidia.com/gpu-mem"))
                                    .or_else(|| res_map.get("nvidia.com/gpu.memory"))
                                    .or_else(|| res_map.get("nvidia.com/vcuda-memory"))
                                {
                                    c_vram += parse_vram_quantity(&q.0);
                                }
                                // MIG slices
                                for (k, q) in res_map {
                                    if k.starts_with("nvidia.com/mig-") {
                                        if let Ok(slices) = q.0.trim().parse::<i64>() {
                                            c_gpu += slices;
                                            c_vram += parse_mig_slice_vram_mib(k) * slices;
                                        }
                                    }
                                }
                            }
                        }

                        if c_gpu > 0 || c_vram > 0 {
                            // If VRAM wasn't explicitly specified via HAMi or MIG, but discrete GPUs were requested,
                            // allocate full VRAM of those GPUs based on node's per-GPU capacity
                            if c_vram == 0 && c_gpu > 0 {
                                if let Some(vram_per_gpu) = vram_per_gpu_mib {
                                    c_vram = c_gpu * vram_per_gpu;
                                }
                            }
                            pod_gpus += c_gpu;
                            pod_vram_mib += c_vram;
                            requesting_containers.push(c.name.clone());
                        }
                    }
                }

                // If pod requested GPU resources, record it
                if pod_gpus > 0 || pod_vram_mib > 0 {
                    node_gpu_reqs += pod_gpus;
                    node_vram_reqs_mib += pod_vram_mib;

                    gpu_pod_items.push(GpuPodItem {
                        name: p_name,
                        namespace: p_ns,
                        phase: p_phase,
                        gpu_requests: pod_gpus,
                        vram_requests_mib: pod_vram_mib,
                        ready_containers,
                        restarts,
                        age,
                        containers: requesting_containers,
                    });
                }
            }
        }

        // Sort pods by requested VRAM / GPU count descending
        gpu_pod_items.sort_by(|a, b| {
            b.vram_requests_mib
                .cmp(&a.vram_requests_mib)
                .then_with(|| b.gpu_requests.cmp(&a.gpu_requests))
                .then_with(|| a.name.cmp(&b.name))
        });

        let total_node_vram_mib = vram_per_gpu_mib.map(|per_gpu| per_gpu * gpu_capacity.max(1));

        cluster_total_gpus += gpu_capacity;
        cluster_allocated_gpus += node_gpu_reqs;
        if let Some(node_vram) = total_node_vram_mib {
            cluster_total_vram_mib += node_vram;
        }
        cluster_allocated_vram_mib += node_vram_reqs_mib;
        cluster_total_gpu_pods += gpu_pod_items.len();

        gpu_nodes.push(GpuNodeInfo {
            name,
            status,
            unschedulable,
            roles,
            instance_type,
            gpu_model,
            gpu_driver_version,
            gpu_cuda_version,
            gpu_capacity,
            gpu_allocatable,
            gpu_requests: node_gpu_reqs,
            vram_per_gpu_mib,
            vram_capacity_total_mib: total_node_vram_mib,
            vram_requests_total_mib: node_vram_reqs_mib,
            pods: gpu_pod_items,
        });
    }

    // Sort nodes alphabetically by name
    gpu_nodes.sort_by(|a, b| a.name.cmp(&b.name));

    let total_nodes = gpu_nodes.len();
    GpuClusterInfo {
        nodes: gpu_nodes,
        total_gpu_nodes: total_nodes,
        total_gpus: cluster_total_gpus,
        total_allocated_gpus: cluster_allocated_gpus,
        total_vram_mib: cluster_total_vram_mib,
        total_allocated_vram_mib: cluster_allocated_vram_mib,
        total_gpu_pods: cluster_total_gpu_pods,
    }
}

/// Helper to parse quantity string (e.g. "16384", "16384Mi", "16Gi") into MiB.
fn parse_vram_quantity(raw: &str) -> i64 {
    let s = raw.trim();
    if let Ok(n) = s.parse::<i64>() {
        return n; // Raw number in MiB
    }
    if let Some(stripped) = s.strip_suffix("Gi") {
        if let Ok(n) = stripped.trim().parse::<f64>() {
            return (n * 1024.0) as i64;
        }
    }
    if let Some(stripped) = s.strip_suffix("Mi") {
        if let Ok(n) = stripped.trim().parse::<f64>() {
            return n as i64;
        }
    }
    if let Some(stripped) = s.strip_suffix("G") {
        if let Ok(n) = stripped.trim().parse::<f64>() {
            return (n * 1000.0 * 1000.0 / 1024.0 / 1024.0) as i64;
        }
    }
    crate::metrics::mem_mib(s)
}

/// Helper to parse standard NVIDIA MIG profile slice memory into MiB.
fn parse_mig_slice_vram_mib(profile: &str) -> i64 {
    // e.g. "nvidia.com/mig-1g.5gb" -> 5 GiB -> 5120 MiB
    // "nvidia.com/mig-2g.10gb" -> 10 GiB -> 10240 MiB
    // "nvidia.com/mig-3g.20gb" -> 20 GiB -> 20480 MiB
    // "nvidia.com/mig-7g.80gb" -> 80 GiB -> 81920 MiB
    if profile.contains(".5gb") {
        5120
    } else if profile.contains(".10gb") {
        10240
    } else if profile.contains(".20gb") {
        20480
    } else if profile.contains(".40gb") {
        40960
    } else if profile.contains(".80gb") {
        81920
    } else {
        10240 // reasonable fallback
    }
}

/// Format MiB into a human-readable VRAM string (e.g. "80.0 GiB" or "512 MiB").
pub fn format_vram_mib(mib: i64) -> String {
    if mib >= 1024 {
        let gib = mib as f64 / 1024.0;
        if (gib - gib.round()).abs() < 0.05 {
            format!("{} GiB", gib.round() as i64)
        } else {
            format!("{:.1} GiB", gib)
        }
    } else {
        format!("{} MiB", mib)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use k8s_openapi::api::core::v1::{Container, NodeSpec, NodeStatus, PodSpec, PodStatus, ResourceRequirements};
    use std::collections::BTreeMap;

    #[test]
    fn test_parse_gpu_cluster_info_and_pod_vram() {
        let mut node_labels = BTreeMap::new();
        node_labels.insert("nvidia.com/gpu.product".to_string(), "NVIDIA-A100-SXM4-80GB".to_string());
        node_labels.insert("nvidia.com/cuda.driver-version".to_string(), "535.129.03".to_string());
        node_labels.insert("nvidia.com/cuda.runtime.version".to_string(), "12.2".to_string());
        node_labels.insert("node.kubernetes.io/instance-type".to_string(), "p4de.24xlarge".to_string());

        let mut node_capacity = BTreeMap::new();
        node_capacity.insert("nvidia.com/gpu".to_string(), Quantity("8".to_string()));

        let node = Node {
            metadata: ObjectMeta {
                name: Some("gpu-node-01".to_string()),
                labels: Some(node_labels),
                ..Default::default()
            },
            spec: Some(NodeSpec {
                unschedulable: Some(false),
                ..Default::default()
            }),
            status: Some(NodeStatus {
                capacity: Some(node_capacity.clone()),
                allocatable: Some(node_capacity),
                ..Default::default()
            }),
        };

        // Pod 1: requests 2 discrete GPUs -> 2 * 80 GiB = 160 GiB
        let mut pod1_requests = BTreeMap::new();
        pod1_requests.insert("nvidia.com/gpu".to_string(), Quantity("2".to_string()));

        let pod1 = Pod {
            metadata: ObjectMeta {
                name: Some("llm-trainer-0".to_string()),
                namespace: Some("ai-training".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                node_name: Some("gpu-node-01".to_string()),
                containers: vec![Container {
                    name: "pytorch".to_string(),
                    resources: Some(ResourceRequirements {
                        requests: Some(pod1_requests),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                ..Default::default()
            }),
        };

        // Pod 2: requests fractional HAMi VRAM 16 GiB
        let mut pod2_requests = BTreeMap::new();
        pod2_requests.insert("nvidia.com/gpumem".to_string(), Quantity("16384".to_string()));

        let pod2 = Pod {
            metadata: ObjectMeta {
                name: Some("inference-worker".to_string()),
                namespace: Some("ai-serving".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                node_name: Some("gpu-node-01".to_string()),
                containers: vec![Container {
                    name: "vllm".to_string(),
                    resources: Some(ResourceRequirements {
                        requests: Some(pod2_requests),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                ..Default::default()
            }),
        };

        // Non-GPU pod on same node (should not be in GPU pods list)
        let pod3 = Pod {
            metadata: ObjectMeta {
                name: Some("log-collector".to_string()),
                namespace: Some("monitoring".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                node_name: Some("gpu-node-01".to_string()),
                containers: vec![Container {
                    name: "fluentbit".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                ..Default::default()
            }),
        };

        let cluster = parse_gpu_cluster_info(&[node], &[pod1, pod2, pod3]);

        assert_eq!(cluster.total_gpu_nodes, 1);
        assert_eq!(cluster.total_gpus, 8);
        assert_eq!(cluster.total_allocated_gpus, 2);
        // Total VRAM for 8 x 80GB = 640 GiB = 655360 MiB
        assert_eq!(cluster.total_vram_mib, 655360);
        // Allocated VRAM: 160 GiB (163840 MiB) + 16 GiB (16384 MiB) = 180224 MiB (176 GiB)
        assert_eq!(cluster.total_allocated_vram_mib, 180224);

        let n = &cluster.nodes[0];
        assert_eq!(n.name, "gpu-node-01");
        assert_eq!(n.gpu_capacity, 8);
        assert_eq!(n.gpu_model.as_deref(), Some("NVIDIA A100 SXM4 80GB"));
        assert_eq!(n.gpu_driver_version.as_deref(), Some("535.129.03"));
        assert_eq!(n.vram_per_gpu_mib, Some(81920));
        assert_eq!(n.vram_capacity_total_mib, Some(655360));
        assert_eq!(n.vram_requests_total_mib, 180224);

        // Only 2 GPU pods listed, sorted by VRAM descending (llm-trainer-0 first, then inference-worker)
        assert_eq!(n.pods.len(), 2);
        assert_eq!(n.pods[0].name, "llm-trainer-0");
        assert_eq!(n.pods[0].gpu_requests, 2);
        assert_eq!(n.pods[0].vram_requests_mib, 163840);
        assert_eq!(format_vram_mib(n.pods[0].vram_requests_mib), "160 GiB");

        assert_eq!(n.pods[1].name, "inference-worker");
        assert_eq!(n.pods[1].gpu_requests, 0);
        assert_eq!(n.pods[1].vram_requests_mib, 16384);
        assert_eq!(format_vram_mib(n.pods[1].vram_requests_mib), "16 GiB");
    }
}
