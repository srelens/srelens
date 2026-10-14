//! Tool invoker filtering and curated SRE tool allowlist.
//!
//! Exposing all 120+ MCP tool schemas to an in-process agent consumes 15,000+ tokens
//! on every request before any user message is evaluated. `FilteredToolInvoker`
//! scopes the native agent's advertised tools to the curated `CORE_SRE_TOOLS` set,
//! preserving full diagnostic and remediation capabilities while reducing static
//! schema overhead by ~70%.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::LlmError;
use crate::provider::{ToolCallResult, ToolInvoker};
use crate::types::ToolDef;

/// Core 41 Kubernetes SRE tools essential for workload diagnosis, manifest inspection,
/// event timeline triage, node health, networking, and safe remediation.
pub const CORE_SRE_TOOLS: &[&str] = &[
    // Workloads & Pods
    "k8s.listPods",
    "k8s.listDeployments",
    "k8s.listStatefulSets",
    "k8s.listDaemonSets",
    "k8s.listJobs",
    "k8s.listCronJobs",
    "k8s.listReplicaSets",
    "k8s.podOverview",
    "k8s.podCount",
    "k8s.podsOnNode",
    "k8s.podsForSelector",
    // Manifests & Objects
    "k8s.getManifest",
    "k8s.getObject",
    "k8s.diffManifest",
    "k8s.getCustomResource",
    "k8s.listCustomResource",
    "k8s.listResource",
    // Diagnostics & Triage
    "k8s.listChanges",
    "k8s.podLogs",
    "k8s.listEvents",
    "k8s.nodeMetrics",
    "k8s.podMetrics",
    // Cluster & Topology
    "k8s.listNodes",
    "k8s.listNamespaces",
    "k8s.clusterInfo",
    "k8s.clusterFacts",
    "k8s.topologyGraph",
    // Networking & Storage
    "k8s.listServices",
    "k8s.listEndpoints",
    "k8s.listEndpointSlices",
    "k8s.listIngresses",
    "k8s.listConfigMaps",
    "k8s.listNetworkPolicies",
    "k8s.listPersistentVolumeClaims",
    "k8s.listStorageClasses",
    // Actions & Safe Mutations
    "k8s.rolloutRestart",
    "k8s.scale",
    "k8s.deletePod",
    "k8s.cordonNode",
    // Monitoring & Connectivity
    "k8s.prometheusQuery",
    "ping",
];

/// A decorator that restricts `list_tools` to a permitted subset of tool names,
/// while delegating `call_tool` transparently to the inner invoker.
pub struct FilteredToolInvoker<T> {
    inner: T,
    allowed: HashSet<String>,
}

impl<T> FilteredToolInvoker<T> {
    pub fn new(inner: T, allowed_names: &[&str]) -> Self {
        let mut allowed = HashSet::with_capacity(allowed_names.len() * 2);
        for &name in allowed_names {
            allowed.insert(name.to_string());
            // Support provider-safe underscore aliases (e.g. k8s_listPods)
            allowed.insert(name.replace('.', "_"));
        }
        Self { inner, allowed }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }
}

#[async_trait]
impl<T: ToolInvoker + ?Sized> ToolInvoker for Arc<T> {
    async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
        (**self).list_tools().await
    }

    async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolCallResult, LlmError> {
        (**self).call_tool(name, args).await
    }
}

#[async_trait]
impl<T: ToolInvoker> ToolInvoker for FilteredToolInvoker<T> {
    async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
        let all = self.inner.list_tools().await?;
        Ok(all
            .into_iter()
            .filter(|t| self.allowed.contains(&t.name))
            .collect())
    }

    async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolCallResult, LlmError> {
        self.inner.call_tool(name, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct StubInvoker {
        tools: Vec<ToolDef>,
    }

    #[async_trait]
    impl ToolInvoker for StubInvoker {
        async fn list_tools(&self) -> Result<Vec<ToolDef>, LlmError> {
            Ok(self.tools.clone())
        }

        async fn call_tool(&self, name: &str, args: &Value) -> Result<ToolCallResult, LlmError> {
            Ok(ToolCallResult {
                content: format!("called {name} with {args}"),
                is_error: false,
                denied: false,
            })
        }
    }

    fn stub_tool(name: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: format!("Description for {name}"),
            input_schema: json!({ "type": "object" }),
            read_only: true,
        }
    }

    #[tokio::test]
    async fn test_filtered_invoker_allows_dotted_and_underscored_names() {
        let stub = StubInvoker {
            tools: vec![
                stub_tool("k8s.listPods"),
                stub_tool("k8s_listDeployments"),
                stub_tool("unrelated.secretDump"),
                stub_tool("k8s.rolloutRestart"),
            ],
        };

        let filtered = FilteredToolInvoker::new(stub, CORE_SRE_TOOLS);
        let tools = filtered.list_tools().await.unwrap();

        let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec!["k8s.listPods", "k8s_listDeployments", "k8s.rolloutRestart"]
        );
    }

    #[tokio::test]
    async fn test_filtered_invoker_delegates_call_tool() {
        let stub = StubInvoker { tools: vec![] };
        let filtered = FilteredToolInvoker::new(stub, CORE_SRE_TOOLS);
        let res = filtered
            .call_tool("k8s.listPods", &json!({ "namespace": "default" }))
            .await
            .unwrap();
        assert_eq!(
            res.content,
            "called k8s.listPods with {\"namespace\":\"default\"}"
        );
    }

    #[tokio::test]
    async fn test_arc_tool_invoker_delegates() {
        let stub = Arc::new(StubInvoker {
            tools: vec![stub_tool("k8s.listPods")],
        });
        let tools = stub.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn test_core_sre_tools_count_and_inclusions() {
        assert_eq!(CORE_SRE_TOOLS.len(), 41);
        assert!(CORE_SRE_TOOLS.contains(&"k8s.listResource"));
        assert!(CORE_SRE_TOOLS.contains(&"k8s.listCustomResource"));
        assert!(CORE_SRE_TOOLS.contains(&"k8s.listEndpoints"));
        assert!(CORE_SRE_TOOLS.contains(&"k8s.listEndpointSlices"));
        assert!(CORE_SRE_TOOLS.contains(&"k8s.podLogs"));
    }
}
