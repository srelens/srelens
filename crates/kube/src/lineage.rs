//! Kubernetes resource lineage & relationship resolver.
//! Traces owner hierarchies (upwards), dependents (downwards), and
//! linked networking, configuration, and storage (cross-links).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{Api, DynamicObject, ListParams};
use kube::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};
use tokio::sync::Semaphore;

use crate::client_cache::ClientCache;

/// Type of relationship in the lineage tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
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

/// The outcome of attempting to fetch a node from the cluster.
/// Upholds "Say what you know, not what you guess": distinguishes
/// not found, forbidden, timed out, and truncated outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum NodeOutcome {
    Healthy,
    NotFound,
    Forbidden,
    TimedOut,
    Truncated,
    Error(String),
}

impl NodeOutcome {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Healthy => "",
            Self::NotFound => "[NOT FOUND]",
            Self::Forbidden => "[FORBIDDEN]",
            Self::TimedOut => "[TIMED OUT]",
            Self::Truncated => "[TRUNCATED]",
            Self::Error(_) => "[ERROR]",
        }
    }
}

/// A node in the resource lineage hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LineageNode {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    pub relationship: LineageRelation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_outcome: Option<NodeOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
            api_version: None,
            uid: None,
            status: None,
            details: None,
            relationship,
            fetch_outcome: None,
            children: Vec::new(),
        }
    }

    pub fn with_api_version(mut self, api_version: impl Into<String>) -> Self {
        let v = api_version.into();
        if !v.is_empty() {
            self.api_version = Some(v);
        }
        self
    }

    pub fn with_uid(mut self, uid: impl Into<String>) -> Self {
        let u = uid.into();
        if !u.is_empty() {
            self.uid = Some(u);
        }
        self
    }

    pub fn with_status(mut self, status: Option<String>, details: Option<String>) -> Self {
        self.status = status;
        self.details = details;
        self
    }

    pub fn with_outcome(mut self, outcome: NodeOutcome) -> Self {
        self.fetch_outcome = Some(outcome);
        self
    }
}

/// Parse an `apiVersion` string into `(group, version)`.
/// Handles "group/version" (e.g. "kubevirt.io/v1") and bare "version" (e.g. "v1").
pub fn parse_api_version(api_version: &str) -> (Option<&str>, Option<&str>) {
    let trimmed = api_version.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    if let Some((group, version)) = trimmed.split_once('/') {
        (Some(group), Some(version))
    } else {
        (None, Some(trimmed))
    }
}

/// Helper to get a dynamic API for a resource kind using ClientCache resolution.
async fn resolve_dynamic_api(
    cache: &ClientCache,
    context: &str,
    client: &Client,
    kind: &str,
    group: Option<&str>,
    version: Option<&str>,
    namespace: Option<&str>,
) -> Result<(Api<DynamicObject>, bool), String> {
    let (ar, namespaced) = cache
        .resolve_resource(context, kind, group, version, None)
        .await?;

    let api = if namespaced && namespace.is_some() {
        Api::namespaced_with(client.clone(), namespace.unwrap(), &ar)
    } else {
        Api::all_with(client.clone(), &ar)
    };
    Ok((api, namespaced))
}

