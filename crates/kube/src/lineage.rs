//! Kubernetes resource lineage & relationship resolver.
//! Traces owner hierarchies (upwards), dependents (downwards), and
//! linked networking, configuration, and storage (cross-links).

use std::collections::HashMap;
use std::time::Duration;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{Api, DynamicObject, ListParams};
use kube::core::ApiResource;
use kube::Client;
use serde::{Deserialize, Serialize};

/// Type of relationship in the lineage tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineageRelation {
    /// The primary resource requested.
    Target,
    /// An ancestor/parent resource (via ownerReferences).
    Owner,
    /// A dependent child resource (e.g. Pod of a ReplicaSet, Container of a Pod).
    Child,
    /// A networking Service exposing this workload/pod.
    Service,
    /// An Ingress routing traffic to this service.
    Ingress,
    /// A mounted or referenced ConfigMap.
    Config,
    /// A mounted or referenced Secret.
    Secret,
    /// A PersistentVolumeClaim or PersistentVolume.
    Storage,
    /// A Node hosting this pod.
    Node,
}

impl LineageRelation {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Target => "[TARGET]",
            Self::Owner => "[OWNER]",
            Self::Child => "[CHILD]",
            Self::Service => "[SERVICE]",
            Self::Ingress => "[INGRESS]",
            Self::Config => "[CONFIG]",
            Self::Secret => "[SECRET]",
            Self::Storage => "[STORAGE]",
            Self::Node => "[NODE]",
        }
    }
}

/// A node in the resource lineage hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineageNode {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    pub status: Option<String>,
    pub details: Option<String>,
    pub relationship: LineageRelation,
    pub children: Vec<LineageNode>,
}

impl LineageNode {
    pub fn new(
        kind: impl Into<String>,
        name: impl Into<String>,
        namespace: Option<String>,
        relationship: LineageRelation,
    ) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            namespace,
            status: None,
            details: None,
            relationship,
            children: Vec::new(),
        }
    }
}

/// Helper to get a dynamic API for a resource kind.
fn dynamic_api(
    client: &Client,
    kind: &str,
    namespace: Option<&str>,
) -> Option<(Api<DynamicObject>, bool)> {
    let (gvk, namespaced) = crate::manifest::gvk_for(kind)?;
    let ar = ApiResource::from_gvk(&gvk);
    let api = if namespaced && namespace.is_some() {
        Api::namespaced_with(client.clone(), namespace.unwrap(), &ar)
    } else {
        Api::all_with(client.clone(), &ar)
    };
    Some((api, namespaced))
}

