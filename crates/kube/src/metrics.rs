//! Metrics capabilities (metrics.k8s.io) — pod and node CPU/memory usage.
//! Best-effort: returns an error if metrics-server is not installed.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use kube::api::{Api, DynamicObject, ListParams};
use kube::core::{ApiResource, GroupVersionKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Parse a Kubernetes CPU quantity to integer millicores.
pub fn cpu_millicores(s: &str) -> i64 {
    let s = s.trim();
    let parse = |suffix: &str| s.trim_end_matches(suffix).parse::<f64>().unwrap_or(0.0);
    if let Some(rest) = s.strip_suffix('n') {
        (rest.parse::<f64>().unwrap_or(0.0) / 1_000_000.0) as i64
    } else if let Some(rest) = s.strip_suffix('u') {
        (rest.parse::<f64>().unwrap_or(0.0) / 1_000.0) as i64
    } else if s.ends_with('m') {
        parse("m") as i64
    } else {
        (parse("") * 1000.0) as i64
    }
}

/// Parse a Kubernetes memory quantity to integer MiB.
pub fn mem_mib(s: &str) -> i64 {
    let s = s.trim();
    let num = |suffix: &str| s.trim_end_matches(suffix).parse::<f64>().unwrap_or(0.0);
    if s.ends_with("Ki") {
        (num("Ki") / 1024.0) as i64
    } else if s.ends_with("Mi") {
        num("Mi") as i64
    } else if s.ends_with("Gi") {
        (num("Gi") * 1024.0) as i64
    } else if s.ends_with("Ti") {
        (num("Ti") * 1024.0 * 1024.0) as i64
    } else {
        (num("") / 1_048_576.0) as i64
    }
}

fn metrics_api(client: kube::Client, kind: &str, namespaced: bool, namespace: &str) -> Api<DynamicObject> {
    let gvk = GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", kind);
    let ar = ApiResource::from_gvk(&gvk);
    if namespaced && !namespace.is_empty() {
        Api::namespaced_with(client, namespace, &ar)
    } else {
        Api::all_with(client, &ar)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NodeMetricsIn {
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct NodeMetric {
    pub name: String,
    #[serde(rename = "cpuMillicores")]
    pub cpu_millicores: i64,
    #[serde(rename = "memoryMiB")]
    pub memory_mib: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NodeMetricsOut {
    pub metrics: Vec<NodeMetric>,
}

/// `k8s.nodeMetrics` — per-node CPU (millicores) and memory (MiB) usage.
pub fn node_metrics_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<NodeMetricsIn, NodeMetricsOut, _, _>(
        "k8s.nodeMetrics",
        "node CPU/memory usage (requires metrics-server)",
        Annotations::READ_ONLY,
        move |input: NodeMetricsIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api = metrics_api(client, "NodeMetrics", false, "");
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("node metrics timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let metrics = list
                    .items
                    .into_iter()
                    .map(|o| {
                        let usage = &o.data["usage"];
                        NodeMetric {
                            name: o.metadata.name.unwrap_or_default(),
                            cpu_millicores: cpu_millicores(usage["cpu"].as_str().unwrap_or("0")),
                            memory_mib: mem_mib(usage["memory"].as_str().unwrap_or("0")),
                        }
                    })
                    .collect();
                Ok(NodeMetricsOut { metrics })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PodMetricsIn {
    pub context: String,
    #[serde(default)]
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PodMetric {
    pub name: String,
    pub namespace: String,
    #[serde(rename = "cpuMillicores")]
    pub cpu_millicores: i64,
    #[serde(rename = "memoryMiB")]
    pub memory_mib: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PodMetricsOut {
    pub metrics: Vec<PodMetric>,
}

fn sum_pod_usage(containers: &Value) -> (i64, i64) {
    let mut cpu = 0;
    let mut mem = 0;
    if let Some(arr) = containers.as_array() {
        for c in arr {
            let u = &c["usage"];
            cpu += cpu_millicores(u["cpu"].as_str().unwrap_or("0"));
            mem += mem_mib(u["memory"].as_str().unwrap_or("0"));
        }
    }
    (cpu, mem)
}

/// `k8s.podMetrics` — per-pod CPU (millicores) and memory (MiB) usage.
pub fn pod_metrics_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodMetricsIn, PodMetricsOut, _, _>(
        "k8s.podMetrics",
        "pod CPU/memory usage (requires metrics-server)",
        Annotations::READ_ONLY,
        move |input: PodMetricsIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api = metrics_api(client, "PodMetrics", true, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("pod metrics timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let metrics = list
                    .items
                    .into_iter()
                    .map(|o| {
                        let (cpu, mem) = sum_pod_usage(&o.data["containers"]);
                        PodMetric {
                            name: o.metadata.name.unwrap_or_default(),
                            namespace: o.metadata.namespace.unwrap_or_default(),
                            cpu_millicores: cpu,
                            memory_mib: mem,
                        }
                    })
                    .collect();
                Ok(PodMetricsOut { metrics })
            }
        },
    )
}

/// Fetch per-pod CPU and memory metrics from metrics.k8s.io
pub async fn fetch_pod_metrics(
    cache: Arc<ClientCache>,
    context: &str,
    namespace: &str,
) -> Result<Vec<PodMetric>, String> {
    let client = cache.get(context).await.map_err(|e| e.to_string())?;
    let api = metrics_api(client, "PodMetrics", true, namespace);
    let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
        .await
        .map_err(|_| "pod metrics timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let metrics = list
        .items
        .into_iter()
        .map(|o| {
            let (cpu, mem) = sum_pod_usage(&o.data["containers"]);
            PodMetric {
                name: o.metadata.name.unwrap_or_default(),
                namespace: o.metadata.namespace.unwrap_or_default(),
                cpu_millicores: cpu,
                memory_mib: mem,
            }
        })
        .collect();
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_cpu_quantities() {
        assert_eq!(cpu_millicores("250m"), 250);
        assert_eq!(cpu_millicores("1"), 1000);
        assert_eq!(cpu_millicores("123456789n"), 123);
        assert_eq!(cpu_millicores("5000u"), 5);
    }

    #[test]
    fn parses_memory_quantities() {
        assert_eq!(mem_mib("131072Ki"), 128);
        assert_eq!(mem_mib("256Mi"), 256);
        assert_eq!(mem_mib("1Gi"), 1024);
    }

    #[test]
    fn capabilities_have_ids() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(node_metrics_capability(cache.clone()).id, "k8s.nodeMetrics");
        assert_eq!(pod_metrics_capability(cache).id, "k8s.podMetrics");
    }
}