/// Extract status and details from a DynamicObject.
/// For built-ins (Pods, Deployments, ReplicaSets, etc.), uses specialized fields.
/// For generic CRDs, inspects `status.printableStatus` (e.g. KubeVirt), `status.phase`,
/// and standard `status.conditions` with polarity awareness.
pub fn extract_status_and_details(
    kind: &str,
    obj: &DynamicObject,
) -> (Option<String>, Option<String>) {
    let status_obj = obj.data.get("status");
    let spec_obj = obj.data.get("spec");

    // 1. High-fidelity extractors for well-known core/workload kinds
    match kind.to_lowercase().as_str() {
        "pod" | "pods" => {
            let phase = status_obj
                .and_then(|s| s.get("phase"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let mut container_details = Vec::new();
            if let Some(c_statuses) = status_obj
                .and_then(|s| s.get("containerStatuses"))
                .and_then(|v| v.as_array())
            {
                let ready_count = c_statuses
                    .iter()
                    .filter(|c| c.get("ready").and_then(|v| v.as_bool()).unwrap_or(false))
                    .count();
                let total = c_statuses.len();
                container_details.push(format!("Ready: {}/{}", ready_count, total));

                for c in c_statuses {
                    if let Some(waiting) =
                        c.pointer("/state/waiting/reason").and_then(|v| v.as_str())
                    {
                        container_details.push(waiting.to_string());
                        break;
                    }
                }
            }

            let node = spec_obj
                .and_then(|s| s.get("nodeName"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let details = if !node.is_empty() {
                Some(format!("node: {}, {}", node, container_details.join(", ")))
            } else if !container_details.is_empty() {
                Some(container_details.join(", "))
            } else {
                None
            };

            return (Some(phase), details);
        }
        "deployment" | "deployments" => {
            let replicas = status_obj
                .and_then(|s| s.get("replicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let ready = status_obj
                .and_then(|s| s.get("readyReplicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let updated = status_obj
                .and_then(|s| s.get("updatedReplicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            return (
                Some(format!("Ready: {}/{}", ready, replicas)),
                Some(format!("Up-to-date: {}", updated)),
            );
        }
        "replicaset" | "replicasets" => {
            let replicas = status_obj
                .and_then(|s| s.get("replicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let ready = status_obj
                .and_then(|s| s.get("readyReplicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            return (Some(format!("Ready: {}/{}", ready, replicas)), None);
        }
        "statefulset" | "statefulsets" => {
            let replicas = status_obj
                .and_then(|s| s.get("replicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let ready = status_obj
                .and_then(|s| s.get("readyReplicas"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            return (Some(format!("Ready: {}/{}", ready, replicas)), None);
        }
        "daemonset" | "daemonsets" => {
            let desired = status_obj
                .and_then(|s| s.get("desiredNumberScheduled"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let ready = status_obj
                .and_then(|s| s.get("numberReady"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            return (Some(format!("Ready: {}/{}", ready, desired)), None);
        }
        "service" | "services" => {
            let svc_type = spec_obj
                .and_then(|s| s.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("ClusterIP");
            let cluster_ip = spec_obj
                .and_then(|s| s.get("clusterIP"))
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            return (
                Some(svc_type.to_string()),
                Some(format!("IP: {}", cluster_ip)),
            );
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
            return (
                Some("Active".to_string()),
                if !hosts.is_empty() {
                    Some(hosts.join(", "))
                } else {
                    None
                },
            );
        }
        "persistentvolumeclaim" | "persistentvolumeclaims" => {
            let phase = status_obj
                .and_then(|s| s.get("phase"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let vol = spec_obj
                .and_then(|s| s.get("volumeName"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            return (
                Some(phase.to_string()),
                if !vol.is_empty() {
                    Some(format!("PV: {}", vol))
                } else {
                    None
                },
            );
        }
        "job" | "jobs" => {
            let succeeded = status_obj
                .and_then(|s| s.get("succeeded"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let active = status_obj
                .and_then(|s| s.get("active"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let failed = status_obj
                .and_then(|s| s.get("failed"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            return (
                Some(
                    if succeeded > 0 {
                        "Completed"
                    } else if active > 0 {
                        "Running"
                    } else if failed > 0 {
                        "Failed"
                    } else {
                        "Pending"
                    }
                    .to_string(),
                ),
                Some(format!(
                    "Active: {}, Succeeded: {}, Failed: {}",
                    active, succeeded, failed
                )),
            );
        }
        _ => {}
    }

    // 2. Generic CRD / Custom Resource extraction:
    let printable = status_obj
        .and_then(|s| s.get("printableStatus"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let phase = status_obj
        .and_then(|s| s.get("phase"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut cond_status = None;
    let mut cond_details = None;

    if let Some(conditions) = status_obj
        .and_then(|s| s.get("conditions"))
        .and_then(|v| v.as_array())
    {
        // Abnormal-true conditions: if any is True, the resource is degraded/unhealthy
        const ABNORMAL_CONDITIONS: &[&str] = &[
            "Degraded",
            "Stalled",
            "Failed",
            "ReplicaFailure",
            "DisruptionTarget",
            "Error",
        ];

        for cond in conditions {
            let c_type = cond.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let c_status = cond.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let reason = cond.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let msg = cond.get("message").and_then(|v| v.as_str()).unwrap_or("");

            if ABNORMAL_CONDITIONS
                .iter()
                .any(|&ac| ac.eq_ignore_ascii_case(c_type))
                && c_status.eq_ignore_ascii_case("True")
            {
                cond_status = Some(c_type.to_string());
                let detail = if !reason.is_empty() {
                    reason.to_string()
                } else if !msg.is_empty() {
                    msg.to_string()
                } else {
                    format!("{c_type}=True")
                };
                cond_details = Some(detail);
                break;
            }
        }

        // If no abnormal conditions found, check normal-true conditions
        if cond_status.is_none() {
            const NORMAL_CONDITIONS: &[&str] =
                &["Ready", "Available", "Synced", "Healthy", "Progressing"];
            for &nc in NORMAL_CONDITIONS {
                if let Some(cond) = conditions.iter().find(|c| {
                    c.get("type")
                        .and_then(|v| v.as_str())
                        .map_or(false, |t| t.eq_ignore_ascii_case(nc))
                }) {
                    let c_status = cond.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let reason = cond.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                    let msg = cond.get("message").and_then(|v| v.as_str()).unwrap_or("");

                    if c_status.eq_ignore_ascii_case("True") {
                        cond_status = Some(nc.to_string());
                        if !reason.is_empty() {
                            cond_details = Some(reason.to_string());
                        }
                    } else if c_status.eq_ignore_ascii_case("False") {
                        cond_status = Some(format!("Not {}", nc));
                        let detail = if !reason.is_empty() {
                            reason.to_string()
                        } else {
                            msg.to_string()
                        };
                        if !detail.is_empty() {
                            cond_details = Some(detail);
                        }
                    }
                    break;
                }
            }
        }
    }

    let final_status = cond_status.or(printable).or(phase);

    let replicas_summary = status_obj.and_then(|s| {
        let ready = s
            .get("readyReplicas")
            .or_else(|| s.get("ready"))
            .and_then(|v| v.as_i64());
        let total = s
            .get("replicas")
            .or_else(|| s.get("desired"))
            .and_then(|v| v.as_i64());
        match (ready, total) {
            (Some(r), Some(t)) => Some(format!("Ready: {}/{}", r, t)),
            (None, Some(t)) => Some(format!("Replicas: {}", t)),
            _ => None,
        }
    });

    let final_details = match (cond_details, replicas_summary) {
        (Some(cd), Some(rs)) => Some(format!("{cd}, {rs}")),
        (Some(cd), None) => Some(cd),
        (None, Some(rs)) => Some(rs),
        (None, None) => None,
    };

    (final_status, final_details)
}

/// Matches a child's ownerReference against target identification.
/// Strict UID matching is required when target_uid is present; fallback to
/// (kind, apiVersion, name) is allowed ONLY when target_uid is genuinely empty.
pub fn is_child_of(
    owner: &OwnerReference,
    target_uid: &str,
    target_kind: &str,
    target_api_version: Option<&str>,
    target_name: &str,
) -> bool {
    if !target_uid.is_empty() {
        return owner.uid == target_uid;
    }
    if owner.kind.eq_ignore_ascii_case(target_kind) && owner.name == target_name {
        if let Some(target_api) = target_api_version {
            return owner.api_version == target_api;
        }
        return true;
    }
    false
}

/// Recursively resolve owner hierarchy upwards, nesting the child node inside its parent,
/// and returning the highest ancestor (root) of the hierarchy.
async fn nest_in_ancestors(
    cache: &ClientCache,
    context: &str,
    client: &Client,
    child: LineageNode,
    owners: &[OwnerReference],
    namespace: Option<&str>,
    visited_uids: &mut HashSet<String>,
    depth: usize,
) -> LineageNode {
    if owners.is_empty() || depth >= 12 {
        return child;
    }

    // Prefer controller owner if present, otherwise first owner
    let primary_owner = owners
        .iter()
        .find(|o| o.controller == Some(true))
        .unwrap_or(&owners[0]);

    if !primary_owner.uid.is_empty() && !visited_uids.insert(primary_owner.uid.clone()) {
        // Cycle detected
        return child;
    }

    let mut parent_node = LineageNode::new(
        &primary_owner.kind,
        &primary_owner.name,
        namespace.map(|s| s.to_string()),
        LineageRelation::Owner,
    )
    .with_api_version(&primary_owner.api_version)
    .with_uid(&primary_owner.uid);

    let (group, version) = parse_api_version(&primary_owner.api_version);
    let mut grandparents = None;

    match resolve_dynamic_api(
        cache,
        context,
        client,
        &primary_owner.kind,
        group,
        version,
        namespace,
    )
    .await
    {
        Ok((api, namespaced)) => {
            if !namespaced {
                parent_node.namespace = None;
            }
            match tokio::time::timeout(Duration::from_millis(1500), api.get(&primary_owner.name))
                .await
            {
                Ok(Ok(parent_obj)) => {
                    let (status, details) =
                        extract_status_and_details(&primary_owner.kind, &parent_obj);
                    parent_node.status = status;
                    parent_node.details = details;
                    parent_node.fetch_outcome = Some(NodeOutcome::Healthy);
                    grandparents = parent_obj.metadata.owner_references;
                }
                Ok(Err(kube::Error::Api(ref ae))) if ae.code == 404 => {
                    parent_node.fetch_outcome = Some(NodeOutcome::NotFound);
                }
                Ok(Err(kube::Error::Api(ref ae))) if ae.code == 403 => {
                    parent_node.fetch_outcome = Some(NodeOutcome::Forbidden);
                }
                Ok(Err(e)) => {
                    parent_node.fetch_outcome = Some(NodeOutcome::Error(e.to_string()));
                }
                Err(_) => {
                    parent_node.fetch_outcome = Some(NodeOutcome::TimedOut);
                }
            }
        }
        Err(e) => {
            parent_node.fetch_outcome = Some(NodeOutcome::Error(e));
        }
    }

    parent_node.children.push(child);

    // If there were other non-primary owners on the child, attach them as sibling nodes to parent_node
    for owner in owners {
        if !owner.uid.is_empty() && owner.uid == primary_owner.uid {
            continue;
        }
        if owner.uid.is_empty() && owner.name == primary_owner.name {
            continue;
        }

        let mut other_node = LineageNode::new(
            &owner.kind,
            &owner.name,
            namespace.map(|s| s.to_string()),
            LineageRelation::Owner,
        )
        .with_api_version(&owner.api_version)
        .with_uid(&owner.uid);

        let (o_group, o_version) = parse_api_version(&owner.api_version);
        match resolve_dynamic_api(
            cache,
            context,
            client,
            &owner.kind,
            o_group,
            o_version,
            namespace,
        )
        .await
        {
            Ok((api, namespaced)) => {
                if !namespaced {
                    other_node.namespace = None;
                }
                match tokio::time::timeout(Duration::from_millis(1500), api.get(&owner.name)).await
                {
                    Ok(Ok(obj)) => {
                        let (status, details) = extract_status_and_details(&owner.kind, &obj);
                        other_node.status = status;
                        other_node.details = details;
                        other_node.fetch_outcome = Some(NodeOutcome::Healthy);
                    }
                    Ok(Err(kube::Error::Api(ref ae))) if ae.code == 404 => {
                        other_node.fetch_outcome = Some(NodeOutcome::NotFound);
                    }
                    Ok(Err(kube::Error::Api(ref ae))) if ae.code == 403 => {
                        other_node.fetch_outcome = Some(NodeOutcome::Forbidden);
                    }
                    Ok(Err(e)) => {
                        other_node.fetch_outcome = Some(NodeOutcome::Error(e.to_string()));
                    }
                    Err(_) => {
                        other_node.fetch_outcome = Some(NodeOutcome::TimedOut);
                    }
                }
            }
            Err(e) => {
                other_node.fetch_outcome = Some(NodeOutcome::Error(e));
            }
        }

        parent_node.children.push(other_node);
    }

    // Continue upwards if this parent also has ownerReferences
    if let Some(gps) = grandparents {
        if !gps.is_empty() {
            return Box::pin(nest_in_ancestors(
                cache,
                context,
                client,
                parent_node,
                &gps,
                namespace,
                visited_uids,
                depth + 1,
            ))
            .await;
        }
    }

    parent_node
}

/// Extract config and storage references from a pod spec.
fn extract_pod_spec_references(
    spec: &serde_json::Value,
    namespace: Option<&str>,
) -> Vec<LineageNode> {
    let mut references = Vec::new();

    if let Some(volumes) = spec.get("volumes").and_then(|v| v.as_array()) {
        for vol in volumes {
            if let Some(cm) = vol
                .get("configMap")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str())
            {
                let mut n = LineageNode::new(
                    "ConfigMap",
                    cm,
                    namespace.map(|s| s.to_string()),
                    LineageRelation::Config,
                );
                n.details = Some(format!(
                    "Volume: {}",
                    vol.get("name").and_then(|v| v.as_str()).unwrap_or("-")
                ));
                n.fetch_outcome = Some(NodeOutcome::Healthy);
                references.push(n);
            }
            if let Some(sec) = vol
                .get("secret")
                .and_then(|v| v.get("secretName"))
                .and_then(|v| v.as_str())
            {
                let mut n = LineageNode::new(
                    "Secret",
                    sec,
                    namespace.map(|s| s.to_string()),
                    LineageRelation::Secret,
                );
                n.details = Some(format!(
                    "Volume: {}",
                    vol.get("name").and_then(|v| v.as_str()).unwrap_or("-")
                ));
                n.fetch_outcome = Some(NodeOutcome::Healthy);
                references.push(n);
            }
            if let Some(pvc) = vol
                .get("persistentVolumeClaim")
                .and_then(|v| v.get("claimName"))
                .and_then(|v| v.as_str())
            {
                let mut n = LineageNode::new(
                    "PersistentVolumeClaim",
                    pvc,
                    namespace.map(|s| s.to_string()),
                    LineageRelation::Storage,
                );
                n.details = Some(format!(
                    "Volume: {}",
                    vol.get("name").and_then(|v| v.as_str()).unwrap_or("-")
                ));
                n.fetch_outcome = Some(NodeOutcome::Healthy);
                references.push(n);
            }
        }
    }

    // Containers envFrom
    if let Some(containers) = spec.get("containers").and_then(|v| v.as_array()) {
        for c in containers {
            if let Some(env_from) = c.get("envFrom").and_then(|v| v.as_array()) {
                for ef in env_from {
                    if let Some(cm) = ef
                        .get("configMapRef")
                        .and_then(|v| v.get("name"))
                        .and_then(|v| v.as_str())
                    {
                        let mut n = LineageNode::new(
                            "ConfigMap",
                            cm,
                            namespace.map(|s| s.to_string()),
                            LineageRelation::Config,
                        );
                        n.details = Some("envFrom".to_string());
                        n.fetch_outcome = Some(NodeOutcome::Healthy);
                        if !references
                            .iter()
                            .any(|r| r.kind == "ConfigMap" && r.name == cm)
                        {
                            references.push(n);
                        }
                    }
                    if let Some(sec) = ef
                        .get("secretRef")
                        .and_then(|v| v.get("name"))
                        .and_then(|v| v.as_str())
                    {
                        let mut n = LineageNode::new(
                            "Secret",
                            sec,
                            namespace.map(|s| s.to_string()),
                            LineageRelation::Secret,
                        );
                        n.details = Some("envFrom".to_string());
                        n.fetch_outcome = Some(NodeOutcome::Healthy);
                        if !references
                            .iter()
                            .any(|r| r.kind == "Secret" && r.name == sec)
                        {
                            references.push(n);
                        }
                    }
                }
            }
        }
    }

    references
}

/// Attach container nodes and volume references to a Pod lineage node.
fn attach_pod_internals(node: &mut LineageNode, obj: &DynamicObject, namespace: Option<&str>) {
    if let Some(containers) = obj
        .data
        .pointer("/spec/containers")
        .and_then(|v| v.as_array())
    {
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
                        } else if let Some(waiting) =
                            c.pointer("/state/waiting/reason").and_then(|v| v.as_str())
                        {
                            waiting.to_string()
                        } else if let Some(term) = c
                            .pointer("/state/terminated/reason")
                            .and_then(|v| v.as_str())
                        {
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
                let (st, restarts) = statuses_map
                    .get(c_name)
                    .cloned()
                    .unwrap_or(("Waiting".to_string(), 0));
                let mut c_node = LineageNode::new(
                    "Container",
                    c_name,
                    namespace.map(|s| s.to_string()),
                    LineageRelation::Child,
                );
                c_node.status = Some(st);
                c_node.details = Some(format!("image: {}, restarts: {}", img, restarts));
                c_node.fetch_outcome = Some(NodeOutcome::Healthy);
                node.children.push(c_node);
            }
        }
    }

    if let Some(spec) = obj.data.get("spec") {
        let refs = extract_pod_spec_references(spec, namespace);
        node.children.extend(refs);
    }
}

/// Collect candidate child resource kinds to scan in the namespace.
fn collect_candidate_resources(
    disc: Option<&crate::client_cache::DiscoveryIndex>,
    target_kind: &str,
    target_group: Option<&str>,
) -> Vec<(String, Option<String>, Option<String>)> {
    let mut candidates = Vec::new();
    let target_kind_lower = target_kind.to_lowercase();

    // 1. Common workload types that are frequently children
    candidates.push(("Pod".to_string(), None, None));
    candidates.push(("ReplicaSet".to_string(), Some("apps".to_string()), None));
    candidates.push(("Job".to_string(), Some("batch".to_string()), None));
    candidates.push(("PersistentVolumeClaim".to_string(), None, None));

    // 2. High-frequency controller mappings
    match target_kind_lower.as_str() {
        "virtualmachine" => {
            candidates.push((
                "VirtualMachineInstance".to_string(),
                Some("kubevirt.io".to_string()),
                None,
            ));
        }
        "deployment" => {
            candidates.push(("ReplicaSet".to_string(), Some("apps".to_string()), None));
        }
        "cronjob" => {
            candidates.push(("Job".to_string(), Some("batch".to_string()), None));
        }
        "certificate" => {
            candidates.push((
                "CertificateRequest".to_string(),
                Some("cert-manager.io".to_string()),
                None,
            ));
        }
        "certificaterequest" => {
            candidates.push((
                "Order".to_string(),
                Some("acme.cert-manager.io".to_string()),
                None,
            ));
        }
        "order" => {
            candidates.push((
                "Challenge".to_string(),
                Some("acme.cert-manager.io".to_string()),
                None,
            ));
        }
        "application" => {
            candidates.push(("Deployment".to_string(), Some("apps".to_string()), None));
            candidates.push(("StatefulSet".to_string(), Some("apps".to_string()), None));
            candidates.push(("Service".to_string(), None, None));
        }
        _ => {}
    }

    // 3. Same API group CRDs from cluster discovery (including cluster-scoped candidates)
    if let Some(disc_idx) = disc {
        if let Some(grp) = target_group.filter(|g| !g.is_empty()) {
            let grp_lower = grp.to_lowercase();
            for res in disc_idx.all_resources() {
                if res.ar.group.to_lowercase() == grp_lower {
                    candidates.push((res.ar.kind, Some(res.ar.group), Some(res.ar.version)));
                }
            }
        }

        // Add other discovered CRDs up to a bounded limit (including cluster-scoped)
        let mut crd_count = 0;
        for res in disc_idx.all_resources() {
            if !res.ar.group.is_empty() && !res.ar.group.contains("k8s.io") {
                if crd_count >= 15 {
                    break;
                }
                candidates.push((res.ar.kind, Some(res.ar.group), Some(res.ar.version)));
                crd_count += 1;
            }
        }
    }

    // Deduplicate candidates (preserving same-kind CRDs so nested hierarchies are discovered)
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for (k, g, v) in candidates {
        let key = (
            g.clone().unwrap_or_default().to_lowercase(),
            k.to_lowercase(),
        );
        if k.eq_ignore_ascii_case("secret") {
            continue;
        }
        if seen.insert(key) {
            deduped.push((k, g, v));
        }
    }

    deduped
}

enum ScanResult {
    Success {
        cand_kind: String,
        items: Vec<DynamicObject>,
        truncated: bool,
    },
    Skipped,
    Failed {
        cand_kind: String,
        outcome: NodeOutcome,
    },
}

/// Recursively expand children from the in-memory owner index.
fn expand_dependents_from_index(
    parent_uid: &str,
    parent_kind: &str,
    parent_name: &str,
    parent_api_version: Option<&str>,
    effective_ns: Option<&str>,
    owner_index: &HashMap<String, Vec<(String, DynamicObject)>>,
    owner_name_index: &HashMap<(String, String), Vec<(String, DynamicObject)>>,
    truncated_kinds: &HashSet<String>,
    visited_uids: &mut HashSet<String>,
    depth: usize,
) -> Vec<LineageNode> {
    if depth >= 4 {
        return Vec::new();
    }

    let candidate_items = if !parent_uid.is_empty() {
        owner_index.get(parent_uid).cloned().unwrap_or_default()
    } else {
        owner_name_index
            .get(&(parent_kind.to_lowercase(), parent_name.to_string()))
            .cloned()
            .unwrap_or_default()
    };

    let mut children = Vec::new();
    let mut seen_in_this_parent = HashSet::new();

    for (item_kind, item) in candidate_items {
        let is_match = item
            .metadata
            .owner_references
            .as_ref()
            .map_or(false, |owners| {
                owners.iter().any(|o| {
                    is_child_of(o, parent_uid, parent_kind, parent_api_version, parent_name)
                })
            });

        if !is_match {
            continue;
        }

        let item_name = item.metadata.name.clone().unwrap_or_default();
        let item_uid = item.metadata.uid.clone().unwrap_or_default();
        let item_ns = item
            .metadata
            .namespace
            .clone()
            .or_else(|| effective_ns.map(|s| s.to_string()));
        let item_api = item.types.as_ref().map(|t| t.api_version.clone());

        let dedup_key = if !item_uid.is_empty() {
            item_uid.clone()
        } else {
            format!(
                "{}/{}/{}",
                item_kind,
                item_ns.as_deref().unwrap_or(""),
                item_name
            )
        };

        if !seen_in_this_parent.insert(dedup_key) {
            continue;
        }

        if !item_uid.is_empty() && !visited_uids.insert(item_uid.clone()) {
            continue;
        }

        let (status, details) = extract_status_and_details(&item_kind, &item);

        let mut child_node = LineageNode::new(
            &item_kind,
            &item_name,
            item_ns.clone(),
            LineageRelation::Child,
        )
        .with_api_version(item_api.as_deref().unwrap_or(""))
        .with_uid(&item_uid)
        .with_status(status, details)
        .with_outcome(NodeOutcome::Healthy);

        if item_kind.eq_ignore_ascii_case("pod") {
            attach_pod_internals(&mut child_node, &item, item_ns.as_deref());
        }

        if depth < 3 && (!item_uid.is_empty() || !item_name.is_empty()) {
            let grand_children = expand_dependents_from_index(
                &item_uid,
                &item_kind,
                &item_name,
                item_api.as_deref(),
                item_ns.as_deref(),
                owner_index,
                owner_name_index,
                truncated_kinds,
                visited_uids,
                depth + 1,
            );
            child_node.children.extend(grand_children);
        }

        children.push(child_node);
    }

    children
}

/// Discovers child resources across the namespace/cluster in a single pass using ownerReferences.
async fn discover_dependents(
    cache: Arc<ClientCache>,
    context: &str,
    client: &Client,
    target_uid: &str,
    target_kind: &str,
    target_name: &str,
    target_api_version: Option<&str>,
    target_group: Option<&str>,
    effective_ns: Option<&str>,
    visited_uids: &mut HashSet<String>,
) -> Vec<LineageNode> {
    let disc = cache.get_discovery(context).await.ok();
    let candidates = collect_candidate_resources(disc.as_deref(), target_kind, target_group);

    let mut scanned_keys: HashSet<(String, Option<String>)> = HashSet::new();
    let mut candidates_to_scan = candidates;
    let mut owner_index: HashMap<String, Vec<(String, DynamicObject)>> = HashMap::new();
    let mut owner_name_index: HashMap<(String, String), Vec<(String, DynamicObject)>> =
        HashMap::new();
    let mut truncated_kinds: HashSet<String> = HashSet::new();
    let mut failed_scans: Vec<(String, NodeOutcome)> = Vec::new();

    let sem = Arc::new(Semaphore::new(6));

    // Iteratively expand candidate collections when newly reached child types emerge
    for _round in 0..3 {
        let mut batch = Vec::new();
        for (k, g, v) in candidates_to_scan {
            let key = (k.to_lowercase(), g.clone().map(|s| s.to_lowercase()));
            if scanned_keys.insert(key) {
                batch.push((k, g, v));
            }
        }

        if batch.is_empty() {
            break;
        }

        let mut join_set = tokio::task::JoinSet::new();

        for (cand_kind, cand_group, cand_version) in batch {
            let sem = sem.clone();
            let cache = cache.clone();
            let ctx = context.to_string();
            let client = client.clone();
            let ns = effective_ns.map(|s| s.to_string());

            join_set.spawn(async move {
                let _permit = match sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return ScanResult::Skipped,
                };

                let (api, namespaced) = match resolve_dynamic_api(
                    &cache,
                    &ctx,
                    &client,
                    &cand_kind,
                    cand_group.as_deref(),
                    cand_version.as_deref(),
                    ns.as_deref(),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => return ScanResult::Skipped,
                };

                if !namespaced && ns.is_some() {
                    return ScanResult::Skipped;
                }

                let lp = ListParams::default();
                match crate::list_cap::list_capped_within(&api, lp, Duration::from_millis(1500))
                    .await
                {
                    Ok((items, truncated)) => ScanResult::Success {
                        cand_kind,
                        items,
                        truncated,
                    },
                    Err(crate::list_cap::ListCappedError::Api(kube::Error::Api(ref ae)))
                        if ae.code == 403 =>
                    {
                        ScanResult::Failed {
                            cand_kind,
                            outcome: NodeOutcome::Forbidden,
                        }
                    }
                    Err(crate::list_cap::ListCappedError::Timeout { .. }) => ScanResult::Failed {
                        cand_kind,
                        outcome: NodeOutcome::TimedOut,
                    },
                    Err(e) => ScanResult::Failed {
                        cand_kind,
                        outcome: NodeOutcome::Error(e.to_string()),
                    },
                }
            });
        }

        let mut new_child_types: Vec<(String, Option<String>)> = Vec::new();

        while let Some(res) = join_set.join_next().await {
            match res {
                Ok(ScanResult::Success {
                    cand_kind,
                    items,
                    truncated,
                }) => {
                    if truncated {
                        truncated_kinds.insert(cand_kind.clone());
                    }
                    for item in items {
                        let item_api = item.types.as_ref().map(|t| t.api_version.as_str());
                        let (child_grp, _) = parse_api_version(item_api.unwrap_or(""));
                        new_child_types.push((cand_kind.clone(), child_grp.map(|s| s.to_string())));

                        if let Some(owners) = &item.metadata.owner_references {
                            for owner in owners {
                                if !owner.uid.is_empty() {
                                    owner_index
                                        .entry(owner.uid.clone())
                                        .or_default()
                                        .push((cand_kind.clone(), item.clone()));
                                }
                                if !owner.name.is_empty() {
                                    owner_name_index
                                        .entry((owner.kind.to_lowercase(), owner.name.clone()))
                                        .or_default()
                                        .push((cand_kind.clone(), item.clone()));
                                }
                            }
                        }
                    }
                }
                Ok(ScanResult::Failed { cand_kind, outcome }) => {
                    failed_scans.push((cand_kind, outcome));
                }
                _ => {}
            }
        }

        // Dynamically extend candidate set for newly discovered child kinds
        let mut next_candidates = Vec::new();
        for (c_kind, c_grp) in new_child_types {
            let further = collect_candidate_resources(disc.as_deref(), &c_kind, c_grp.as_deref());
            next_candidates.extend(further);
        }
        candidates_to_scan = next_candidates;
    }

    let mut generic_children = expand_dependents_from_index(
        target_uid,
        target_kind,
        target_name,
        target_api_version,
        effective_ns,
        &owner_index,
        &owner_name_index,
        &truncated_kinds,
        visited_uids,
        0,
    );

    // If any potential child kind failed with Forbidden/TimedOut/Error, report it truthfully
    for (failed_kind, outcome) in failed_scans {
        let is_expected_child = match target_kind.to_lowercase().as_str() {
            "deployment" => failed_kind.eq_ignore_ascii_case("replicaset"),
            "cronjob" => failed_kind.eq_ignore_ascii_case("job"),
            "virtualmachine" => failed_kind.eq_ignore_ascii_case("virtualmachineinstance"),
            "certificate" => failed_kind.eq_ignore_ascii_case("certificaterequest"),
            _ => false,
        };
        if is_expected_child {
            let placeholder_name = match &outcome {
                NodeOutcome::Forbidden => "<unauthorized>".to_string(),
                NodeOutcome::TimedOut => "<timed out>".to_string(),
                NodeOutcome::Error(msg) => format!("<error: {msg}>"),
                _ => "<failed>".to_string(),
            };
            let mut fail_node = LineageNode::new(
                &failed_kind,
                placeholder_name,
                effective_ns.map(|s| s.to_string()),
                LineageRelation::Child,
            );
            fail_node.fetch_outcome = Some(outcome);
            generic_children.push(fail_node);
        }
    }

    // If any kind with discovered children was cut off at the cap, report truncation
    for truncated_kind in &truncated_kinds {
        let has_child = generic_children
            .iter()
            .any(|c| c.kind.eq_ignore_ascii_case(truncated_kind));
        if has_child {
            let mut trunc_node = LineageNode::new(
                truncated_kind,
                "<more items truncated at 2,000 ceiling>",
                effective_ns.map(|s| s.to_string()),
                LineageRelation::Child,
            );
            trunc_node.fetch_outcome = Some(NodeOutcome::Truncated);
            generic_children.push(trunc_node);
        }
    }

    generic_children
}

/// Resolve the full relationship lineage tree for any Kubernetes resource (built-in or CRD).
pub async fn resolve_resource_lineage(
    cache: Arc<ClientCache>,
    context: &str,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
    api_version: Option<&str>,
    group: Option<&str>,
    plural: Option<&str>,
) -> Result<LineageNode, String> {
    let client = cache.get(context).await?;
    let (parsed_group, parsed_version) = parse_api_version(api_version.unwrap_or(""));
    let effective_group = group.or(parsed_group);
    let effective_version = parsed_version;

    let (ar, namespaced) = cache
        .resolve_resource(context, kind, effective_group, effective_version, plural)
        .await
        .map_err(|e| format!("Could not resolve resource {kind}: {e}"))?;

    let api: Api<DynamicObject> = if namespaced && namespace.is_some() {
        Api::namespaced_with(client.clone(), namespace.unwrap(), &ar)
    } else {
        Api::all_with(client.clone(), &ar)
    };

    let get_res = tokio::time::timeout(Duration::from_secs(3), api.get(name)).await;
    let obj: DynamicObject = match get_res {
        Ok(Ok(o)) => o,
        Ok(Err(kube::Error::Api(ref ae))) if ae.code == 404 => {
            return Err(format!("Resource '{kind}/{name}' not found"));
        }
        Ok(Err(kube::Error::Api(ref ae))) if ae.code == 403 => {
            return Err(format!("Access forbidden to '{kind}/{name}' (RBAC error)"));
        }
        Ok(Err(e)) => {
            return Err(format!("Failed to get '{kind}/{name}': {e}"));
        }
        Err(_) => {
            return Err(format!("Timed out fetching '{kind}/{name}' after 3s"));
        }
    };

    let effective_ns = if namespaced {
        obj.metadata.namespace.as_deref().or(namespace)
    } else {
        None
    };

    let target_uid = obj.metadata.uid.clone().unwrap_or_default();
    let target_labels = obj.metadata.labels.clone().unwrap_or_default();
    let actual_api_ver = obj
        .types
        .as_ref()
        .map(|t| t.api_version.clone())
        .or_else(|| {
            if !ar.api_version.is_empty() {
                Some(ar.api_version.clone())
            } else {
                api_version.map(|s| s.to_string())
            }
        });

    let resolved_group = if !ar.group.is_empty() {
        Some(ar.group.as_str())
    } else {
        effective_group
    };

    let mut target_node = LineageNode::new(
        kind,
        name,
        effective_ns.map(|s| s.to_string()),
        LineageRelation::Target,
    )
    .with_api_version(actual_api_ver.as_deref().unwrap_or(""))
    .with_uid(&target_uid)
    .with_outcome(NodeOutcome::Healthy);

    let (status, details) = extract_status_and_details(kind, &obj);
    target_node.status = status;
    target_node.details = details;

    let mut visited_uids = HashSet::new();
    if !target_uid.is_empty() {
        visited_uids.insert(target_uid.clone());
    }

    // 1. Resolve Children / Dependents (Downwards)
    let mut dependents = Vec::new();

    // Workload-specific container & cross-link inspections
    let kind_lower = kind.to_lowercase();
    match kind_lower.as_str() {
        "pod" | "pods" => {
            attach_pod_internals(&mut target_node, &obj, effective_ns);

            // Linked Services matching this pod
            if let Some(ns) = effective_ns {
                if let Ok((svc_api, _)) =
                    resolve_dynamic_api(&cache, context, &client, "Service", None, None, Some(ns))
                        .await
                {
                    if let Ok(Ok(svc_list)) = tokio::time::timeout(
                        Duration::from_millis(1500),
                        svc_api.list(&ListParams::default()),
                    )
                    .await
                    {
                        for svc in svc_list.items {
                            if let Some(selector) = svc
                                .data
                                .pointer("/spec/selector")
                                .and_then(|v| v.as_object())
                            {
                                if !selector.is_empty()
                                    && selector.iter().all(|(k, v)| {
                                        target_labels.get(k).map(|lv| lv.as_str()) == v.as_str()
                                    })
                                {
                                    let svc_name = svc.metadata.name.clone().unwrap_or_default();
                                    let (s_st, s_dt) = extract_status_and_details("Service", &svc);
                                    let mut s_node = LineageNode::new(
                                        "Service",
                                        svc_name,
                                        Some(ns.to_string()),
                                        LineageRelation::Service,
                                    );
                                    s_node.status = s_st;
                                    s_node.details = s_dt;
                                    s_node.fetch_outcome = Some(NodeOutcome::Healthy);
                                    dependents.push(s_node);
                                }
                            }
                        }
                    }
                }
            }
        }
        "deployment" | "deployments" | "statefulset" | "statefulsets" | "daemonset"
        | "daemonsets" => {
            if let Some(ns) = effective_ns {
                // Linked Services matching workload's template labels
                if let Some(tmpl_labels) = obj
                    .data
                    .pointer("/spec/template/metadata/labels")
                    .and_then(|v| v.as_object())
                {
                    if let Ok((svc_api, _)) = resolve_dynamic_api(
                        &cache,
                        context,
                        &client,
                        "Service",
                        None,
                        None,
                        Some(ns),
                    )
                    .await
                    {
                        if let Ok(Ok(svc_list)) = tokio::time::timeout(
                            Duration::from_millis(1500),
                            svc_api.list(&ListParams::default()),
                        )
                        .await
                        {
                            for svc in svc_list.items {
                                if let Some(selector) = svc
                                    .data
                                    .pointer("/spec/selector")
                                    .and_then(|v| v.as_object())
                                {
                                    if !selector.is_empty()
                                        && selector.iter().all(|(k, v)| {
                                            tmpl_labels.get(k).and_then(|lv| lv.as_str())
                                                == v.as_str()
                                        })
                                    {
                                        let svc_name =
                                            svc.metadata.name.clone().unwrap_or_default();
                                        let (s_st, s_dt) =
                                            extract_status_and_details("Service", &svc);
                                        let mut s_node = LineageNode::new(
                                            "Service",
                                            svc_name,
                                            Some(ns.to_string()),
                                            LineageRelation::Service,
                                        );
                                        s_node.status = s_st;
                                        s_node.details = s_dt;
                                        s_node.fetch_outcome = Some(NodeOutcome::Healthy);
                                        dependents.push(s_node);
                                    }
                                }
                            }
                        }
                    }
                }

                // Referenced ConfigMaps, Secrets, and PVCs in pod template
                if let Some(spec) = obj.data.pointer("/spec/template/spec") {
                    let refs = extract_pod_spec_references(spec, effective_ns);
                    dependents.extend(refs);
                }
            }
        }
        "service" | "services" => {
            if let Some(ns) = effective_ns {
                // Find Pods matching selector
                if let Some(selector) = obj
                    .data
                    .pointer("/spec/selector")
                    .and_then(|v| v.as_object())
                {
                    if !selector.is_empty() {
                        if let Ok((pod_api, _)) = resolve_dynamic_api(
                            &cache,
                            context,
                            &client,
                            "Pod",
                            None,
                            None,
                            Some(ns),
                        )
                        .await
                        {
                            if let Ok(Ok(pod_list)) = tokio::time::timeout(
                                Duration::from_millis(1500),
                                pod_api.list(&ListParams::default()),
                            )
                            .await
                            {
                                for pod in pod_list.items {
                                    let pod_labels = pod.metadata.labels.as_ref();
                                    let matches = selector.iter().all(|(k, v)| {
                                        pod_labels.and_then(|pl| pl.get(k)).map(|lv| lv.as_str())
                                            == v.as_str()
                                    });
                                    if matches {
                                        let pod_name =
                                            pod.metadata.name.clone().unwrap_or_default();
                                        let (p_st, p_dt) = extract_status_and_details("Pod", &pod);
                                        let mut p_node = LineageNode::new(
                                            "Pod",
                                            pod_name,
                                            Some(ns.to_string()),
                                            LineageRelation::Child,
                                        );
                                        p_node.status = p_st;
                                        p_node.details = p_dt;
                                        p_node.fetch_outcome = Some(NodeOutcome::Healthy);
                                        dependents.push(p_node);
                                    }
                                }
                            }
                        }
                    }
                }

                // Find Ingresses pointing to this Service
                if let Ok((ing_api, _)) = resolve_dynamic_api(
                    &cache,
                    context,
                    &client,
                    "Ingress",
                    Some("networking.k8s.io"),
                    None,
                    Some(ns),
                )
                .await
                {
                    if let Ok(Ok(ing_list)) = tokio::time::timeout(
                        Duration::from_millis(1500),
                        ing_api.list(&ListParams::default()),
                    )
                    .await
                    {
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
                                let mut i_node = LineageNode::new(
                                    "Ingress",
                                    ing_name,
                                    Some(ns.to_string()),
                                    LineageRelation::Ingress,
                                );
                                i_node.status = i_st;
                                i_node.details = i_dt;
                                i_node.fetch_outcome = Some(NodeOutcome::Healthy);
                                dependents.push(i_node);
                            }
                        }
                    }
                }
            }
        }
        "node" | "nodes" => {
            if let Ok((pod_api, _)) =
                resolve_dynamic_api(&cache, context, &client, "Pod", None, None, None).await
            {
                let lp = ListParams::default().fields(&format!("spec.nodeName={}", name));
                if let Ok(Ok(pod_list)) =
                    tokio::time::timeout(Duration::from_millis(1500), pod_api.list(&lp)).await
                {
                    for pod in pod_list.items {
                        let pod_name = pod.metadata.name.clone().unwrap_or_default();
                        let pod_ns = pod.metadata.namespace.clone();
                        let (p_st, p_dt) = extract_status_and_details("Pod", &pod);
                        let mut p_node =
                            LineageNode::new("Pod", pod_name, pod_ns, LineageRelation::Child);
                        p_node.status = p_st;
                        p_node.details = p_dt;
                        p_node.fetch_outcome = Some(NodeOutcome::Healthy);
                        dependents.push(p_node);
                    }
                }
            }
        }
        _ => {}
    }

    // Generic downward dependents search (for CRDs and native controllers)
    let generic_children = discover_dependents(
        cache.clone(),
        context,
        &client,
        &target_uid,
        kind,
        name,
        actual_api_ver.as_deref(),
        resolved_group,
        effective_ns,
        &mut visited_uids,
    )
    .await;

    dependents.extend(generic_children);
    target_node.children.extend(dependents);

    // 2. Resolve Ancestors (Upwards via ownerReferences)
    if let Some(owners) = &obj.metadata.owner_references {
        if !owners.is_empty() {
            let root = nest_in_ancestors(
                &cache,
                context,
                &client,
                target_node,
                owners,
                effective_ns,
                &mut visited_uids,
                0,
            )
            .await;
            return Ok(root);
        }
    }

    Ok(target_node)
}

// ---------------------------------------------------------------------------
// Wire contract and capability definition (k8s.resourceTree)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTreeIn {
    pub context: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub plural: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTreeOut {
    pub root: LineageNode,
    pub total_nodes: usize,
    pub target_path: Vec<String>,
}

fn count_nodes(node: &LineageNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

fn find_target_path(node: &LineageNode) -> Vec<String> {
    if node.relationship == LineageRelation::Target {
        return vec![format!("{}/{}", node.kind, node.name)];
    }
    for child in &node.children {
        let child_path = find_target_path(child);
        if !child_path.is_empty() {
            let mut path = vec![format!("{}/{}", node.kind, node.name)];
            path.extend(child_path);
            return path;
        }
    }
    Vec::new()
}

pub fn resource_tree_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ResourceTreeIn, ResourceTreeOut, _, _>(
        "k8s.resourceTree",
        "fetch relationship lineage tree (ancestors via ownerReferences and dependents) for any resource",
        Annotations::READ_ONLY,
        move |input: ResourceTreeIn| {
            let cache = cache.clone();
            async move {
                let api_ver = input.api_version.clone().or_else(|| {
                    if let (Some(g), Some(v)) = (&input.group, &input.version) {
                        Some(format!("{g}/{v}"))
                    } else {
                        input.version.clone()
                    }
                });
                let root = resolve_resource_lineage(
                    cache,
                    &input.context,
                    &input.kind,
                    &input.name,
                    input.namespace.as_deref(),
                    api_ver.as_deref(),
                    input.group.as_deref(),
                    input.plural.as_deref(),
                )
                .await
                .map_err(CapabilityError::Handler)?;

                let total_nodes = count_nodes(&root);
                let target_path = find_target_path(&root);

                Ok(ResourceTreeOut {
                    root,
                    total_nodes,
                    target_path,
                })
            }
        },
    )
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
        assert!(refs
            .iter()
            .any(|r| r.kind == "ConfigMap" && r.name == "app-config"));
        assert!(refs
            .iter()
            .any(|r| r.kind == "Secret" && r.name == "app-secret"));
        assert!(refs
            .iter()
            .any(|r| r.kind == "PersistentVolumeClaim" && r.name == "data-pvc"));
        assert!(refs
            .iter()
            .any(|r| r.kind == "ConfigMap" && r.name == "env-config"));
    }

    #[test]
    fn test_lineage_node_tree_nesting() {
        let mut root = LineageNode::new(
            "Deployment",
            "web-app",
            Some("default".into()),
            LineageRelation::Owner,
        );
        let mut rs = LineageNode::new(
            "ReplicaSet",
            "web-app-7b8c9d",
            Some("default".into()),
            LineageRelation::Owner,
        );
        let pod = LineageNode::new(
            "Pod",
            "web-app-7b8c9d-x1",
            Some("default".into()),
            LineageRelation::Target,
        );

        rs.children.push(pod);
        root.children.push(rs);

        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].children.len(), 1);
        assert_eq!(root.children[0].children[0].kind, "Pod");
    }

    #[test]
    fn test_owner_hierarchy_ordering() {
        let pod = LineageNode::new(
            "Pod",
            "connection-sink-5z2xs",
            Some("prod".into()),
            LineageRelation::Target,
        );
        let mut rs = LineageNode::new(
            "ReplicaSet",
            "connection-sink-7595b",
            Some("prod".into()),
            LineageRelation::Owner,
        );
        let mut deploy = LineageNode::new(
            "Deployment",
            "connection-sink",
            Some("prod".into()),
            LineageRelation::Owner,
        );

        rs.children.push(pod);
        deploy.children.push(rs);

        assert_eq!(deploy.kind, "Deployment");
        assert_eq!(deploy.children[0].kind, "ReplicaSet");
        assert_eq!(deploy.children[0].children[0].kind, "Pod");
    }

    #[test]
    fn test_resource_tree_in_wire_deserialization() {
        let raw = serde_json::json!({
            "context": "staging-cluster",
            "kind": "VirtualMachine",
            "name": "atlantis-sidecar",
            "apiVersion": "kubevirt.io/v1",
            "namespace": "vm-system"
        });

        let parsed: ResourceTreeIn =
            serde_json::from_value(raw).expect("must deserialize camelCase");
        assert_eq!(parsed.context, "staging-cluster");
        assert_eq!(parsed.kind, "VirtualMachine");
        assert_eq!(parsed.name, "atlantis-sidecar");
        assert_eq!(parsed.api_version.as_deref(), Some("kubevirt.io/v1"));
        assert_eq!(parsed.namespace.as_deref(), Some("vm-system"));
    }

    #[test]
    fn test_lineage_node_wire_serialization() {
        let mut node = LineageNode::new(
            "VirtualMachine",
            "vm-1",
            Some("default".into()),
            LineageRelation::Target,
        )
        .with_api_version("kubevirt.io/v1")
        .with_uid("uid-123")
        .with_outcome(NodeOutcome::Healthy);
        node.status = Some("Running".into());

        let val = serde_json::to_value(&node).expect("must serialize");
        assert_eq!(val["apiVersion"], "kubevirt.io/v1");
        assert_eq!(val["uid"], "uid-123");
        assert_eq!(val["relationship"], "target");
        assert_eq!(val["fetchOutcome"], "healthy");
    }

    #[test]
    fn test_parse_api_version() {
        assert_eq!(
            parse_api_version("kubevirt.io/v1"),
            (Some("kubevirt.io"), Some("v1"))
        );
        assert_eq!(parse_api_version("apps/v1"), (Some("apps"), Some("v1")));
        assert_eq!(parse_api_version("v1"), (None, Some("v1")));
        assert_eq!(parse_api_version(""), (None, None));
    }

    #[test]
    fn test_is_child_of_strict_uid_and_fallback() {
        let owner = OwnerReference {
            api_version: "kubevirt.io/v1".to_string(),
            kind: "VirtualMachine".to_string(),
            name: "vm-1".to_string(),
            uid: "uid-vm-1".to_string(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        };

        // Strict UID match
        assert!(is_child_of(
            &owner,
            "uid-vm-1",
            "VirtualMachine",
            Some("kubevirt.io/v1"),
            "vm-1"
        ));
        // Different UID fails even if name matches!
        assert!(!is_child_of(
            &owner,
            "uid-different",
            "VirtualMachine",
            Some("kubevirt.io/v1"),
            "vm-1"
        ));

        // When target UID is empty: fallback checks kind, name, and apiVersion
        assert!(is_child_of(
            &owner,
            "",
            "VirtualMachine",
            Some("kubevirt.io/v1"),
            "vm-1"
        ));
        assert!(!is_child_of(
            &owner,
            "",
            "DifferentKind",
            Some("kubevirt.io/v1"),
            "vm-1"
        ));
        assert!(!is_child_of(
            &owner,
            "",
            "VirtualMachine",
            Some("kubevirt.io/v1"),
            "wrong-name"
        ));
    }

    #[test]
    fn test_status_extraction_crd_printable_and_conditions() {
        // 1. KubeVirt printableStatus
        let vm_raw = serde_json::json!({
            "apiVersion": "kubevirt.io/v1",
            "kind": "VirtualMachine",
            "status": {
                "printableStatus": "Running"
            }
        });
        let vm_obj: DynamicObject = serde_json::from_value(vm_raw).unwrap();
        let (st, _) = extract_status_and_details("VirtualMachine", &vm_obj);
        assert_eq!(st.as_deref(), Some("Running"));

        // 2. Abnormal condition (Degraded=True) takes precedence
        let cr_raw = serde_json::json!({
            "apiVersion": "example.com/v1",
            "kind": "MyCRD",
            "status": {
                "conditions": [
                    { "type": "Ready", "status": "True" },
                    { "type": "Degraded", "status": "True", "reason": "CrashLoopBackOff" }
                ]
            }
        });
        let cr_obj: DynamicObject = serde_json::from_value(cr_raw).unwrap();
        let (st, dt) = extract_status_and_details("MyCRD", &cr_obj);
        assert_eq!(st.as_deref(), Some("Degraded"));
        assert_eq!(dt.as_deref(), Some("CrashLoopBackOff"));

        // 3. Normal condition (Ready=True)
        let cr_healthy_raw = serde_json::json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "status": {
                "conditions": [
                    { "type": "Ready", "status": "True", "reason": "CertificateIssued" }
                ]
            }
        });
        let cr_healthy_obj: DynamicObject = serde_json::from_value(cr_healthy_raw).unwrap();
        let (st, dt) = extract_status_and_details("Certificate", &cr_healthy_obj);
        assert_eq!(st.as_deref(), Some("Ready"));
        assert_eq!(dt.as_deref(), Some("CertificateIssued"));
    }

    #[tokio::test]
    async fn test_fake_api_generic_crd_owner_chain() {
        use crate::client_cache::DiscoveredResource;
        use crate::crds::custom_api_resource;
        use std::path::PathBuf;

        let (client, _seen) = crate::test_support::fake_api_raw(|seen| {
            if seen.path == "/apis/kubevirt.io/v1/namespaces/default/virtualmachineinstances/vmi-1"
            {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "kubevirt.io/v1",
                        "kind": "VirtualMachineInstance",
                        "metadata": {
                            "name": "vmi-1",
                            "namespace": "default",
                            "uid": "uid-vmi-1",
                            "ownerReferences": [
                                {
                                    "apiVersion": "kubevirt.io/v1",
                                    "kind": "VirtualMachine",
                                    "name": "vm-1",
                                    "uid": "uid-vm-1",
                                    "controller": true
                                }
                            ]
                        },
                        "status": {
                            "phase": "Running"
                        }
                    }),
                )
            } else if seen.path == "/apis/kubevirt.io/v1/namespaces/default/virtualmachines/vm-1" {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "kubevirt.io/v1",
                        "kind": "VirtualMachine",
                        "metadata": {
                            "name": "vm-1",
                            "namespace": "default",
                            "uid": "uid-vm-1"
                        },
                        "status": {
                            "printableStatus": "Running"
                        }
                    }),
                )
            } else if seen.path == "/api/v1/namespaces/default/pods" {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "PodList",
                        "items": [
                            {
                                "apiVersion": "v1",
                                "kind": "Pod",
                                "metadata": {
                                    "name": "virt-launcher-vmi-1-xyz",
                                    "namespace": "default",
                                    "uid": "uid-pod-launcher-1",
                                    "ownerReferences": [
                                        {
                                            "apiVersion": "kubevirt.io/v1",
                                            "kind": "VirtualMachineInstance",
                                            "name": "vmi-1",
                                            "uid": "uid-vmi-1",
                                            "controller": true
                                        }
                                    ]
                                },
                                "status": {
                                    "phase": "Running"
                                }
                            }
                        ]
                    }),
                )
            } else {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "List",
                        "items": []
                    }),
                )
            }
        });

        let cache = ClientCache::new(PathBuf::from("/test/kubeconfig"));
        cache.preload("test-context", client).await;

        let mut disc = crate::client_cache::DiscoveryIndex::new();
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("kubevirt.io", "v1", "VirtualMachine", "virtualmachines"),
            namespaced: true,
        });
        disc.insert(DiscoveredResource {
            ar: custom_api_resource(
                "kubevirt.io",
                "v1",
                "VirtualMachineInstance",
                "virtualmachineinstances",
            ),
            namespaced: true,
        });
        cache.preload_discovery("test-context", disc).await;

        let root = resolve_resource_lineage(
            cache,
            "test-context",
            "VirtualMachineInstance",
            "vmi-1",
            Some("default"),
            Some("kubevirt.io/v1"),
            None,
            None,
        )
        .await
        .expect("lineage should resolve");

        // Root must be the parent VirtualMachine
        assert_eq!(root.kind, "VirtualMachine");
        assert_eq!(root.name, "vm-1");
        assert_eq!(root.status.as_deref(), Some("Running"));
        assert_eq!(root.relationship, LineageRelation::Owner);

        // Child of Root is Target VirtualMachineInstance
        assert_eq!(root.children.len(), 1);
        let vmi = &root.children[0];
        assert_eq!(vmi.kind, "VirtualMachineInstance");
        assert_eq!(vmi.name, "vmi-1");
        assert_eq!(vmi.status.as_deref(), Some("Running"));
        assert_eq!(vmi.relationship, LineageRelation::Target);

        // Child of VirtualMachineInstance is the launcher Pod
        assert_eq!(vmi.children.len(), 1);
        let pod = &vmi.children[0];
        assert_eq!(pod.kind, "Pod");
        assert_eq!(pod.name, "virt-launcher-vmi-1-xyz");
        assert_eq!(pod.relationship, LineageRelation::Child);
    }

    #[tokio::test]
    async fn test_fake_api_parent_not_found_truthfulness() {
        use crate::client_cache::DiscoveredResource;
        use crate::crds::custom_api_resource;
        use std::path::PathBuf;

        let (client, _seen) = crate::test_support::fake_api_raw(|seen| {
            if seen.path
                == "/apis/kubevirt.io/v1/namespaces/default/virtualmachineinstances/vmi-orphaned"
            {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "kubevirt.io/v1",
                        "kind": "VirtualMachineInstance",
                        "metadata": {
                            "name": "vmi-orphaned",
                            "namespace": "default",
                            "uid": "uid-vmi-orphaned",
                            "ownerReferences": [
                                {
                                    "apiVersion": "kubevirt.io/v1",
                                    "kind": "VirtualMachine",
                                    "name": "vm-deleted",
                                    "uid": "uid-vm-deleted",
                                    "controller": true
                                }
                            ]
                        },
                        "status": {
                            "phase": "Running"
                        }
                    }),
                )
            } else if seen.path
                == "/apis/kubevirt.io/v1/namespaces/default/virtualmachines/vm-deleted"
            {
                (
                    404,
                    serde_json::json!({
                        "kind": "Status",
                        "apiVersion": "v1",
                        "status": "Failure",
                        "message": "virtualmachines.kubevirt.io \"vm-deleted\" not found",
                        "reason": "NotFound",
                        "code": 404
                    }),
                )
            } else {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "v1",
                        "kind": "List",
                        "items": []
                    }),
                )
            }
        });

        let cache = ClientCache::new(PathBuf::from("/test/kubeconfig"));
        cache.preload("test-context", client).await;

        let mut disc = crate::client_cache::DiscoveryIndex::new();
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("kubevirt.io", "v1", "VirtualMachine", "virtualmachines"),
            namespaced: true,
        });
        disc.insert(DiscoveredResource {
            ar: custom_api_resource(
                "kubevirt.io",
                "v1",
                "VirtualMachineInstance",
                "virtualmachineinstances",
            ),
            namespaced: true,
        });
        cache.preload_discovery("test-context", disc).await;

        let root = resolve_resource_lineage(
            cache,
            "test-context",
            "VirtualMachineInstance",
            "vmi-orphaned",
            Some("default"),
            Some("kubevirt.io/v1"),
            None,
            None,
        )
        .await
        .expect("lineage should resolve even if parent is deleted");

        assert_eq!(root.kind, "VirtualMachine");
        assert_eq!(root.name, "vm-deleted");
        assert_eq!(root.fetch_outcome, Some(NodeOutcome::NotFound));
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].name, "vmi-orphaned");
    }

    #[test]
    fn test_resource_tree_in_wire_hints_preservation() {
        let raw = serde_json::json!({
            "context": "prod-cluster",
            "kind": "Deployment",
            "name": "custom-app",
            "group": "acme.io",
            "plural": "deployments",
            "namespace": "production"
        });

        let parsed: ResourceTreeIn = serde_json::from_value(raw).expect("must deserialize");
        assert_eq!(parsed.group.as_deref(), Some("acme.io"));
        assert_eq!(parsed.plural.as_deref(), Some("deployments"));
        assert_eq!(parsed.version, None);
    }

    #[test]
    fn test_collect_candidate_resources_includes_same_kind_and_cluster_scoped() {
        use crate::client_cache::DiscoveredResource;
        use crate::crds::custom_api_resource;

        let mut disc = crate::client_cache::DiscoveryIndex::new();
        // Same group, same kind (e.g. nested ArgoCD Applications)
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("argoproj.io", "v1alpha1", "Application", "applications"),
            namespaced: true,
        });
        // Cluster scoped CRD
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("example.com", "v1", "ClusterProfile", "clusterprofiles"),
            namespaced: false,
        });

        let candidates =
            collect_candidate_resources(Some(&disc), "Application", Some("argoproj.io"));
        // Must include Application itself (same-kind)
        assert!(candidates
            .iter()
            .any(|(k, g, _)| k == "Application" && g.as_deref() == Some("argoproj.io")));
        // Must include cluster-scoped CRD from discovered CRDs
        assert!(candidates
            .iter()
            .any(|(k, g, _)| k == "ClusterProfile" && g.as_deref() == Some("example.com")));
    }

    #[tokio::test]
    async fn test_fake_api_cluster_scoped_owner_clears_namespace() {
        use crate::client_cache::DiscoveredResource;
        use crate::crds::custom_api_resource;
        use std::path::PathBuf;

        let (client, _seen) = crate::test_support::fake_api_raw(|seen| {
            if seen.path == "/apis/example.com/v1/namespaces/default/children/child-1" {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "example.com/v1",
                        "kind": "ChildResource",
                        "metadata": {
                            "name": "child-1",
                            "namespace": "default",
                            "uid": "uid-child-1",
                            "ownerReferences": [
                                {
                                    "apiVersion": "example.com/v1",
                                    "kind": "ClusterOwner",
                                    "name": "cluster-owner-1",
                                    "uid": "uid-cluster-owner-1",
                                    "controller": true
                                }
                            ]
                        }
                    }),
                )
            } else if seen.path == "/apis/example.com/v1/clusterowners/cluster-owner-1" {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "example.com/v1",
                        "kind": "ClusterOwner",
                        "metadata": {
                            "name": "cluster-owner-1",
                            "uid": "uid-cluster-owner-1"
                        }
                    }),
                )
            } else {
                (
                    200,
                    serde_json::json!({ "apiVersion": "v1", "kind": "List", "items": [] }),
                )
            }
        });

        let cache = ClientCache::new(PathBuf::from("/test/kubeconfig"));
        cache.preload("test-context", client).await;

        let mut disc = crate::client_cache::DiscoveryIndex::new();
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("example.com", "v1", "ChildResource", "children"),
            namespaced: true,
        });
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("example.com", "v1", "ClusterOwner", "clusterowners"),
            namespaced: false,
        });
        cache.preload_discovery("test-context", disc).await;

        let root = resolve_resource_lineage(
            cache,
            "test-context",
            "ChildResource",
            "child-1",
            Some("default"),
            Some("example.com/v1"),
            None,
            None,
        )
        .await
        .expect("lineage should resolve");

        // The root is cluster-scoped ClusterOwner: its namespace MUST be None, not "default"
        assert_eq!(root.kind, "ClusterOwner");
        assert_eq!(root.namespace, None);

        // The child is namespaced: its namespace MUST be preserved as "default"
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].kind, "ChildResource");
        assert_eq!(root.children[0].namespace.as_deref(), Some("default"));
    }

    #[tokio::test]
    async fn test_fake_api_multi_group_crd_lineage_expansion() {
        use crate::client_cache::DiscoveredResource;
        use crate::crds::custom_api_resource;
        use std::path::PathBuf;

        let (client, _seen) = crate::test_support::fake_api_raw(|seen| {
            if seen.path == "/apis/cert-manager.io/v1/namespaces/default/certificates/my-cert" {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "cert-manager.io/v1",
                        "kind": "Certificate",
                        "metadata": {
                            "name": "my-cert",
                            "namespace": "default",
                            "uid": "uid-cert-1"
                        },
                        "status": {
                            "conditions": [
                                {
                                    "type": "Ready",
                                    "status": "True"
                                }
                            ]
                        }
                    }),
                )
            } else if seen.path == "/apis/cert-manager.io/v1/namespaces/default/certificaterequests"
            {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "cert-manager.io/v1",
                        "kind": "CertificateRequestList",
                        "items": [
                            {
                                "apiVersion": "cert-manager.io/v1",
                                "kind": "CertificateRequest",
                                "metadata": {
                                    "name": "my-cert-req-1",
                                    "namespace": "default",
                                    "uid": "uid-cr-1",
                                    "ownerReferences": [
                                        {
                                            "apiVersion": "cert-manager.io/v1",
                                            "kind": "Certificate",
                                            "name": "my-cert",
                                            "uid": "uid-cert-1",
                                            "controller": true
                                        }
                                    ]
                                },
                                "status": {
                                    "conditions": [
                                        {
                                            "type": "Ready",
                                            "status": "True"
                                        }
                                    ]
                                }
                            }
                        ]
                    }),
                )
            } else if seen.path == "/apis/acme.cert-manager.io/v1/namespaces/default/orders" {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "acme.cert-manager.io/v1",
                        "kind": "OrderList",
                        "items": [
                            {
                                "apiVersion": "acme.cert-manager.io/v1",
                                "kind": "Order",
                                "metadata": {
                                    "name": "my-order-1",
                                    "namespace": "default",
                                    "uid": "uid-order-1",
                                    "ownerReferences": [
                                        {
                                            "apiVersion": "cert-manager.io/v1",
                                            "kind": "CertificateRequest",
                                            "name": "my-cert-req-1",
                                            "uid": "uid-cr-1",
                                            "controller": true
                                        }
                                    ]
                                },
                                "status": {
                                    "state": "valid"
                                }
                            }
                        ]
                    }),
                )
            } else if seen.path == "/apis/acme.cert-manager.io/v1/namespaces/default/challenges" {
                (
                    200,
                    serde_json::json!({
                        "apiVersion": "acme.cert-manager.io/v1",
                        "kind": "ChallengeList",
                        "items": [
                            {
                                "apiVersion": "acme.cert-manager.io/v1",
                                "kind": "Challenge",
                                "metadata": {
                                    "name": "my-chal-1",
                                    "namespace": "default",
                                    "uid": "uid-chal-1",
                                    "ownerReferences": [
                                        {
                                            "apiVersion": "acme.cert-manager.io/v1",
                                            "kind": "Order",
                                            "name": "my-order-1",
                                            "uid": "uid-order-1",
                                            "controller": true
                                        }
                                    ]
                                },
                                "status": {
                                    "state": "valid"
                                }
                            }
                        ]
                    }),
                )
            } else {
                (
                    200,
                    serde_json::json!({ "apiVersion": "v1", "kind": "List", "items": [] }),
                )
            }
        });

        let cache = ClientCache::new(PathBuf::from("/test/kubeconfig"));
        cache.preload("test-context", client).await;

        let mut disc = crate::client_cache::DiscoveryIndex::new();
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("cert-manager.io", "v1", "Certificate", "certificates"),
            namespaced: true,
        });
        disc.insert(DiscoveredResource {
            ar: custom_api_resource(
                "cert-manager.io",
                "v1",
                "CertificateRequest",
                "certificaterequests",
            ),
            namespaced: true,
        });
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("acme.cert-manager.io", "v1", "Order", "orders"),
            namespaced: true,
        });
        disc.insert(DiscoveredResource {
            ar: custom_api_resource("acme.cert-manager.io", "v1", "Challenge", "challenges"),
            namespaced: true,
        });
        cache.preload_discovery("test-context", disc).await;

        let root = resolve_resource_lineage(
            cache,
            "test-context",
            "Certificate",
            "my-cert",
            Some("default"),
            Some("cert-manager.io/v1"),
            None,
            None,
        )
        .await
        .expect("lineage should resolve multi-group CRD chain");

        // Level 1: Root Certificate
        assert_eq!(root.kind, "Certificate");
        assert_eq!(root.name, "my-cert");
        assert_eq!(root.relationship, LineageRelation::Target);
        assert_eq!(root.children.len(), 1);

        // Level 2: Child CertificateRequest (cert-manager.io/v1)
        let cr = &root.children[0];
        assert_eq!(cr.kind, "CertificateRequest");
        assert_eq!(cr.name, "my-cert-req-1");
        assert_eq!(cr.relationship, LineageRelation::Child);
        assert_eq!(cr.children.len(), 1);

        // Level 3: Grandchild Order (acme.cert-manager.io/v1)
        let order = &cr.children[0];
        assert_eq!(order.kind, "Order");
        assert_eq!(order.name, "my-order-1");
        assert_eq!(order.relationship, LineageRelation::Child);
        assert_eq!(order.children.len(), 1);

        // Level 4: Great-grandchild Challenge (acme.cert-manager.io/v1)
        let chal = &order.children[0];
        assert_eq!(chal.kind, "Challenge");
        assert_eq!(chal.name, "my-chal-1");
        assert_eq!(chal.relationship, LineageRelation::Child);
    }
}