/// Extract status and brief details from a DynamicObject based on its kind.
fn extract_status_and_details(kind: &str, obj: &DynamicObject) -> (Option<String>, Option<String>) {
    let status_obj = obj.data.get("status");
    let spec_obj = obj.data.get("spec");

    match kind.to_lowercase().as_str() {
        "pod" | "pods" => {
            let phase = status_obj
                .and_then(|s| s.get("phase"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();

            // Check container statuses for waiting reasons or restarts
            let mut container_details = Vec::new();
            if let Some(c_statuses) = status_obj.and_then(|s| s.get("containerStatuses")).and_then(|v| v.as_array()) {
                let ready_count = c_statuses.iter().filter(|c| c.get("ready").and_then(|v| v.as_bool()).unwrap_or(false)).count();
                let total = c_statuses.len();
                container_details.push(format!("Ready: {}/{}", ready_count, total));

                for c in c_statuses {
                    if let Some(waiting) = c.pointer("/state/waiting/reason").and_then(|v| v.as_str()) {
                        container_details.push(waiting.to_string());
                        break;
                    }
                }
            }

            let node = spec_obj.and_then(|s| s.get("nodeName")).and_then(|v| v.as_str()).unwrap_or("");
            let details = if !node.is_empty() {
                Some(format!("node: {}, {}", node, container_details.join(", ")))
            } else if !container_details.is_empty() {
                Some(container_details.join(", "))
            } else {
                None
            };

            (Some(phase), details)
        }
        "deployment" | "deployments" => {
            let replicas = status_obj.and_then(|s| s.get("replicas")).and_then(|v| v.as_i64()).unwrap_or(0);
            let ready = status_obj.and_then(|s| s.get("readyReplicas")).and_then(|v| v.as_i64()).unwrap_or(0);
            let updated = status_obj.and_then(|s| s.get("updatedReplicas")).and_then(|v| v.as_i64()).unwrap_or(0);
            (
                Some(format!("Ready: {}/{}", ready, replicas)),
                Some(format!("Up-to-date: {}", updated)),
            )
        }
        "replicaset" | "replicasets" => {
            let replicas = status_obj.and_then(|s| s.get("replicas")).and_then(|v| v.as_i64()).unwrap_or(0);
            let ready = status_obj.and_then(|s| s.get("readyReplicas")).and_then(|v| v.as_i64()).unwrap_or(0);
            (
                Some(format!("Ready: {}/{}", ready, replicas)),
                None,
            )
        }
        "statefulset" | "statefulsets" => {
            let replicas = status_obj.and_then(|s| s.get("replicas")).and_then(|v| v.as_i64()).unwrap_or(0);
            let ready = status_obj.and_then(|s| s.get("readyReplicas")).and_then(|v| v.as_i64()).unwrap_or(0);
            (
                Some(format!("Ready: {}/{}", ready, replicas)),
                None,
            )
        }
        "daemonset" | "daemonsets" => {
            let desired = status_obj.and_then(|s| s.get("desiredNumberScheduled")).and_then(|v| v.as_i64()).unwrap_or(0);
            let ready = status_obj.and_then(|s| s.get("numberReady")).and_then(|v| v.as_i64()).unwrap_or(0);
            (
                Some(format!("Ready: {}/{}", ready, desired)),
                None,
            )
        }
        "service" | "services" => {
            let svc_type = spec_obj.and_then(|s| s.get("type")).and_then(|v| v.as_str()).unwrap_or("ClusterIP");
            let cluster_ip = spec_obj.and_then(|s| s.get("clusterIP")).and_then(|v| v.as_str()).unwrap_or("-");
            (
                Some(svc_type.to_string()),
                Some(format!("IP: {}", cluster_ip)),
            )
        }
        "ingress" | "ingresses" => {
            let hosts: Vec<&str> = spec_obj
                .and_then(|s| s.get("rules"))
                .and_then(|v| v.as_array())
                .map(|rules| {
                    rules
                        .iter()
                        .filter_map(|r| r.get("host").and_then(|h| h.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            (
                Some("Active".to_string()),
                if !hosts.is_empty() { Some(hosts.join(", ")) } else { None },
            )
        }
        "persistentvolumeclaim" | "persistentvolumeclaims" => {
            let phase = status_obj.and_then(|s| s.get("phase")).and_then(|v| v.as_str()).unwrap_or("Unknown");
            let vol = spec_obj.and_then(|s| s.get("volumeName")).and_then(|v| v.as_str()).unwrap_or("");
            (
                Some(phase.to_string()),
                if !vol.is_empty() { Some(format!("PV: {}", vol)) } else { None },
            )
        }
        "job" | "jobs" => {
            let succeeded = status_obj.and_then(|s| s.get("succeeded")).and_then(|v| v.as_i64()).unwrap_or(0);
            let active = status_obj.and_then(|s| s.get("active")).and_then(|v| v.as_i64()).unwrap_or(0);
            let failed = status_obj.and_then(|s| s.get("failed")).and_then(|v| v.as_i64()).unwrap_or(0);
            (
                Some(if succeeded > 0 { "Completed" } else if active > 0 { "Running" } else if failed > 0 { "Failed" } else { "Pending" }.to_string()),
                Some(format!("Active: {}, Succeeded: {}, Failed: {}", active, succeeded, failed)),
            )
        }
        _ => (None, None),
    }
}

/// Recursively resolve owner hierarchy upwards, nesting the child node inside its parent,
/// and returning the highest ancestor (root) of the hierarchy.
async fn nest_in_ancestors(
    client: &Client,
    child: LineageNode,
    owners: &[OwnerReference],
    namespace: Option<&str>,
) -> LineageNode {
    if owners.is_empty() {
        return child;
    }

    // Prefer controller owner if present, otherwise first owner
    let primary_owner = owners
        .iter()
        .find(|o| o.controller == Some(true))
        .unwrap_or(&owners[0]);

    let mut parent_node = LineageNode::new(
        &primary_owner.kind,
        &primary_owner.name,
        namespace.map(|s| s.to_string()),
        LineageRelation::Owner,
    );

    let mut grandparents = None;
    if let Some((api, _)) = dynamic_api(client, &primary_owner.kind, namespace) {
        if let Ok(Ok(parent_obj)) = tokio::time::timeout(Duration::from_millis(1500), api.get(&primary_owner.name)).await {
            let (status, details) = extract_status_and_details(&primary_owner.kind, &parent_obj);
            parent_node.status = status;
            parent_node.details = details;
            grandparents = parent_obj.metadata.owner_references;
        }
    }

    parent_node.children.push(child);

    // If there were other non-primary owners on the child, attach them as sibling nodes to parent_node
    for owner in owners {
        if owner.uid != primary_owner.uid && owner.name != primary_owner.name {
            let mut other_node = LineageNode::new(
                &owner.kind,
                &owner.name,
                namespace.map(|s| s.to_string()),
                LineageRelation::Owner,
            );
            if let Some((api, _)) = dynamic_api(client, &owner.kind, namespace) {
                if let Ok(Ok(obj)) = tokio::time::timeout(Duration::from_millis(1500), api.get(&owner.name)).await {
                    let (status, details) = extract_status_and_details(&owner.kind, &obj);
                    other_node.status = status;
                    other_node.details = details;
                }
            }
            parent_node.children.push(other_node);
        }
    }

    // Continue upwards if this parent also has ownerReferences (e.g. ReplicaSet -> Deployment)
    if let Some(gps) = grandparents {
        if !gps.is_empty() {
            return Box::pin(nest_in_ancestors(client, parent_node, &gps, namespace)).await;
        }
    }

    parent_node
}

/// Extract config and storage references from a pod spec.
fn extract_pod_spec_references(spec: &serde_json::Value, namespace: Option<&str>) -> Vec<LineageNode> {
    let mut references = Vec::new();

    if let Some(volumes) = spec.get("volumes").and_then(|v| v.as_array()) {
        for vol in volumes {
            if let Some(cm) = vol.get("configMap").and_then(|v| v.get("name")).and_then(|v| v.as_str()) {
                let mut n = LineageNode::new("ConfigMap", cm, namespace.map(|s| s.to_string()), LineageRelation::Config);
                n.details = Some(format!("Volume: {}", vol.get("name").and_then(|v| v.as_str()).unwrap_or("-")));
                references.push(n);
            }
            if let Some(sec) = vol.get("secret").and_then(|v| v.get("secretName")).and_then(|v| v.as_str()) {
                let mut n = LineageNode::new("Secret", sec, namespace.map(|s| s.to_string()), LineageRelation::Secret);
                n.details = Some(format!("Volume: {}", vol.get("name").and_then(|v| v.as_str()).unwrap_or("-")));
                references.push(n);
            }
            if let Some(pvc) = vol.get("persistentVolumeClaim").and_then(|v| v.get("claimName")).and_then(|v| v.as_str()) {
                let mut n = LineageNode::new("PersistentVolumeClaim", pvc, namespace.map(|s| s.to_string()), LineageRelation::Storage);
                n.details = Some(format!("Volume: {}", vol.get("name").and_then(|v| v.as_str()).unwrap_or("-")));
                references.push(n);
            }
        }
    }

    // Containers envFrom
    if let Some(containers) = spec.get("containers").and_then(|v| v.as_array()) {
        for c in containers {
            if let Some(env_from) = c.get("envFrom").and_then(|v| v.as_array()) {
                for ef in env_from {
                    if let Some(cm) = ef.get("configMapRef").and_then(|v| v.get("name")).and_then(|v| v.as_str()) {
                        let mut n = LineageNode::new("ConfigMap", cm, namespace.map(|s| s.to_string()), LineageRelation::Config);
                        n.details = Some("envFrom".to_string());
                        if !references.iter().any(|r| r.kind == "ConfigMap" && r.name == cm) {
                            references.push(n);
                        }
                    }
                    if let Some(sec) = ef.get("secretRef").and_then(|v| v.get("name")).and_then(|v| v.as_str()) {
                        let mut n = LineageNode::new("Secret", sec, namespace.map(|s| s.to_string()), LineageRelation::Secret);
                        n.details = Some("envFrom".to_string());
                        if !references.iter().any(|r| r.kind == "Secret" && r.name == sec) {
                            references.push(n);
                        }
                    }
                }
            }
        }
    }

    references
}

/// Resolve the full relationship lineage tree for any resource.
pub async fn resolve_resource_lineage(
    client: Client,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<LineageNode, String> {
    let (api, namespaced) = dynamic_api(&client, kind, namespace)
        .ok_or_else(|| format!("Unknown resource kind: {}", kind))?;

    let obj = tokio::time::timeout(Duration::from_secs(3), api.get(name))
        .await
        .map_err(|_| format!("Timed out fetching {}/{}", kind, name))?
        .map_err(|e| format!("Failed to get {}/{}: {}", kind, name, e))?;

    let effective_ns = if namespaced {
        obj.metadata.namespace.as_deref().or(namespace)
    } else {
        None
    };

    let mut target_node = LineageNode::new(
        kind,
        name,
        effective_ns.map(|s| s.to_string()),
        LineageRelation::Target,
    );
    let (status, details) = extract_status_and_details(kind, &obj);
    target_node.status = status;
    target_node.details = details;

    let target_uid = obj.metadata.uid.clone().unwrap_or_default();
    let target_labels = obj.metadata.labels.clone().unwrap_or_default();

    // 1. Resolve Children / Dependents (Downwards)
    let mut dependents = Vec::new();

    let kind_lower = kind.to_lowercase();
    match kind_lower.as_str() {
        "pod" | "pods" => {
            // Containers inside the Pod
            if let Some(containers) = obj.data.pointer("/spec/containers").and_then(|v| v.as_array()) {
                let statuses_map: HashMap<String, (String, i64)> = obj
                    .data
                    .pointer("/status/containerStatuses")
                    .and_then(|v| v.as_array())
                    .map(|cs| {
                        cs.iter()
                            .filter_map(|c| {
                                let c_name = c.get("name")?.as_str()?.to_string();
                                let restarts = c.get("restartCount").and_then(|v| v.as_i64()).unwrap_or(0);
                                let state_str = if c.pointer("/state/running").is_some() {
                                    "Running".to_string()
                                } else if let Some(waiting) = c.pointer("/state/waiting/reason").and_then(|v| v.as_str()) {
                                    waiting.to_string()
                                } else if let Some(term) = c.pointer("/state/terminated/reason").and_then(|v| v.as_str()) {
                                    term.to_string()
                                } else {
                                    "Waiting".to_string()
                                };
                                Some((c_name, (state_str, restarts)))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                for c in containers {
                    if let Some(c_name) = c.get("name").and_then(|v| v.as_str()) {
                        let img = c.get("image").and_then(|v| v.as_str()).unwrap_or("");
                        let (st, restarts) = statuses_map.get(c_name).cloned().unwrap_or(("Waiting".to_string(), 0));
                        let mut c_node = LineageNode::new("Container", c_name, effective_ns.map(|s| s.to_string()), LineageRelation::Child);
                        c_node.status = Some(st);
                        c_node.details = Some(format!("image: {}, restarts: {}", img, restarts));
                        dependents.push(c_node);
                    }
                }
            }

            // Linked configs and storage from pod spec
            if let Some(spec) = obj.data.get("spec") {
                let refs = extract_pod_spec_references(spec, effective_ns);
                dependents.extend(refs);
            }

            // Linked Services matching this pod
            if let Some(ns) = effective_ns {
                if let Some((svc_api, _)) = dynamic_api(&client, "Service", Some(ns)) {
                    if let Ok(Ok(svc_list)) = tokio::time::timeout(Duration::from_millis(1500), svc_api.list(&ListParams::default())).await {
                        for svc in svc_list.items {
                            if let Some(selector) = svc.data.pointer("/spec/selector").and_then(|v| v.as_object()) {
                                if !selector.is_empty() && selector.iter().all(|(k, v)| target_labels.get(k).map(|lv| lv.as_str()) == v.as_str()) {
                                    let svc_name = svc.metadata.name.clone().unwrap_or_default();
                                    let (s_st, s_dt) = extract_status_and_details("Service", &svc);
                                    let mut s_node = LineageNode::new("Service", svc_name, Some(ns.to_string()), LineageRelation::Service);
                                    s_node.status = s_st;
                                    s_node.details = s_dt;
                                    dependents.push(s_node);
                                }
                            }
                        }
                    }
                }
            }
        }
        "deployment" | "deployments" => {
            if let Some(ns) = effective_ns {
                // Find ReplicaSets owned by this Deployment
                if let Some((rs_api, _)) = dynamic_api(&client, "ReplicaSet", Some(ns)) {
                    if let Ok(Ok(rs_list)) = tokio::time::timeout(Duration::from_millis(1500), rs_api.list(&ListParams::default())).await {
                        for rs in rs_list.items {
                            let is_owned = rs.metadata.owner_references.as_ref().map(|owners| {
                                owners.iter().any(|o| o.uid == target_uid || o.name == name)
                            }).unwrap_or(false);

                            if is_owned {
                                let rs_name = rs.metadata.name.clone().unwrap_or_default();
                                let rs_uid = rs.metadata.uid.clone().unwrap_or_default();
                                let (rs_st, rs_dt) = extract_status_and_details("ReplicaSet", &rs);
                                let mut rs_node = LineageNode::new("ReplicaSet", &rs_name, Some(ns.to_string()), LineageRelation::Child);
                                rs_node.status = rs_st;
                                rs_node.details = rs_dt;

                                // Find Pods owned by this ReplicaSet
                                if let Some((pod_api, _)) = dynamic_api(&client, "Pod", Some(ns)) {
                                    if let Ok(Ok(pod_list)) = tokio::time::timeout(Duration::from_millis(1500), pod_api.list(&ListParams::default())).await {
                                        for pod in pod_list.items {
                                            let pod_owned = pod.metadata.owner_references.as_ref().map(|owners| {
                                                owners.iter().any(|o| o.uid == rs_uid || o.name == rs_name)
                                            }).unwrap_or(false);
                                            if pod_owned {
                                                let pod_name = pod.metadata.name.clone().unwrap_or_default();
                                                let (p_st, p_dt) = extract_status_and_details("Pod", &pod);
                                                let mut p_node = LineageNode::new("Pod", pod_name, Some(ns.to_string()), LineageRelation::Child);
                                                p_node.status = p_st;
                                                p_node.details = p_dt;
                                                rs_node.children.push(p_node);
                                            }
                                        }
                                    }
                                }
                                dependents.push(rs_node);
                            }
                        }
                    }
                }

                // Linked Services matching workload's template labels
                if let Some(tmpl_labels) = obj.data.pointer("/spec/template/metadata/labels").and_then(|v| v.as_object()) {
                    if let Some((svc_api, _)) = dynamic_api(&client, "Service", Some(ns)) {
                        if let Ok(Ok(svc_list)) = tokio::time::timeout(Duration::from_millis(1500), svc_api.list(&ListParams::default())).await {
                            for svc in svc_list.items {
                                if let Some(selector) = svc.data.pointer("/spec/selector").and_then(|v| v.as_object()) {
                                    if !selector.is_empty() && selector.iter().all(|(k, v)| tmpl_labels.get(k).and_then(|lv| lv.as_str()) == v.as_str()) {
                                        let svc_name = svc.metadata.name.clone().unwrap_or_default();
                                        let (s_st, s_dt) = extract_status_and_details("Service", &svc);
                                        let mut s_node = LineageNode::new("Service", svc_name, Some(ns.to_string()), LineageRelation::Service);
                                        s_node.status = s_st;
                                        s_node.details = s_dt;
                                        dependents.push(s_node);
                                    }
                                }
                            }
                        }
                    }
                }

                // Referenced ConfigMaps and Secrets in pod template
                if let Some(spec) = obj.data.pointer("/spec/template/spec") {
                    let refs = extract_pod_spec_references(spec, effective_ns);
                    dependents.extend(refs);
                }
            }
        }
        "statefulset" | "statefulsets" | "daemonset" | "daemonsets" => {
            if let Some(ns) = effective_ns {
                if let Some((pod_api, _)) = dynamic_api(&client, "Pod", Some(ns)) {
                    if let Ok(Ok(pod_list)) = tokio::time::timeout(Duration::from_millis(1500), pod_api.list(&ListParams::default())).await {
                        for pod in pod_list.items {
                            let is_owned = pod.metadata.owner_references.as_ref().map(|owners| {
                                owners.iter().any(|o| o.uid == target_uid || o.name == name)
                            }).unwrap_or(false);
                            if is_owned {
                                let pod_name = pod.metadata.name.clone().unwrap_or_default();
                                let (p_st, p_dt) = extract_status_and_details("Pod", &pod);
                                let mut p_node = LineageNode::new("Pod", pod_name, Some(ns.to_string()), LineageRelation::Child);
                                p_node.status = p_st;
                                p_node.details = p_dt;
                                dependents.push(p_node);
                            }
                        }
                    }
                }
            }
        }
        "service" | "services" => {
            if let Some(ns) = effective_ns {
                // Find Pods matching selector
                if let Some(selector) = obj.data.pointer("/spec/selector").and_then(|v| v.as_object()) {
                    if !selector.is_empty() {
                        if let Some((pod_api, _)) = dynamic_api(&client, "Pod", Some(ns)) {
                            if let Ok(Ok(pod_list)) = tokio::time::timeout(Duration::from_millis(1500), pod_api.list(&ListParams::default())).await {
                                for pod in pod_list.items {
                                    let pod_labels = pod.metadata.labels.as_ref();
                                    let matches = selector.iter().all(|(k, v)| {
                                        pod_labels.and_then(|pl| pl.get(k)).map(|lv| lv.as_str()) == v.as_str()
                                    });
                                    if matches {
                                        let pod_name = pod.metadata.name.clone().unwrap_or_default();
                                        let (p_st, p_dt) = extract_status_and_details("Pod", &pod);
                                        let mut p_node = LineageNode::new("Pod", pod_name, Some(ns.to_string()), LineageRelation::Child);
                                        p_node.status = p_st;
                                        p_node.details = p_dt;
                                        dependents.push(p_node);
                                    }
                                }
                            }
                        }
                    }
                }

                // Find Ingresses pointing to this Service
                if let Some((ing_api, _)) = dynamic_api(&client, "Ingress", Some(ns)) {
                    if let Ok(Ok(ing_list)) = tokio::time::timeout(Duration::from_millis(1500), ing_api.list(&ListParams::default())).await {
                        for ing in ing_list.items {
                            let points_to_svc = ing
                                .data
                                .pointer("/spec/rules")
                                .and_then(|v| v.as_array())
                                .map(|rules| {
                                    rules.iter().any(|r| {
                                        r.pointer("/http/paths")
                                            .and_then(|v| v.as_array())
                                            .map(|paths| {
                                                paths.iter().any(|p| {
                                                    p.pointer("/backend/service/name")
                                                        .and_then(|v| v.as_str())
                                                        == Some(name)
                                                })
                                            })
                                            .unwrap_or(false)
                                    })
                                })
                                .unwrap_or(false);

                            if points_to_svc {
                                let ing_name = ing.metadata.name.clone().unwrap_or_default();
                                let (i_st, i_dt) = extract_status_and_details("Ingress", &ing);
                                let mut i_node = LineageNode::new("Ingress", ing_name, Some(ns.to_string()), LineageRelation::Ingress);
                                i_node.status = i_st;
                                i_node.details = i_dt;
                                dependents.push(i_node);
                            }
                        }
                    }
                }
            }
        }
        "node" | "nodes" => {
            // Find Pods scheduled on this node
            if let Some((pod_api, _)) = dynamic_api(&client, "Pod", None) {
                let lp = ListParams::default().fields(&format!("spec.nodeName={}", name));
                if let Ok(Ok(pod_list)) = tokio::time::timeout(Duration::from_millis(1500), pod_api.list(&lp)).await {
                    for pod in pod_list.items {
                        let pod_name = pod.metadata.name.clone().unwrap_or_default();
                        let pod_ns = pod.metadata.namespace.clone();
                        let (p_st, p_dt) = extract_status_and_details("Pod", &pod);
                        let mut p_node = LineageNode::new("Pod", pod_name, pod_ns, LineageRelation::Child);
                        p_node.status = p_st;
                        p_node.details = p_dt;
                        dependents.push(p_node);
                    }
                }
            }
        }
        _ => {}
    }

    target_node.children = dependents;

    // If there are ancestors (ownerReferences), nest target_node inside its ancestors upwards
    if let Some(owners) = &obj.metadata.owner_references {
        if !owners.is_empty() {
            let root = nest_in_ancestors(&client, target_node, owners, effective_ns).await;
            return Ok(root);
        }
    }

    Ok(target_node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pod_spec_references() {
        let spec = serde_json::json!({
            "volumes": [
                { "name": "cfg", "configMap": { "name": "app-config" } },
                { "name": "auth", "secret": { "secretName": "app-secret" } },
                { "name": "data", "persistentVolumeClaim": { "claimName": "data-pvc" } },
            ],
            "containers": [
                {
                    "name": "web",
                    "image": "nginx:latest",
                    "envFrom": [
                        { "configMapRef": { "name": "env-config" } }
                    ]
                }
            ]
        });

        let refs = extract_pod_spec_references(&spec, Some("prod"));
        assert_eq!(refs.len(), 4);
        assert!(refs.iter().any(|r| r.kind == "ConfigMap" && r.name == "app-config"));
        assert!(refs.iter().any(|r| r.kind == "Secret" && r.name == "app-secret"));
        assert!(refs.iter().any(|r| r.kind == "PersistentVolumeClaim" && r.name == "data-pvc"));
        assert!(refs.iter().any(|r| r.kind == "ConfigMap" && r.name == "env-config"));
    }

    #[test]
    fn test_lineage_node_tree_nesting() {
        let mut root = LineageNode::new("Deployment", "web-app", Some("default".into()), LineageRelation::Owner);
        let mut rs = LineageNode::new("ReplicaSet", "web-app-7b8c9d", Some("default".into()), LineageRelation::Owner);
        let pod = LineageNode::new("Pod", "web-app-7b8c9d-x1", Some("default".into()), LineageRelation::Target);

        rs.children.push(pod);
        root.children.push(rs);

        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].children.len(), 1);
        assert_eq!(root.children[0].children[0].kind, "Pod");
    }

    #[test]
    fn test_owner_hierarchy_ordering() {
        // Verify that Deployment is the root ancestor that owns ReplicaSet, which owns Pod
        let pod = LineageNode::new("Pod", "connection-sink-5z2xs", Some("prod".into()), LineageRelation::Target);
        let mut rs = LineageNode::new("ReplicaSet", "connection-sink-7595b", Some("prod".into()), LineageRelation::Owner);
        let mut deploy = LineageNode::new("Deployment", "connection-sink", Some("prod".into()), LineageRelation::Owner);

        rs.children.push(pod);
        deploy.children.push(rs);

        assert_eq!(deploy.kind, "Deployment");
        assert_eq!(deploy.children[0].kind, "ReplicaSet");
        assert_eq!(deploy.children[0].children[0].kind, "Pod");
    }
}
