//! ArgoCD application capabilities and data models.
//!
//! Handles both in-cluster ArgoCD deployments and Hub-and-Spoke topologies
//! where a management cluster (with arbitrary name) manages workloads
//! deployed to remote spoke clusters.

use std::sync::Arc;

use kube::api::{Api, ApiResource, DynamicObject, ListParams, Patch, PatchParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client_cache::ClientCache;

pub fn argo_application_resource() -> ApiResource {
    ApiResource {
        group: "argoproj.io".to_string(),
        version: "v1alpha1".to_string(),
        api_version: "argoproj.io/v1alpha1".to_string(),
        kind: "Application".to_string(),
        plural: "applications".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgoResourceItem {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub status: String,
    pub health: String,
    pub message: String,
    pub hook: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgoSyncHistoryItem {
    pub id: i64,
    pub revision: String,
    pub deployed_at: String,
    pub repo_url: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgoApplication {
    pub name: String,
    pub namespace: String,
    pub project: String,
    pub destination_server: String,
    pub destination_name: String,
    pub destination_namespace: String,
    pub repo_url: String,
    pub target_revision: String,
    pub path: String,
    pub sync_status: String,
    pub health_status: String,
    pub health_message: String,
    pub sync_revision: String,
    pub operation_phase: String,
    pub operation_message: String,
    pub auto_sync_enabled: bool,
    pub self_heal_enabled: bool,
    pub prune_enabled: bool,
    pub last_sync_time: String,
    pub created_at: String,
    pub resources: Vec<ArgoResourceItem>,
    pub sync_history: Vec<ArgoSyncHistoryItem>,
}

impl ArgoApplication {
    pub fn from_json(val: &Value) -> Self {
        let meta = val.get("metadata");
        let name = meta.and_then(|m| m.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let namespace = meta.and_then(|m| m.get("namespace")).and_then(|v| v.as_str()).unwrap_or("argocd").to_string();
        let created_at = meta.and_then(|m| m.get("creationTimestamp")).and_then(|v| v.as_str()).unwrap_or("").to_string();

        let spec = val.get("spec");
        let project = spec.and_then(|s| s.get("project")).and_then(|v| v.as_str()).unwrap_or("default").to_string();

        let source = spec.and_then(|s| s.get("source"));
        let repo_url = source.and_then(|s| s.get("repoURL")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let target_revision = source.and_then(|s| s.get("targetRevision")).and_then(|v| v.as_str()).unwrap_or("HEAD").to_string();
        let path = source.and_then(|s| s.get("path")).and_then(|v| v.as_str()).unwrap_or("").to_string();

        let dest = spec.and_then(|s| s.get("destination"));
        let destination_server = dest.and_then(|d| d.get("server")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let destination_name = dest.and_then(|d| d.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let destination_namespace = dest.and_then(|d| d.get("namespace")).and_then(|v| v.as_str()).unwrap_or("default").to_string();

        let sync_policy = spec.and_then(|s| s.get("syncPolicy"));
        let automated = sync_policy.and_then(|p| p.get("automated"));
        let auto_sync_enabled = automated.is_some() && !automated.unwrap().is_null();
        let self_heal_enabled = automated.and_then(|a| a.get("selfHeal")).and_then(|v| v.as_bool()).unwrap_or(false);
        let prune_enabled = automated.and_then(|a| a.get("prune")).and_then(|v| v.as_bool()).unwrap_or(false);

        let status = val.get("status");
        let sync = status.and_then(|s| s.get("sync"));
        let sync_status = sync.and_then(|s| s.get("status")).and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
        let sync_revision = sync.and_then(|s| s.get("revision")).and_then(|v| v.as_str()).unwrap_or("").to_string();

        let health = status.and_then(|s| s.get("health"));
        let health_status = health.and_then(|h| h.get("status")).and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
        let health_message = health.and_then(|h| h.get("message")).and_then(|v| v.as_str()).unwrap_or("").to_string();

        let op_state = status.and_then(|s| s.get("operationState"));
        let operation_phase = op_state.and_then(|o| o.get("phase")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let operation_message = op_state.and_then(|o| o.get("message")).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let last_sync_time = op_state.and_then(|o| o.get("finishedAt")).and_then(|v| v.as_str()).unwrap_or("").to_string();

        let mut resources = Vec::new();
        if let Some(res_arr) = status.and_then(|s| s.get("resources")).and_then(|r| r.as_array()) {
            for r in res_arr {
                let group = r.get("group").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let version = r.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let ns = r.get("namespace").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let r_name = r.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let r_status = r.get("status").and_then(|v| v.as_str()).unwrap_or("Synced").to_string();
                let r_health = r.get("health").and_then(|h| h.get("status")).and_then(|v| v.as_str()).unwrap_or("Healthy").to_string();
                let r_msg = r.get("health").and_then(|h| h.get("message")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let hook = r.get("hook").and_then(|v| v.as_str()).map(ToString::to_string);

                resources.push(ArgoResourceItem {
                    group,
                    version,
                    kind,
                    namespace: ns,
                    name: r_name,
                    status: r_status,
                    health: r_health,
                    message: r_msg,
                    hook,
                });
            }
        }

        let mut sync_history = Vec::new();
        if let Some(hist_arr) = status.and_then(|s| s.get("history")).and_then(|h| h.as_array()) {
            for h in hist_arr {
                let id = h.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let rev = h.get("revision").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let deployed_at = h.get("deployedAt").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let h_src = h.get("source");
                let h_url = h_src.and_then(|s| s.get("repoURL")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let h_path = h_src.and_then(|s| s.get("path")).and_then(|v| v.as_str()).unwrap_or("").to_string();

                sync_history.push(ArgoSyncHistoryItem {
                    id,
                    revision: rev,
                    deployed_at,
                    repo_url: h_url,
                    path: h_path,
                });
            }
        }

        Self {
            name,
            namespace,
            project,
            destination_server,
            destination_name,
            destination_namespace,
            repo_url,
            target_revision,
            path,
            sync_status,
            health_status,
            health_message,
            sync_revision,
            operation_phase,
            operation_message,
            auto_sync_enabled,
            self_heal_enabled,
            prune_enabled,
            last_sync_time,
            created_at,
            resources,
            sync_history,
        }
    }
}

pub fn normalize_server_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_lowercase()
}

pub fn matches_destination(
    app: &ArgoApplication,
    current_context: &str,
    current_server_url: Option<&str>,
    match_by_name: bool,
) -> bool {
    if match_by_name {
        if !app.destination_name.is_empty() && app.destination_name == current_context {
            return true;
        }
    }
    if let Some(server) = current_server_url {
        let norm_current = normalize_server_url(server);
        let norm_dest = normalize_server_url(&app.destination_server);
        if !norm_dest.is_empty() && norm_dest == norm_current {
            return true;
        }
    }
    if !app.destination_name.is_empty() && app.destination_name == current_context {
        return true;
    }
    false
}

/// Fetch applications for the given context.
///
/// If `hub_context` is supplied and differs from `current_context`,
/// SRElens queries the Hub cluster and filters applications targeting `current_context` / `current_server_url`.
/// If `hub_context` matches `current_context` (or no hub is configured), it lists applications on the active cluster.
pub async fn fetch_argo_applications(
    cache: &Arc<ClientCache>,
    current_context: &str,
    current_server_url: Option<&str>,
    hub_context: Option<&str>,
    target_namespace: Option<&str>,
    match_by_name: bool,
) -> Result<(Vec<ArgoApplication>, bool), String> {
    let (query_context, is_remote_hub) = match hub_context {
        Some(hub) if hub != current_context => (hub, true),
        _ => (current_context, false),
    };

    let client = cache
        .get(query_context)
        .await
        .map_err(|e| format!("Failed to connect to cluster '{}': {}", query_context, e))?;

    let ar = argo_application_resource();
    let api: Api<DynamicObject> = match target_namespace {
        Some(ns) if !ns.is_empty() => Api::namespaced_with(client, ns, &ar),
        _ => Api::all_with(client, &ar),
    };

    let list = api
        .list(&ListParams::default())
        .await
        .map_err(|e| {
            let err_str = e.to_string();
            let is_not_found = err_str.contains("404")
                || err_str.to_lowercase().contains("not found")
                || err_str.to_lowercase().contains("notfound");
            if !is_remote_hub && is_not_found {
                "No ArgoCD deployment in this cluster AND no kubeconfig set to point to the ArgoCD cluster.".to_string()
            } else if is_remote_hub && is_not_found {
                format!("Failed to list ArgoCD Applications on hub '{}': ArgoCD CRD (applications.argoproj.io) is not installed on the Hub cluster.", query_context)
            } else {
                format!("Failed to list ArgoCD Applications on '{}': {}", query_context, e)
            }
        })?;

    let all_apps: Vec<ArgoApplication> = list
        .items
        .into_iter()
        .map(|item| {
            let val = serde_json::to_value(&item).unwrap_or_default();
            ArgoApplication::from_json(&val)
        })
        .collect();

    if is_remote_hub {
        let filtered: Vec<ArgoApplication> = all_apps
            .into_iter()
            .filter(|app| matches_destination(app, current_context, current_server_url, match_by_name))
            .collect();
        Ok((filtered, true))
    } else {
        Ok((all_apps, false))
    }
}

pub async fn fetch_argo_application_detail(
    cache: &Arc<ClientCache>,
    context: &str,
    name: &str,
    namespace: &str,
) -> Result<ArgoApplication, String> {
    let client = cache
        .get(context)
        .await
        .map_err(|e| format!("Failed to connect to cluster '{}': {}", context, e))?;

    let ar = argo_application_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &ar);

    let obj = api
        .get(name)
        .await
        .map_err(|e| format!("Failed to get ArgoCD Application '{}/{}': {}", namespace, name, e))?;

    let val = serde_json::to_value(&obj).unwrap_or_default();
    Ok(ArgoApplication::from_json(&val))
}

pub async fn trigger_argo_sync(
    cache: &Arc<ClientCache>,
    context: &str,
    name: &str,
    namespace: &str,
    prune: bool,
    dry_run: bool,
) -> Result<(), String> {
    let client = cache
        .get(context)
        .await
        .map_err(|e| format!("Failed to connect to cluster '{}': {}", context, e))?;

    let ar = argo_application_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &ar);

    let patch = serde_json::json!({
        "operation": {
            "sync": {
                "prune": prune,
                "dryRun": dry_run,
                "strategy": {
                    "hook": {}
                }
            }
        }
    });

    api.patch(name, &PatchParams::apply("srelens"), &Patch::Merge(&patch))
        .await
        .map_err(|e| format!("Failed to trigger sync for '{name}': {e}"))?;

    Ok(())
}

pub async fn toggle_argo_auto_sync(
    cache: &Arc<ClientCache>,
    context: &str,
    name: &str,
    namespace: &str,
    enable: bool,
) -> Result<bool, String> {
    let client = cache
        .get(context)
        .await
        .map_err(|e| format!("Failed to connect to cluster '{}': {}", context, e))?;

    let ar = argo_application_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &ar);

    let patch = if enable {
        serde_json::json!({
            "spec": {
                "syncPolicy": {
                    "automated": {
                        "prune": true,
                        "selfHeal": true
                    }
                }
            }
        })
    } else {
        serde_json::json!({
            "spec": {
                "syncPolicy": {
                    "automated": null
                }
            }
        })
    };

    api.patch(name, &PatchParams::apply("srelens"), &Patch::Merge(&patch))
        .await
        .map_err(|e| format!("Failed to update auto-sync for '{name}': {e}"))?;

    Ok(enable)
}

pub async fn trigger_argo_hard_refresh(
    cache: &Arc<ClientCache>,
    context: &str,
    name: &str,
    namespace: &str,
) -> Result<(), String> {
    let client = cache
        .get(context)
        .await
        .map_err(|e| format!("Failed to connect to cluster '{}': {}", context, e))?;

    let ar = argo_application_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(client, namespace, &ar);

    let patch = serde_json::json!({
        "metadata": {
            "annotations": {
                "argocd.argoproj.io/refresh": "hard"
            }
        }
    });

    api.patch(name, &PatchParams::apply("srelens"), &Patch::Merge(&patch))
        .await
        .map_err(|e| format!("Failed to trigger hard refresh for '{name}': {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_argo_application_json() {
        let raw = serde_json::json!({
            "apiVersion": "argoproj.io/v1alpha1",
            "kind": "Application",
            "metadata": {
                "name": "payments-api",
                "namespace": "argocd",
                "creationTimestamp": "2026-03-01T12:00:00Z"
            },
            "spec": {
                "project": "core",
                "source": {
                    "repoURL": "https://github.com/acme/infra",
                    "targetRevision": "main",
                    "path": "apps/payments"
                },
                "destination": {
                    "server": "https://api.prod-us-east.corp.internal:6443",
                    "name": "prod-us-east",
                    "namespace": "payments"
                },
                "syncPolicy": {
                    "automated": {
                        "prune": true,
                        "selfHeal": true
                    }
                }
            },
            "status": {
                "sync": {
                    "status": "OutOfSync",
                    "revision": "8f3b12a"
                },
                "health": {
                    "status": "Degraded",
                    "message": "Deployment/payments-api has 0 available replicas"
                },
                "operationState": {
                    "phase": "Failed",
                    "message": "one or more synchronization tasks completed with a status of 'Failed'",
                    "finishedAt": "2026-03-01T12:05:00Z"
                },
                "resources": [
                    {
                        "group": "apps",
                        "version": "v1",
                        "kind": "Deployment",
                        "namespace": "payments",
                        "name": "payments-api",
                        "status": "OutOfSync",
                        "health": {
                            "status": "Degraded",
                            "message": "ReplicaFailure"
                        }
                    }
                ],
                "history": [
                    {
                        "id": 12,
                        "revision": "8f3b12a",
                        "deployedAt": "2026-03-01T12:00:00Z",
                        "source": {
                            "repoURL": "https://github.com/acme/infra",
                            "targetRevision": "main",
                            "path": "apps/payments"
                        }
                    }
                ]
            }
        });

        let app = ArgoApplication::from_json(&raw);
        assert_eq!(app.name, "payments-api");
        assert_eq!(app.namespace, "argocd");
        assert_eq!(app.project, "core");
        assert_eq!(app.destination_name, "prod-us-east");
        assert_eq!(app.destination_server, "https://api.prod-us-east.corp.internal:6443");
        assert_eq!(app.destination_namespace, "payments");
        assert_eq!(app.repo_url, "https://github.com/acme/infra");
        assert_eq!(app.target_revision, "main");
        assert_eq!(app.path, "apps/payments");
        assert_eq!(app.sync_status, "OutOfSync");
        assert_eq!(app.health_status, "Degraded");
        assert_eq!(app.health_message, "Deployment/payments-api has 0 available replicas");
        assert_eq!(app.sync_revision, "8f3b12a");
        assert_eq!(app.operation_phase, "Failed");
        assert!(app.auto_sync_enabled);
        assert!(app.self_heal_enabled);
        assert!(app.prune_enabled);
        assert_eq!(app.resources.len(), 1);
        assert_eq!(app.resources[0].name, "payments-api");
        assert_eq!(app.resources[0].health, "Degraded");
        assert_eq!(app.sync_history.len(), 1);
        assert_eq!(app.sync_history[0].id, 12);
    }

    #[test]
    fn correlation_matches_by_server_url_and_cluster_name() {
        let app = ArgoApplication {
            name: "test-app".into(),
            namespace: "argocd".into(),
            project: "default".into(),
            destination_server: "https://api.prod.example.com:6443/".into(),
            destination_name: "prod-cluster".into(),
            destination_namespace: "default".into(),
            repo_url: "".into(),
            target_revision: "".into(),
            path: "".into(),
            sync_status: "Synced".into(),
            health_status: "Healthy".into(),
            health_message: "".into(),
            sync_revision: "".into(),
            operation_phase: "".into(),
            operation_message: "".into(),
            auto_sync_enabled: true,
            self_heal_enabled: false,
            prune_enabled: false,
            last_sync_time: "".into(),
            created_at: "".into(),
            resources: vec![],
            sync_history: vec![],
        };

        // Match by server URL (with trailing slash differences)
        assert!(matches_destination(&app, "any-context", Some("https://api.prod.example.com:6443"), false));
        assert!(matches_destination(&app, "any-context", Some("https://api.prod.example.com:6443/"), false));

        // Match by cluster name
        assert!(matches_destination(&app, "prod-cluster", None, true));
        assert!(matches_destination(&app, "prod-cluster", Some("https://other-url.com"), true));

        // Mismatches
        assert!(!matches_destination(&app, "staging", Some("https://api.staging.example.com:6443"), false));
    }
}
