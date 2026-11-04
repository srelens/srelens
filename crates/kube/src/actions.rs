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
use crate::manifest::{gvk_for, resolve_kind};

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
    Ok(scoped_api(client, &ApiResource::from_gvk(&gvk), namespaced, namespace))
}

/// Whether `plural` in `group/version` is namespaced, by that API's own
/// discovery, which every authenticated user may read. The host's confirmation
/// names the caller's `kind`, so a plural that serves another kind is refused:
/// approving "ConfigMap default/db" must not delete a Secret.
async fn served_scope(
    client: &kube::Client,
    group: &str,
    version: &str,
    plural: &str,
    kind: &str,
) -> Result<bool, CapabilityError> {
    let list = if group.is_empty() {
        tokio::time::timeout(request_timeout(), client.list_core_api_resources(version)).await
    } else {
        tokio::time::timeout(request_timeout(), client.list_api_group_resources(&format!("{group}/{version}"))).await
    }
    .map_err(|_| CapabilityError::Handler("discovery timed out".into()))?
    .map_err(|e| CapabilityError::Handler(format!("discover {plural}: {e}")))?;
    let served = list
        .resources
        .iter()
        .find(|r| r.name == plural)
        .ok_or_else(|| CapabilityError::Handler(format!("{} serves no {plural}", list.group_version)))?;
    if !served.kind.eq_ignore_ascii_case(kind) {
        return Err(CapabilityError::Handler(format!(
            "{plural} in {} are {}, not {kind}",
            list.group_version, served.kind
        )));
    }
    Ok(served.namespaced)
}

