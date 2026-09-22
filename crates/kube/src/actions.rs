//! Mutating (write) capabilities. These carry the `DESTRUCTIVE` annotation,
//! which drives both MCP tool hints and the UI confirmation dialog.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::api::{Api, DeleteParams, DynamicObject, EvictParams, ListParams, Patch, PatchParams};
use kube::core::ApiResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::manifest::gvk_for;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeletePodIn {
    pub context: String,
    pub namespace: String,
    pub pod: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeletePodOut {
    pub name: String,
    pub deleted: bool,
}

/// `k8s.deletePod` — delete a pod. Destructive: requires confirmation.
pub fn delete_pod_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<DeletePodIn, DeletePodOut, _, _>(
        "k8s.deletePod",
        "delete a pod in a connected kube context (destructive)",
        Annotations::DESTRUCTIVE,
        move |input: DeletePodIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = Api::namespaced(client, &input.namespace);
                tokio::time::timeout(request_timeout(), api.delete(&input.pod, &DeleteParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("delete pod timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(DeletePodOut {
                    name: input.pod,
                    deleted: true,
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EvictPodIn {
    pub context: String,
    pub namespace: String,
    pub pod: String,
}

/// `k8s.evictPod` — evict a pod via the eviction API (graceful, respects
/// PodDisruptionBudgets), unlike a raw delete. Destructive.
pub fn evict_pod_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<EvictPodIn, ActionOut, _, _>(
        "k8s.evictPod",
        "evict a pod via the eviction API (respects PodDisruptionBudgets)",
        Annotations::DESTRUCTIVE,
        move |input: EvictPodIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = Api::namespaced(client, &input.namespace);
                tokio::time::timeout(request_timeout(), api.evict(&input.pod, &EvictParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("evict pod timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ActionOut { name: input.pod, ok: true })
            }
        },
    )
}

// ---- Generic write actions over any supported kind ----

fn dynamic_api(
    client: kube::Client,
    kind: &str,
    namespace: &str,
) -> Result<Api<DynamicObject>, CapabilityError> {
    let (gvk, namespaced) =
        gvk_for(kind).ok_or_else(|| CapabilityError::Handler(format!("unsupported kind: {kind}")))?;
    let ar = ApiResource::from_gvk(&gvk);
    Ok(if namespaced && !namespace.is_empty() {
        Api::namespaced_with(client, namespace, &ar)
    } else {
        Api::all_with(client, &ar)
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteResourceIn {
    pub context: String,
    pub kind: String,
    #[serde(default)]
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActionOut {
    pub name: String,
    pub ok: bool,
}

/// Build the merge-patch body for an in-place ConfigMap/Secret data edit.
///
/// ConfigMap values are plain strings under `data`. Secret values go under
/// `stringData` (write-only) so the apiserver base64-encodes them — the UI
/// therefore never has to encode, and no existing values are touched beyond the
/// keys provided (a merge patch only updates the given keys).
pub(crate) fn build_config_patch(
    kind: &str,
    data: &std::collections::BTreeMap<String, String>,
) -> Result<serde_json::Value, String> {
    let field = match kind {
        "ConfigMap" => "data",
        "Secret" => "stringData",
        other => return Err(format!("in-place data edit not supported for kind: {other}")),
    };
    Ok(json!({ field: data }))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateConfigDataIn {
    pub context: String,
    /// "ConfigMap" or "Secret".
    pub kind: String,
    pub namespace: String,
    pub name: String,
    /// Keys to set to new (plaintext) values; other keys are left untouched.
    pub data: std::collections::BTreeMap<String, String>,
}

/// `k8s.updateConfigData` — edit ConfigMap/Secret values in place via a merge
/// patch. Requires confirmation.
pub fn update_config_data_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<UpdateConfigDataIn, ActionOut, _, _>(
        "k8s.updateConfigData",
        "update ConfigMap or Secret values in place (merge patch)",
        Annotations::MUTATING.with_confirm("Overwrite values[ in {resource}][ in cluster {cluster}]?"),
        move |input: UpdateConfigDataIn| {
            let cache = cache.clone();
            async move {
                let patch = build_config_patch(&input.kind, &input.data)
                    .map_err(CapabilityError::Handler)?;
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api = dynamic_api(client, &input.kind, &input.namespace)?;
                tokio::time::timeout(
                    request_timeout(),
                    api.patch(&input.name, &PatchParams::default(), &Patch::Merge(&patch)),
                )
                .await
                .map_err(|_| CapabilityError::Handler("update config data timed out".into()))?
                .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ActionOut { name: input.name, ok: true })
            }
        },
    )
}

/// `k8s.deleteResource` — delete any supported resource. Destructive.
pub fn delete_resource_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<DeleteResourceIn, ActionOut, _, _>(
        "k8s.deleteResource",
        "delete any supported resource by kind/namespace/name (destructive)",
        Annotations::DESTRUCTIVE,
        move |input: DeleteResourceIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api = dynamic_api(client, &input.kind, &input.namespace)?;
                tokio::time::timeout(request_timeout(), api.delete(&input.name, &DeleteParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("delete timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ActionOut { name: input.name, ok: true })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScaleIn {
    pub context: String,
    /// Workload kind ("Deployment", "StatefulSet", "ReplicaSet").
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub replicas: i32,
}

/// `k8s.scale` — set the replica count of a workload. Requires confirmation.
pub fn scale_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ScaleIn, ActionOut, _, _>(
        "k8s.scale",
        "set the replica count of a workload (Deployment/StatefulSet/ReplicaSet)",
        Annotations::MUTATING
            .with_confirm("Change the replica count[ of {resource}][ in cluster {cluster}]?"),
        move |input: ScaleIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api = dynamic_api(client, &input.kind, &input.namespace)?;
                let patch = json!({ "spec": { "replicas": input.replicas } });
                tokio::time::timeout(
                    request_timeout(),
                    api.patch(&input.name, &PatchParams::default(), &Patch::Merge(&patch)),
                )
                .await
                .map_err(|_| CapabilityError::Handler("scale timed out".into()))?
                .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ActionOut { name: input.name, ok: true })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RestartIn {
    pub context: String,
    /// Workload kind ("Deployment", "StatefulSet", "DaemonSet").
    pub kind: String,
    pub namespace: String,
    pub name: String,
}

/// `k8s.rolloutRestart` — trigger a rolling restart by stamping the pod
/// template (the `kubectl rollout restart` mechanism). Requires confirmation.
pub fn rollout_restart_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<RestartIn, ActionOut, _, _>(
        "k8s.rolloutRestart",
        "trigger a rolling restart of a workload",
        Annotations::MUTATING
            .with_confirm("Roll every pod[ of {resource}][ in cluster {cluster}]?"),
        move |input: RestartIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api = dynamic_api(client, &input.kind, &input.namespace)?;
                let now = k8s_openapi::jiff::Timestamp::now().to_string();
                let patch = json!({
                    "spec": { "template": { "metadata": { "annotations": {
                        "kubectl.kubernetes.io/restartedAt": now
                    }}}}
                });
                tokio::time::timeout(
                    request_timeout(),
                    api.patch(&input.name, &PatchParams::default(), &Patch::Merge(&patch)),
                )
                .await
                .map_err(|_| CapabilityError::Handler("restart timed out".into()))?
                .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ActionOut { name: input.name, ok: true })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CordonIn {
    pub context: String,
    pub name: String,
    /// true = cordon (mark unschedulable), false = uncordon.
    pub unschedulable: bool,
}

/// `k8s.cordonNode` — cordon/uncordon a node by setting `spec.unschedulable`.
/// Requires confirmation.
pub fn cordon_node_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<CordonIn, ActionOut, _, _>(
        "k8s.cordonNode",
        "cordon or uncordon a node (set spec.unschedulable)",
        Annotations::MUTATING
            .with_confirm("Change scheduling[ on {resource}][ in cluster {cluster}]?"),
        move |input: CordonIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api: Api<Node> = Api::all(client);
                let patch = json!({ "spec": { "unschedulable": input.unschedulable } });
                tokio::time::timeout(
                    request_timeout(),
                    api.patch(&input.name, &PatchParams::default(), &Patch::Merge(&patch)),
                )
                .await
                .map_err(|_| CapabilityError::Handler("cordon timed out".into()))?
                .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ActionOut { name: input.name, ok: true })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DrainIn {
    pub context: String,
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DrainOut {
    pub name: String,
    pub evicted: i32,
    /// Pods left in place (DaemonSet-managed, mirror/static, or eviction-blocked).
    pub skipped: i32,
}

/// `k8s.drainNode` — cordon a node and evict its evictable pods (skipping
/// DaemonSet-managed and mirror/static pods). Destructive.
pub fn drain_node_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<DrainIn, DrainOut, _, _>(
        "k8s.drainNode",
        "cordon a node and evict its evictable pods (destructive)",
        Annotations::DESTRUCTIVE,
        move |input: DrainIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                // 1. Cordon so nothing reschedules onto the node mid-drain.
                let nodes: Api<Node> = Api::all(client.clone());
                let patch = json!({ "spec": { "unschedulable": true } });
                tokio::time::timeout(
                    request_timeout(),
                    nodes.patch(&input.name, &PatchParams::default(), &Patch::Merge(&patch)),
                )
                .await
                .map_err(|_| CapabilityError::Handler("cordon timed out".into()))?
                .map_err(|e| CapabilityError::Handler(e.to_string()))?;

                // 2. Find the pods scheduled on this node.
                let pods: Api<Pod> = Api::all(client.clone());
                let lp = ListParams::default().fields(&format!("spec.nodeName={}", input.name));
                let list = tokio::time::timeout(request_timeout(), pods.list(&lp))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pods timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;

                // 3. Evict the evictable ones (best-effort; PDB-blocked → skipped).
                let mut evicted = 0;
                let mut skipped = 0;
                for p in list.items {
                    let ns = p.metadata.namespace.clone().unwrap_or_default();
                    let name = p.metadata.name.clone().unwrap_or_default();
                    let is_daemonset = p
                        .metadata
                        .owner_references
                        .iter()
                        .flatten()
                        .any(|o| o.kind == "DaemonSet");
                    let is_mirror = p
                        .metadata
                        .annotations
                        .as_ref()
                        .map(|a| a.contains_key("kubernetes.io/config.mirror"))
                        .unwrap_or(false);
                    if is_daemonset || is_mirror || name.is_empty() {
                        skipped += 1;
                        continue;
                    }
                    let api: Api<Pod> = Api::namespaced(client.clone(), &ns);
                    match tokio::time::timeout(request_timeout(), api.evict(&name, &EvictParams::default()))
                        .await
                    {
                        Ok(Ok(_)) => evicted += 1,
                        _ => skipped += 1,
                    }
                }
                Ok(DrainOut { name: input.name, evicted, skipped })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cache() -> Arc<ClientCache> {
        ClientCache::new(PathBuf::from("/x"))
    }

    #[test]
    fn config_patch_uses_data_for_configmap_and_stringdata_for_secret() {
        let mut data = std::collections::BTreeMap::new();
        data.insert("app.conf".to_string(), "level=debug".to_string());
        let cm = build_config_patch("ConfigMap", &data).unwrap();
        assert_eq!(cm, serde_json::json!({ "data": { "app.conf": "level=debug" } }));
        // Secret uses stringData so the apiserver base64-encodes it; the UI sends plaintext.
        let sec = build_config_patch("Secret", &data).unwrap();
        assert_eq!(sec, serde_json::json!({ "stringData": { "app.conf": "level=debug" } }));
    }

    #[test]
    fn config_patch_rejects_unsupported_kinds() {
        let data = std::collections::BTreeMap::new();
        assert!(build_config_patch("Pod", &data).is_err());
    }

    #[test]
    fn update_config_data_requires_confirm() {
        let cap = update_config_data_capability(cache());
        assert_eq!(cap.id, "k8s.updateConfigData");
        assert!(cap.annotations.requires_confirm);
        assert!(!cap.annotations.read_only);
    }

    #[test]
    fn delete_pod_is_destructive_and_requires_confirm() {
        let cap = delete_pod_capability(cache());
        assert_eq!(cap.id, "k8s.deletePod");
        assert!(cap.annotations.destructive);
        assert!(cap.annotations.requires_confirm);
        assert!(!cap.annotations.read_only);
    }

    #[test]
    fn write_actions_have_expected_ids_and_confirm() {
        assert_eq!(delete_resource_capability(cache()).id, "k8s.deleteResource");
        let scale = scale_capability(cache());
        assert_eq!(scale.id, "k8s.scale");
        assert!(scale.annotations.requires_confirm);
        assert!(!scale.annotations.read_only);
        let restart = rollout_restart_capability(cache());
        assert_eq!(restart.id, "k8s.rolloutRestart");
        assert!(restart.annotations.requires_confirm);
        let cordon = cordon_node_capability(cache());
        assert_eq!(cordon.id, "k8s.cordonNode");
        assert!(cordon.annotations.requires_confirm);
        assert!(!cordon.annotations.read_only);
        let drain = drain_node_capability(cache());
        assert_eq!(drain.id, "k8s.drainNode");
        assert!(drain.annotations.destructive);
        assert!(drain.annotations.requires_confirm);
        let evict = evict_pod_capability(cache());
        assert_eq!(evict.id, "k8s.evictPod");
        assert!(evict.annotations.destructive);
        assert!(evict.annotations.requires_confirm);
    }
}