fn scoped_api(client: kube::Client, ar: &ApiResource, namespaced: bool, namespace: &str) -> Api<DynamicObject> {
    if namespaced && !namespace.is_empty() {
        Api::namespaced_with(client, namespace, ar)
    } else {
        Api::all_with(client, ar)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteResourceIn {
    pub context: String,
    pub kind: String,
    #[serde(default)]
    pub namespace: String,
    pub name: String,
    /// For a custom resource, its CRD's group, version and plural (e.g.
    /// "traefik.io", "v1alpha1", "ingressroutes"), as `k8s.getManifest` takes
    /// them. A kind outside the built-in table is refused without all three.
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub plural: Option<String>,
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
        "delete any supported resource by kind/namespace/name (destructive); a custom resource also takes its group, version and plural",
        Annotations::DESTRUCTIVE,
        move |input: DeleteResourceIn| {
            let cache = cache.clone();
            async move {
                // Resolved before anything reaches the cluster, by the reads'
                // own resolver: a custom kind by its group/version/plural, a
                // built-in by the table, anything else refused.
                let (ar, namespaced) = resolve_kind(
                    &input.kind,
                    Some(&input.namespace),
                    input.group.as_deref(),
                    input.version.as_deref(),
                    input.plural.as_deref(),
                )?;
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let namespaced = match (&input.group, &input.version, &input.plural) {
                    (Some(g), Some(v), Some(p)) => served_scope(&client, g, v, p, &input.kind).await?,
                    _ => namespaced,
                };
                let api = scoped_api(client, &ar, namespaced, &input.namespace);
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

/// The host-owned rollout patch, shared with the reviewed extension adapter.
pub(crate) fn restart_patch() -> serde_json::Value {
    let now = k8s_openapi::jiff::Timestamp::now().to_string();
    json!({"spec":{"template":{"metadata":{"annotations":{"kubectl.kubernetes.io/restartedAt":now}}}}})
}

/// The host-owned scheduling patch, shared with the reviewed extension adapter.
pub(crate) fn cordon_patch(unschedulable: bool) -> serde_json::Value {
    json!({"spec":{"unschedulable":unschedulable}})
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
                let patch = restart_patch();
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
                let patch = cordon_patch(input.unschedulable);
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

    /// What the fake API server's discovery says a group/version serves.
    fn served(path: &str) -> Option<serde_json::Value> {
        let resource = |name: &str, kind: &str, namespaced: bool| {
            json!({ "name": name, "singularName": "", "kind": kind, "namespaced": namespaced, "verbs": ["delete"] })
        };
        let (group_version, resources) = match path {
            "/apis/acme.io/v1" => (
                "acme.io/v1",
                vec![resource("widgets", "Widget", true), resource("deployments", "Deployment", true)],
            ),
            "/apis/acme.io/v1beta1" => ("acme.io/v1beta1", vec![resource("gadgets", "Gadget", false)]),
            "/api/v1" => ("v1", vec![resource("secrets", "Secret", true), resource("configmaps", "ConfigMap", true)]),
            _ => return None,
        };
        Some(json!({ "kind": "APIResourceList", "apiVersion": "v1", "groupVersion": group_version, "resources": resources }))
    }

    /// Runs `k8s.deleteResource` against a fake API server that answers
    /// discovery from [`served`] and every other request with `status` and
    /// `body`, and hands back what it was asked.
    async fn delete(
        input: serde_json::Value,
        status: u16,
        body: serde_json::Value,
    ) -> (Result<serde_json::Value, CapabilityError>, Vec<String>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            log.lock()
                .unwrap()
                .push(format!("{} {}", request.method(), request.uri().path()));
            let discovery = (request.method() == http::Method::GET)
                .then(|| served(request.uri().path()))
                .flatten();
            let (status, body) = match discovery {
                Some(list) => (200, list.to_string()),
                None => (status, body.to_string()),
            };
            async move {
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.into_bytes()))
                        .unwrap(),
                )
            }
        });
        let cache = cache();
        cache.preload("prod", kube::Client::new(service, "default")).await;
        let out = (delete_resource_capability(cache).handler)(input).await;
        let seen = seen.lock().unwrap().clone();
        (out, seen)
    }

    fn deleted(kind: &str, name: &str) -> serde_json::Value {
        json!({ "apiVersion": "v1", "kind": kind, "metadata": { "name": name } })
    }

    #[tokio::test]
    async fn delete_resource_reaches_a_namespaced_custom_resource_through_its_crd() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "Widget", "namespace": "default", "name": "left",
                    "group": "acme.io", "version": "v1", "plural": "widgets" }),
            200,
            deleted("Widget", "left"),
        )
        .await;
        out.expect("the delete is sent");
        assert_eq!(seen, ["GET /apis/acme.io/v1", "DELETE /apis/acme.io/v1/namespaces/default/widgets/left"]);
    }

    #[tokio::test]
    async fn delete_resource_reaches_a_cluster_scoped_custom_resource_with_no_namespace() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "Gadget", "namespace": "", "name": "big",
                    "group": "acme.io", "version": "v1beta1", "plural": "gadgets" }),
            200,
            deleted("Gadget", "big"),
        )
        .await;
        out.expect("the delete is sent");
        assert_eq!(seen, ["GET /apis/acme.io/v1beta1", "DELETE /apis/acme.io/v1beta1/gadgets/big"]);
    }

    /// A CRD may reuse a built-in kind's name in its own group. Resolved by the
    /// kind alone, this delete went to `apps/v1` — another object entirely.
    #[tokio::test]
    async fn delete_resource_deletes_a_crd_named_like_a_built_in_in_its_own_group() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "Deployment", "namespace": "default", "name": "api",
                    "group": "acme.io", "version": "v1", "plural": "deployments" }),
            200,
            deleted("Deployment", "api"),
        )
        .await;
        out.expect("the delete is sent");
        assert_eq!(seen, ["GET /apis/acme.io/v1", "DELETE /apis/acme.io/v1/namespaces/default/deployments/api"]);
    }

    #[tokio::test]
    async fn delete_resource_still_resolves_a_built_in_kind_by_its_name() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "ConfigMap", "namespace": "default", "name": "web" }),
            200,
            deleted("ConfigMap", "web"),
        )
        .await;
        out.expect("the delete is sent");
        assert_eq!(seen, ["DELETE /api/v1/namespaces/default/configmaps/web"]);
    }

    /// A kind outside the built-in table, with nothing to say which API serves
    /// it, is refused before anything is sent — never resolved by a guess.
    #[tokio::test]
    async fn delete_resource_refuses_a_custom_kind_given_by_name_alone() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "Widget", "namespace": "default", "name": "left" }),
            200,
            deleted("Widget", "left"),
        )
        .await;
        let Err(CapabilityError::Handler(message)) = out else {
            panic!("a bare custom kind must be refused, got {out:?}");
        };
        assert!(message.contains("unsupported kind: Widget"), "{message}");
        assert!(seen.is_empty(), "nothing may reach the cluster: {seen:?}");
    }

    #[tokio::test]
    async fn delete_resource_reports_an_rbac_refusal_as_a_failure() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "Widget", "namespace": "default", "name": "left",
                    "group": "acme.io", "version": "v1", "plural": "widgets" }),
            403,
            json!({ "apiVersion": "v1", "kind": "Status", "status": "Failure", "reason": "Forbidden", "code": 403,
                    "message": "widgets.acme.io \"left\" is forbidden: User \"jane\" cannot delete resource \"widgets\" in API group \"acme.io\" in the namespace \"default\"" }),
        )
        .await;
        let Err(CapabilityError::Handler(message)) = out else {
            panic!("a refusal must not read as success, got {out:?}");
        };
        assert!(message.contains("forbidden"), "{message}");
        assert_eq!(seen, ["GET /apis/acme.io/v1", "DELETE /apis/acme.io/v1/namespaces/default/widgets/left"]);
    }

    /// The confirmation names the caller's kind, so that kind must be what the
    /// plural reaches: approving "ConfigMap default/db" must not delete a Secret.
    #[tokio::test]
    async fn delete_resource_refuses_a_plural_that_serves_another_kind() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "ConfigMap", "namespace": "default", "name": "db",
                    "group": "", "version": "v1", "plural": "secrets" }),
            200,
            deleted("Secret", "db"),
        )
        .await;
        let Err(CapabilityError::Handler(message)) = out else {
            panic!("a kind the plural does not serve must be refused, got {out:?}");
        };
        assert!(message.contains("Secret"), "{message}");
        assert_eq!(seen, ["GET /api/v1"], "nothing may be deleted");
    }

    /// The scope is the cluster's answer, not a guess from whether a namespace
    /// came along.
    #[tokio::test]
    async fn delete_resource_takes_a_custom_kind_s_scope_from_discovery() {
        let (out, seen) = delete(
            json!({ "context": "prod", "kind": "Gadget", "namespace": "default", "name": "big",
                    "group": "acme.io", "version": "v1beta1", "plural": "gadgets" }),
            200,
            deleted("Gadget", "big"),
        )
        .await;
        out.expect("the delete is sent");
        assert_eq!(seen, ["GET /apis/acme.io/v1beta1", "DELETE /apis/acme.io/v1beta1/gadgets/big"]);
    }

    /// Group, version and plural become path segments, so each must be a name
    /// Kubernetes could have given it.
    #[tokio::test]
    async fn delete_resource_refuses_a_group_version_or_plural_that_is_not_a_name() {
        for (group, version, plural) in [
            ("acme.io", "v1", "widgets/../../../api/v1/namespaces/default/secrets"),
            ("acme.io/../../api", "v1", "secrets"),
            ("acme.io", "v1/..", "widgets"),
            ("acme.io", "v1", "widgets%2Fx"),
        ] {
            let (out, seen) = delete(
                json!({ "context": "prod", "kind": "Widget", "namespace": "default", "name": "left",
                        "group": group, "version": version, "plural": plural }),
                200,
                deleted("Widget", "left"),
            )
            .await;
            assert!(out.is_err(), "{group}/{version}/{plural} must be refused, got {out:?}");
            assert!(seen.is_empty(), "nothing may reach the cluster for {group}/{version}/{plural}: {seen:?}");
        }
    }
}
