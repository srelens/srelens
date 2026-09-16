//! ArgoCD application capabilities and data models.
//!
//! Handles both in-cluster ArgoCD deployments and Hub-and-Spoke topologies
//! where a management cluster (with arbitrary name) manages workloads
//! deployed to remote spoke clusters.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ApiResource, DynamicObject, ListParams, Patch, PatchParams};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

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
        let name = meta
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let namespace = meta
            .and_then(|m| m.get("namespace"))
            .and_then(|v| v.as_str())
            .unwrap_or("argocd")
            .to_string();
        let created_at = meta
            .and_then(|m| m.get("creationTimestamp"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let spec = val.get("spec");
        let project = spec
            .and_then(|s| s.get("project"))
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let source = spec.and_then(|s| s.get("source"));
        let repo_url = source
            .and_then(|s| s.get("repoURL"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target_revision = source
            .and_then(|s| s.get("targetRevision"))
            .and_then(|v| v.as_str())
            .unwrap_or("HEAD")
            .to_string();
        let path = source
            .and_then(|s| s.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let dest = spec.and_then(|s| s.get("destination"));
        let destination_server = dest
            .and_then(|d| d.get("server"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let destination_name = dest
            .and_then(|d| d.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let destination_namespace = dest
            .and_then(|d| d.get("namespace"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let sync_policy = spec.and_then(|s| s.get("syncPolicy"));
        let automated = sync_policy.and_then(|p| p.get("automated"));
        let auto_sync_enabled = automated.is_some() && !automated.unwrap().is_null();
        let self_heal_enabled = automated
            .and_then(|a| a.get("selfHeal"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let prune_enabled = automated
            .and_then(|a| a.get("prune"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let status = val.get("status");
        let sync = status.and_then(|s| s.get("sync"));
        let sync_status = sync
            .and_then(|s| s.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sync_revision = sync
            .and_then(|s| s.get("revision"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let health = status.and_then(|s| s.get("health"));
        let health_status = health
            .and_then(|h| h.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let health_message = health
            .and_then(|h| h.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let op_state = status.and_then(|s| s.get("operationState"));
        let operation_phase = op_state
            .and_then(|o| o.get("phase"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let operation_message = op_state
            .and_then(|o| o.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let last_sync_time = op_state
            .and_then(|o| o.get("finishedAt"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut resources = Vec::new();
        if let Some(res_arr) = status
            .and_then(|s| s.get("resources"))
            .and_then(|r| r.as_array())
        {
            for r in res_arr {
                let group = r
                    .get("group")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let version = r
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let kind = r
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let ns = r
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let r_name = r
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let r_status = r
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Synced")
                    .to_string();
                let r_health = r
                    .get("health")
                    .and_then(|h| h.get("status"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Healthy")
                    .to_string();
                let r_msg = r
                    .get("health")
                    .and_then(|h| h.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let hook = r
                    .get("hook")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);

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
        if let Some(hist_arr) = status
            .and_then(|s| s.get("history"))
            .and_then(|h| h.as_array())
        {
            for h in hist_arr {
                let id = h.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let rev = h
                    .get("revision")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let deployed_at = h
                    .get("deployedAt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let h_src = h.get("source");
                let h_url = h_src
                    .and_then(|s| s.get("repoURL"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let h_path = h_src
                    .and_then(|s| s.get("path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgoClusterMapping {
    /// Maps cluster name (lowercase) -> normalized server URL
    pub name_to_server: HashMap<String, String>,
    /// Maps normalized server URL -> cluster name (lowercase)
    pub server_to_name: HashMap<String, String>,
}

impl ArgoClusterMapping {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: &str, server: &str) {
        let n = name.trim().to_lowercase();
        let s = normalize_server_url(server);
        if !n.is_empty() && !s.is_empty() {
            self.name_to_server.insert(n.clone(), s.clone());
            self.server_to_name.insert(s, n);
        }
    }

    pub fn server_for_name(&self, name: &str) -> Option<&str> {
        self.name_to_server
            .get(&name.trim().to_lowercase())
            .map(|s| s.as_str())
    }

    pub fn name_for_server(&self, server: &str) -> Option<&str> {
        let norm = normalize_server_url(server);
        self.server_to_name.get(&norm).map(|s| s.as_str())
    }
}

fn extract_secret_str(secret: &Secret, key: &str) -> Option<String> {
    if let Some(ref data) = secret.data {
        if let Some(bs) = data.get(key) {
            if let Ok(s) = String::from_utf8(bs.0.clone()) {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }
    }
    if let Some(ref string_data) = secret.string_data {
        if let Some(s) = string_data.get(key) {
            let trimmed = s.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

static CLUSTER_MAPPING_CACHE: RwLock<Option<HashMap<String, (Instant, ArgoClusterMapping)>>> =
    RwLock::new(None);
const CLUSTER_MAPPING_TTL: Duration = Duration::from_secs(300);

pub fn invalidate_argo_cluster_mapping_cache() {
    if let Ok(mut guard) = CLUSTER_MAPPING_CACHE.write() {
        *guard = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ArgoAppsCacheKey {
    current_context: String,
    hub_context: Option<String>,
    current_cluster_name: Option<String>,
    current_server_url: Option<String>,
    target_namespace: Option<String>,
}

static ARGO_APPS_CACHE: RwLock<
    Option<HashMap<ArgoAppsCacheKey, (Instant, ArgoApplicationsFetchResult)>>,
> = RwLock::new(None);
const ARGO_APPS_CACHE_TTL: Duration = Duration::from_secs(30);

pub fn invalidate_argo_applications_cache() {
    if let Ok(mut guard) = ARGO_APPS_CACHE.write() {
        *guard = None;
    }
}

pub async fn get_or_fetch_argo_cluster_mapping(
    client: &kube::Client,
    hub_context: &str,
) -> ArgoClusterMapping {
    if let Ok(guard) = CLUSTER_MAPPING_CACHE.read() {
        if let Some(ref map) = *guard {
            if let Some((fetched_at, mapping)) = map.get(hub_context) {
                if fetched_at.elapsed() < CLUSTER_MAPPING_TTL {
                    return mapping.clone();
                }
            }
        }
    }

    let mapping = fetch_argo_cluster_mapping(client).await;
    if let Ok(mut guard) = CLUSTER_MAPPING_CACHE.write() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(hub_context.to_string(), (Instant::now(), mapping.clone()));
    }
    mapping
}

pub async fn fetch_argo_cluster_mapping(client: &kube::Client) -> ArgoClusterMapping {
    let mut mapping = ArgoClusterMapping::new();
    mapping.insert("in-cluster", "https://kubernetes.default.svc");

    let lp = ListParams::default().labels("argocd.argoproj.io/secret-type=cluster");

    // 1. Fast path: check standard "argocd" namespace first with a 2-second timeout.
    // In standard ArgoCD deployments, all cluster secrets live in the ArgoCD control-plane namespace.
    // Querying this single namespace takes ~50ms and avoids scanning all namespaces in the cluster
    // or failing on lack of cluster-scoped Secret RBAC permissions.
    let argocd_api: Api<Secret> = Api::namespaced(client.clone(), "argocd");
    if let Ok(Ok(list)) = tokio::time::timeout(Duration::from_secs(2), argocd_api.list(&lp)).await {
        if !list.items.is_empty() {
            for secret in list.items {
                let name_opt = extract_secret_str(&secret, "name");
                let server_opt = extract_secret_str(&secret, "server");
                if let (Some(name), Some(server)) = (name_opt, server_opt) {
                    mapping.insert(&name, &server);
                }
            }
            return mapping;
        }
    }

    // 2. Fallback: if not in "argocd" namespace, query cluster-wide with a strict 2-second timeout.
    let api_all: Api<Secret> = Api::all(client.clone());
    if let Ok(Ok(list)) = tokio::time::timeout(Duration::from_secs(2), api_all.list(&lp)).await {
        for secret in list.items {
            let name_opt = extract_secret_str(&secret, "name");
            let server_opt = extract_secret_str(&secret, "server");
            if let (Some(name), Some(server)) = (name_opt, server_opt) {
                mapping.insert(&name, &server);
            }
        }
    }
    mapping
}

pub fn matches_destination(
    app: &ArgoApplication,
    current_context: &str,
    current_cluster_name: Option<&str>,
    current_server_url: Option<&str>,
    cluster_mapping: Option<&ArgoClusterMapping>,
    match_by_name: bool,
) -> bool {
    let name_matches = |candidate: &str, dest_name: &str| -> bool {
        if candidate.is_empty() || dest_name.is_empty() {
            return false;
        }
        let c_lower = candidate.to_lowercase();
        let d_lower = dest_name.to_lowercase();
        c_lower == d_lower
            || c_lower.ends_with(&format!("_{}", d_lower))
            || c_lower.ends_with(&format!("-{}", d_lower))
            || c_lower.ends_with(&format!("/{}", d_lower))
            || d_lower.ends_with(&format!("_{}", c_lower))
            || d_lower.ends_with(&format!("-{}", c_lower))
            || d_lower.ends_with(&format!("/{}", c_lower))
    };

    let app_dest_name = &app.destination_name;

    // The explicit opt-in: match on names alone, whatever the servers say.
    let name_fallback = || {
        match_by_name
            && !app_dest_name.is_empty()
            && (name_matches(current_context, app_dest_name)
                || current_cluster_name.is_some_and(|c| name_matches(c, app_dest_name)))
    };

    // Server identity outranks names, `match_by_name` included. The app's
    // server is its own destination.server, or what Argo's cluster secrets
    // register for its destination.name. When that and the current server are
    // both known and differ, this is another cluster however alike the names
    // read — context `prod` passes `name_matches` against destination
    // `team-prod` — and listing it would let a sync land on the wrong spoke
    // (#615). The flag cannot override this: the only production caller
    // (`apps/tui`, the applications fetch) hardcodes it to `true`, so an
    // override would leave that wrong-spoke listing exactly as it was.
    //
    // The cost is that a cluster Argo registers under a different URL than the
    // local kubeconfig no longer matches by name. Two known, differing servers
    // are the one signal here that cannot be a coincidence, and acting on the
    // wrong cluster is worse than not listing its apps.
    let app_server = if !app.destination_server.trim().is_empty() {
        Some(app.destination_server.as_str())
    } else if !app_dest_name.is_empty() {
        cluster_mapping.and_then(|mapping| mapping.server_for_name(app_dest_name))
    } else {
        None
    };
    if let (Some(current), Some(app_server)) = (
        current_server_url.filter(|s| !s.trim().is_empty()),
        app_server,
    ) {
        if normalize_server_url(current) != normalize_server_url(app_server) {
            return false;
        }
    }

    // 1. Direct match by destination_name against context or cluster name
    if !app_dest_name.is_empty() {
        if name_matches(current_context, app_dest_name) {
            return true;
        }
        if let Some(c_cluster) = current_cluster_name {
            if name_matches(c_cluster, app_dest_name) {
                return true;
            }
        }
    }

    // 2. Correlate with ArgoCD cluster secrets mapping
    if let Some(mapping) = cluster_mapping {
        if let Some(curr_server) = current_server_url {
            let norm_curr = normalize_server_url(curr_server);
            if !app_dest_name.is_empty() {
                // If app targets cluster by name, check if that name maps to current_server_url
                if let Some(mapped_server) = mapping.server_for_name(app_dest_name) {
                    if normalize_server_url(mapped_server) == norm_curr {
                        return true;
                    }
                }
            }
            // If current_server_url is registered under a cluster name in ArgoCD, check if app targets that name
            if let Some(mapped_name) = mapping.name_for_server(curr_server) {
                if !app_dest_name.is_empty() && name_matches(mapped_name, app_dest_name) {
                    return true;
                }
            }
        }
        // If app targets cluster by destination_server, check if that server maps to active context/cluster
        if !app.destination_server.is_empty() {
            if let Some(mapped_cluster_name) = mapping.name_for_server(&app.destination_server) {
                if name_matches(mapped_cluster_name, current_context) {
                    return true;
                }
                if let Some(c_cluster) = current_cluster_name {
                    if name_matches(mapped_cluster_name, c_cluster) {
                        return true;
                    }
                }
            }
        }
    }

    // 3. Direct match by destination_server URL
    if let Some(server) = current_server_url {
        let norm_current = normalize_server_url(server);
        let norm_dest = normalize_server_url(&app.destination_server);
        if !norm_dest.is_empty() && norm_dest == norm_current {
            return true;
        }
    }

    // 4. Fallback match_by_name if enabled
    name_fallback()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArgoApplicationsFetchResult {
    /// All applications fetched from the cluster.
    pub all_apps: Vec<ArgoApplication>,
    /// Applications filtered for the active spoke cluster / namespace.
    pub filtered_apps: Vec<ArgoApplication>,
    /// Whether the result was fetched from a remote Hub cluster.
    pub is_remote_hub: bool,
}

/// Fetch applications for the given context.
///
/// If `current_context` has ArgoCD installed (`applications.argoproj.io` CRD exists),
/// applications from `current_context` are returned directly with priority (`is_remote_hub = false`).
/// If ArgoCD is NOT installed on `current_context` and a remote `hub_context` is configured,
/// SRElens falls back to querying the Hub cluster (`is_remote_hub = true`) and filters
/// applications targeting `current_context` / `current_server_url`.
pub async fn fetch_argo_applications(
    cache: &Arc<ClientCache>,
    current_context: &str,
    current_cluster_name: Option<&str>,
    current_server_url: Option<&str>,
    hub_context: Option<&str>,
    target_namespace: Option<&str>,
    match_by_name: bool,
) -> Result<ArgoApplicationsFetchResult, String> {
    fetch_argo_applications_cached(
        cache,
        current_context,
        current_cluster_name,
        current_server_url,
        hub_context,
        target_namespace,
        match_by_name,
        false,
    )
    .await
}

pub async fn fetch_argo_applications_cached(
    cache: &Arc<ClientCache>,
    current_context: &str,
    current_cluster_name: Option<&str>,
    current_server_url: Option<&str>,
    hub_context: Option<&str>,
    target_namespace: Option<&str>,
    match_by_name: bool,
    force_refresh: bool,
) -> Result<ArgoApplicationsFetchResult, String> {
    let cache_key = ArgoAppsCacheKey {
        current_context: current_context.to_string(),
        hub_context: hub_context.map(|s| s.to_string()),
        current_cluster_name: current_cluster_name.map(|s| s.to_string()),
        current_server_url: current_server_url.map(|s| s.to_string()),
        target_namespace: target_namespace.map(|s| s.to_string()),
    };

    if !force_refresh {
        if let Ok(guard) = ARGO_APPS_CACHE.read() {
            if let Some(ref map) = *guard {
                if let Some((fetched_at, cached_res)) = map.get(&cache_key) {
                    if fetched_at.elapsed() < ARGO_APPS_CACHE_TTL {
                        return Ok(cached_res.clone());
                    }
                }
            }
        }
    }

    // 1. Connect to current_context to probe for local ArgoCD installation
    let local_client = cache
        .get(current_context)
        .await
        .map_err(|e| format!("Failed to connect to cluster '{}': {}", current_context, e))?;

    let ar = argo_application_resource();
    let local_api: Api<DynamicObject> = match target_namespace {
        Some(ns) if !ns.is_empty() => Api::namespaced_with(local_client.clone(), ns, &ar),
        _ => Api::all_with(local_client.clone(), &ar),
    };

    let timeout_dur = request_timeout();
    let local_res = tokio::time::timeout(timeout_dur, local_api.list(&ListParams::default())).await;

    match local_res {
        Ok(Ok(list)) => {
            // ArgoCD is installed in the selected cluster! Priority is given to local cluster.
            let all_apps: Vec<ArgoApplication> = list
                .items
                .into_iter()
                .map(|item| {
                    let val = serde_json::to_value(&item).unwrap_or_default();
                    ArgoApplication::from_json(&val)
                })
                .collect();

            let result = ArgoApplicationsFetchResult {
                all_apps: all_apps.clone(),
                filtered_apps: all_apps,
                is_remote_hub: false,
            };

            if let Ok(mut guard) = ARGO_APPS_CACHE.write() {
                let map = guard.get_or_insert_with(HashMap::new);
                map.insert(cache_key, (Instant::now(), result.clone()));
            }

            Ok(result)
        }
        Ok(Err(e)) => {
            let err_str = e.to_string();
            let is_not_found = match &e {
                kube::Error::Api(resp) => resp.code == 404 || resp.reason == "NotFound",
                _ => {
                    err_str.contains("404")
                        || err_str.to_lowercase().contains("not found")
                        || err_str.to_lowercase().contains("notfound")
                }
            };

            if !is_not_found {
                return Err(format!(
                    "Failed to list ArgoCD Applications on '{}': {}",
                    current_context, e
                ));
            }

            // CRD is not installed on the selected cluster.
            // Check if a remote hub cluster is configured.
            let hub = match hub_context {
                Some(h) if h != current_context => h,
                _ => {
                    return Err("No ArgoCD deployment in this cluster AND no kubeconfig set to point to the ArgoCD cluster.".to_string());
                }
            };

            // Spoke cluster: fall back to the remote Hub cluster.
            let hub_client = cache
                .get(hub)
                .await
                .map_err(|e| format!("Failed to connect to cluster '{}': {}", hub, e))?;

            let hub_api: Api<DynamicObject> = Api::all_with(hub_client.clone(), &ar);

            let mapping_fut =
                async { Some(get_or_fetch_argo_cluster_mapping(&hub_client, hub).await) };

            let list_fut = async {
                match tokio::time::timeout(timeout_dur, hub_api.list(&ListParams::default())).await {
                    Ok(res) => res.map_err(|he| {
                        let herr_str = he.to_string();
                        let his_not_found = match &he {
                            kube::Error::Api(resp) => resp.code == 404 || resp.reason == "NotFound",
                            _ => {
                                herr_str.contains("404")
                                    || herr_str.to_lowercase().contains("not found")
                                    || herr_str.to_lowercase().contains("notfound")
                            }
                        };
                        if his_not_found {
                            format!("Failed to list ArgoCD Applications on hub '{}': ArgoCD CRD (applications.argoproj.io) is not installed on the Hub cluster.", hub)
                        } else {
                            format!("Failed to list ArgoCD Applications on hub '{}': {}", hub, he)
                        }
                    }),
                    Err(_) => Err(format!(
                        "Timed out after {}s waiting for ArgoCD Applications from hub '{}'.",
                        timeout_dur.as_secs(),
                        hub
                    )),
                }
            };

            let (cluster_mapping, list_res) = tokio::join!(mapping_fut, list_fut);
            let list = list_res?;

            let all_apps: Vec<ArgoApplication> = list
                .items
                .into_iter()
                .map(|item| {
                    let val = serde_json::to_value(&item).unwrap_or_default();
                    ArgoApplication::from_json(&val)
                })
                .collect();

            let filtered: Vec<ArgoApplication> = all_apps
                .iter()
                .filter(|app| {
                    if !matches_destination(
                        app,
                        current_context,
                        current_cluster_name,
                        current_server_url,
                        cluster_mapping.as_ref(),
                        match_by_name,
                    ) {
                        return false;
                    }
                    if let Some(ns) = target_namespace {
                        if !ns.is_empty() && app.destination_namespace != ns && app.namespace != ns
                        {
                            return false;
                        }
                    }
                    true
                })
                .cloned()
                .collect();

            let result = ArgoApplicationsFetchResult {
                all_apps,
                filtered_apps: filtered,
                is_remote_hub: true,
            };

            if let Ok(mut guard) = ARGO_APPS_CACHE.write() {
                let map = guard.get_or_insert_with(HashMap::new);
                map.insert(cache_key, (Instant::now(), result.clone()));
            }

            Ok(result)
        }
        Err(_) => Err(format!(
            "Timed out after {}s waiting for ArgoCD Applications from '{}'.",
            timeout_dur.as_secs(),
            current_context
        )),
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

    let obj = api.get(name).await.map_err(|e| {
        format!(
            "Failed to get ArgoCD Application '{}/{}': {}",
            namespace, name, e
        )
    })?;

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

/// The merge patch that turns auto-sync on or off.
///
/// Enabling sends `automated: {}` and nothing more. Under JSON merge patch
/// (RFC 7386, what `Patch::Merge` sends for a custom resource) an empty object
/// creates `automated` when the Application has none — auto-sync starts with
/// Argo's defaults, prune and self-heal off — and leaves every key already
/// there untouched when it has one. The confirmation says only "Enable
/// Auto-Sync", so enabling must not switch pruning or self-heal on (#615). Nor
/// may it read the policy first and write it back: a change made between the
/// read and the write would be overwritten with the stale value, and a role
/// allowed to patch Applications but not get them could no longer toggle.
pub fn auto_sync_patch(enable: bool) -> Value {
    let automated = if enable {
        serde_json::json!({})
    } else {
        Value::Null
    };
    serde_json::json!({ "spec": { "syncPolicy": { "automated": automated } } })
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

    let patch = auto_sync_patch(enable);

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
        assert_eq!(
            app.destination_server,
            "https://api.prod-us-east.corp.internal:6443"
        );
        assert_eq!(app.destination_namespace, "payments");
        assert_eq!(app.repo_url, "https://github.com/acme/infra");
        assert_eq!(app.target_revision, "main");
        assert_eq!(app.path, "apps/payments");
        assert_eq!(app.sync_status, "OutOfSync");
        assert_eq!(app.health_status, "Degraded");
        assert_eq!(
            app.health_message,
            "Deployment/payments-api has 0 available replicas"
        );
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
    fn test_parse_argo_application_cluster_scoped_and_missing_sync_status() {
        let raw = serde_json::json!({
            "metadata": {
                "name": "cluster-roles-app",
                "namespace": "argocd"
            },
            "spec": {
                "destination": {
                    "server": "https://kubernetes.default.svc"
                }
            },
            "status": {}
        });

        let app = ArgoApplication::from_json(&raw);
        assert_eq!(app.name, "cluster-roles-app");
        assert_eq!(app.destination_namespace, "");
        assert_eq!(app.sync_status, "");
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
        assert!(matches_destination(
            &app,
            "any-context",
            None,
            Some("https://api.prod.example.com:6443"),
            None,
            false
        ));
        assert!(matches_destination(
            &app,
            "any-context",
            None,
            Some("https://api.prod.example.com:6443/"),
            None,
            false
        ));

        // Match by cluster name and suffix (e.g. GKE / EKS context name)
        assert!(matches_destination(
            &app,
            "prod-cluster",
            None,
            None,
            None,
            true
        ));
        // ...but not once the servers are known to differ: this app targets
        // api.prod.example.com and the reader is on other-url.com, so the
        // matching name is a coincidence, and `match_by_name` does not override
        // it (#615). This asserted a match until that change.
        assert!(!matches_destination(
            &app,
            "prod-cluster",
            None,
            Some("https://other-url.com"),
            None,
            true
        ));
        assert!(matches_destination(
            &app,
            "gke_org-prod_us-east1_prod-cluster",
            None,
            None,
            None,
            true
        ));
        assert!(matches_destination(
            &app,
            "arn:aws:eks:us-east-1:123456789012:cluster/prod-cluster",
            None,
            None,
            None,
            true
        ));

        // Match by explicit cluster name
        assert!(matches_destination(
            &app,
            "some-generic-context",
            Some("prod-cluster"),
            None,
            None,
            false
        ));

        // Mismatches
        assert!(!matches_destination(
            &app,
            "staging",
            None,
            Some("https://api.staging.example.com:6443"),
            None,
            false
        ));
        assert!(!matches_destination(
            &app,
            "gke_org-prod_us-east1_staging-cluster",
            None,
            None,
            None,
            true
        ));
    }

    #[test]
    fn correlation_matches_via_cluster_mapping() {
        // Application has empty destination_server, only destination_name
        let app = ArgoApplication {
            name: "backend-service".into(),
            namespace: "argocd".into(),
            project: "default".into(),
            destination_server: "".into(),
            destination_name: "search-backend-prod0-eu-w4".into(),
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

        let mut mapping = ArgoClusterMapping::new();
        mapping.insert("search-backend-prod0-eu-w4", "https://10.245.248.2");

        // When current_context is arbitrary, but server matches registered cluster secret server URL
        assert!(matches_destination(
            &app,
            "arbitrary-context-name",
            None,
            Some("https://10.245.248.2"),
            Some(&mapping),
            false,
        ));

        // When server has trailing slash
        assert!(matches_destination(
            &app,
            "arbitrary-context-name",
            None,
            Some("https://10.245.248.2/"),
            Some(&mapping),
            false,
        ));

        // When cluster secret name matches the active cluster name
        assert!(matches_destination(
            &app,
            "some-ctx",
            Some("search-backend-prod0-eu-w4"),
            None,
            Some(&mapping),
            false,
        ));

        // Non-matching server URL
        assert!(!matches_destination(
            &app,
            "arbitrary-context-name",
            None,
            Some("https://10.245.248.99"),
            Some(&mapping),
            false,
        ));
    }

    #[test]
    fn spoke_filter_excludes_apps_targeting_other_spokes_even_with_cluster_mapping() {
        let mut mapping = ArgoClusterMapping::new();
        mapping.insert("data-processing-stage-eu-dus1", "https://10.200.1.1:6443");
        mapping.insert("advertiser-service-prod0-as-se1", "https://10.200.2.1:6443");

        // App targeting another spoke cluster (advertiser-service-prod0-as-se1)
        let other_app = ArgoApplication {
            name: "abreuv2-prod-as-se1".into(),
            namespace: "argocd".into(),
            project: "default".into(),
            destination_server: "".into(),
            destination_name: "advertiser-service-prod0-as-se1".into(),
            destination_namespace: "connector".into(),
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

        // Current active context is data-processing-stage-eu-dus1
        assert!(!matches_destination(
            &other_app,
            "data-processing-stage-eu-dus1",
            Some("data-processing-stage-eu-dus1"),
            Some("https://10.200.1.1:6443"),
            Some(&mapping),
            false,
        ), "App targeting advertiser-service-prod0-as-se1 must NOT match active cluster data-processing-stage-eu-dus1");

        // App targeting active cluster by destination_server URL registered in mapping
        let spoke_server_app = ArgoApplication {
            name: "stage-data-pipeline".into(),
            namespace: "argocd".into(),
            project: "default".into(),
            destination_server: "https://10.200.1.1:6443".into(),
            destination_name: "".into(),
            destination_namespace: "pipeline".into(),
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

        assert!(
            matches_destination(
                &spoke_server_app,
                "data-processing-stage-eu-dus1",
                Some("data-processing-stage-eu-dus1"),
                Some("https://10.200.1.1:6443"),
                Some(&mapping),
                false,
            ),
            "App targeting active cluster via registered destination_server must match"
        );
    }

    #[test]
    fn a_known_different_server_outranks_a_similar_name() {
        // #615: context `prod` passes `name_matches` against destination
        // `team-prod`. Once both servers are known and differ, that app belongs
        // to another spoke and must not be listed, or synced, from this one.
        let mut mapping = ArgoClusterMapping::new();
        mapping.insert("prod", "https://10.0.0.1:6443");
        mapping.insert("team-prod", "https://10.0.0.2:6443");
        let mut by_name = ArgoApplication::from_json(&serde_json::json!({}));
        by_name.destination_name = "team-prod".to_string();
        let mut by_server = ArgoApplication::from_json(&serde_json::json!({}));
        by_server.destination_server = "https://10.0.0.2:6443".to_string();

        for app in [&by_name, &by_server] {
            assert!(!matches_destination(
                app,
                "prod",
                Some("prod"),
                Some("https://10.0.0.1:6443"),
                Some(&mapping),
                false,
            ));
        }

        // `match_by_name` does not override it. The only production caller
        // hardcodes the flag to `true`, so an override would leave the
        // wrong-spoke listing exactly as it was.
        assert!(!matches_destination(
            &by_name,
            "prod",
            None,
            Some("https://10.0.0.1:6443"),
            Some(&mapping),
            true,
        ));

        // With no server known on one side, the name is the only signal there
        // is, and fuzzy matching still applies.
        assert!(matches_destination(
            &by_name,
            "prod",
            None,
            None,
            Some(&mapping),
            false
        ));
        assert!(matches_destination(
            &by_name,
            "prod",
            None,
            Some("https://10.0.0.1:6443"),
            Some(&ArgoClusterMapping::new()),
            false,
        ));
    }

    #[test]
    fn enabling_auto_sync_sets_no_policy_flags_of_its_own() {
        // #615: enabling once patched prune and selfHeal to true whatever the
        // Application declared, so a toggle labelled "Enable Auto-Sync" also
        // opted it into deleting resources and reverting live changes. An empty
        // `automated` under merge patch creates the policy with Argo's defaults
        // when absent and leaves an existing one's keys untouched, without a
        // read that could write back a stale policy.
        assert_eq!(
            auto_sync_patch(true),
            serde_json::json!({"spec": {"syncPolicy": {"automated": {}}}})
        );
        assert_eq!(
            auto_sync_patch(false),
            serde_json::json!({"spec": {"syncPolicy": {"automated": null}}})
        );
    }

    static CACHE_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_argo_application_resource() {
        let res = argo_application_resource();
        assert_eq!(res.group, "argoproj.io");
        assert_eq!(res.version, "v1alpha1");
        assert_eq!(res.api_version, "argoproj.io/v1alpha1");
        assert_eq!(res.kind, "Application");
        assert_eq!(res.plural, "applications");
    }

    #[test]
    fn test_normalize_server_url() {
        assert_eq!(
            normalize_server_url("https://10.0.0.1:6443/"),
            "https://10.0.0.1:6443"
        );
        assert_eq!(
            normalize_server_url("  https://KUBERNETES.default.svc/  "),
            "https://kubernetes.default.svc"
        );
        assert_eq!(normalize_server_url(""), "");
        assert_eq!(normalize_server_url("///"), "");
    }

    #[test]
    fn test_argo_cluster_mapping_methods() {
        let mut mapping = ArgoClusterMapping::new();
        assert_eq!(mapping.server_for_name("cluster-a"), None);
        assert_eq!(mapping.name_for_server("https://10.0.0.1"), None);

        // Insertion ignores empty name or server
        mapping.insert("", "https://10.0.0.1");
        mapping.insert("cluster-b", "");
        assert_eq!(mapping.server_for_name(""), None);

        mapping.insert(" Cluster-PROD ", "https://api.prod.local:6443/ ");
        assert_eq!(
            mapping.server_for_name("cluster-prod"),
            Some("https://api.prod.local:6443")
        );
        assert_eq!(
            mapping.server_for_name("CLUSTER-PROD"),
            Some("https://api.prod.local:6443")
        );
        assert_eq!(
            mapping.name_for_server("https://api.prod.local:6443/"),
            Some("cluster-prod")
        );
        assert_eq!(
            mapping.name_for_server("https://api.prod.local:6443"),
            Some("cluster-prod")
        );
    }

    #[test]
    fn test_argo_application_from_json_minimal_and_edge_cases() {
        // 1. Completely empty JSON
        let app = ArgoApplication::from_json(&serde_json::json!({}));
        assert_eq!(app.name, "");
        assert_eq!(app.namespace, "argocd");
        assert_eq!(app.project, "default");
        assert_eq!(app.target_revision, "HEAD");
        assert_eq!(app.destination_server, "");
        assert_eq!(app.destination_name, "");
        assert_eq!(app.destination_namespace, "");
        assert_eq!(app.sync_status, "");
        assert_eq!(app.health_status, "Unknown");
        assert!(!app.auto_sync_enabled);
        assert!(!app.self_heal_enabled);
        assert!(!app.prune_enabled);
        assert!(app.resources.is_empty());
        assert!(app.sync_history.is_empty());

        // 2. Partial JSON with nulls and arrays containing missing fields
        let partial = serde_json::json!({
            "metadata": {
                "name": "edge-app",
                "namespace": null
            },
            "spec": {
                "destination": null,
                "syncPolicy": {
                    "automated": null
                }
            },
            "status": {
                "resources": [
                    {
                        "name": "pod-1"
                    },
                    null
                ],
                "history": [
                    {
                        "id": "not-a-number",
                        "revision": "abc"
                    },
                    null
                ]
            }
        });
        let app2 = ArgoApplication::from_json(&partial);
        assert_eq!(app2.name, "edge-app");
        assert_eq!(app2.namespace, "argocd");
        assert_eq!(app2.resources.len(), 2);
        assert_eq!(app2.resources[0].name, "pod-1");
        assert_eq!(app2.resources[0].status, "Synced");
        assert_eq!(app2.resources[0].health, "Healthy");
        assert_eq!(app2.sync_history.len(), 2);
        assert_eq!(app2.sync_history[0].id, 0);
        assert_eq!(app2.sync_history[0].revision, "abc");
    }

    #[test]
    fn argo_applications_cache_stores_and_invalidates() {
        let _lock = CACHE_TEST_MUTEX.lock().unwrap();
        invalidate_argo_applications_cache();
        let key = ArgoAppsCacheKey {
            current_context: "spoke-cluster".to_string(),
            hub_context: Some("hub-cluster".to_string()),
            current_cluster_name: Some("spoke-cluster".to_string()),
            current_server_url: Some("https://10.0.0.1:6443".to_string()),
            target_namespace: None,
        };

        let result = ArgoApplicationsFetchResult {
            all_apps: vec![],
            filtered_apps: vec![],
            is_remote_hub: true,
        };

        // Cache insert
        if let Ok(mut guard) = ARGO_APPS_CACHE.write() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(key.clone(), (Instant::now(), result.clone()));
        }

        // Cache hit
        {
            let guard = ARGO_APPS_CACHE.read().unwrap();
            let map = guard.as_ref().unwrap();
            let (fetched_at, cached) = map.get(&key).unwrap();
            assert_eq!(cached, &result);
            assert!(fetched_at.elapsed() < Duration::from_secs(10));
        }

        // Invalidation clears cache
        invalidate_argo_applications_cache();
        {
            let guard = ARGO_APPS_CACHE.read().unwrap();
            assert!(guard.is_none());
        }
    }

    #[test]
    fn cluster_mapping_cache_stores_and_invalidates() {
        let _lock = CACHE_TEST_MUTEX.lock().unwrap();
        invalidate_argo_cluster_mapping_cache();
        let mut mapping = ArgoClusterMapping::new();
        mapping.insert("spoke-cluster", "https://10.0.0.1:6443");

        // Cache insert
        if let Ok(mut guard) = CLUSTER_MAPPING_CACHE.write() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert("hub-cluster".to_string(), (Instant::now(), mapping.clone()));
        }

        // Cache hit
        {
            let guard = CLUSTER_MAPPING_CACHE.read().unwrap();
            let map = guard.as_ref().unwrap();
            let (fetched_at, cached) = map.get("hub-cluster").unwrap();
            assert_eq!(cached, &mapping);
            assert!(fetched_at.elapsed() < Duration::from_secs(10));
        }

        // Invalidation clears cache
        invalidate_argo_cluster_mapping_cache();
        {
            let guard = CLUSTER_MAPPING_CACHE.read().unwrap();
            assert!(guard.is_none());
        }
    }

    #[test]
    fn extract_secret_str_reads_byte_data_then_string_data_then_none() {
        use k8s_openapi::api::core::v1::Secret;
        use std::collections::BTreeMap;

        // Byte `data` takes priority when present and valid UTF-8.
        let mut data = BTreeMap::new();
        data.insert(
            "server".to_string(),
            k8s_openapi::ByteString(b"https://10.0.0.1:6443".to_vec()),
        );
        let secret = Secret {
            data: Some(data),
            ..Default::default()
        };
        assert_eq!(
            extract_secret_str(&secret, "server"),
            Some("https://10.0.0.1:6443".to_string())
        );

        // Falls back to `string_data` when `data` doesn't have the key.
        let mut string_data = BTreeMap::new();
        string_data.insert("name".to_string(), "  spoke-cluster  ".to_string());
        let secret = Secret {
            data: None,
            string_data: Some(string_data),
            ..Default::default()
        };
        assert_eq!(
            extract_secret_str(&secret, "name"),
            Some("spoke-cluster".to_string())
        );

        // Empty/whitespace-only values and missing keys yield None.
        let secret = Secret::default();
        assert_eq!(extract_secret_str(&secret, "server"), None);

        let mut blank_string_data = BTreeMap::new();
        blank_string_data.insert("server".to_string(), "   ".to_string());
        let secret = Secret {
            string_data: Some(blank_string_data),
            ..Default::default()
        };
        assert_eq!(extract_secret_str(&secret, "server"), None);
    }

    #[tokio::test]
    async fn fetch_argo_applications_cached_returns_cache_hit_without_contacting_cluster() {
        let _lock = CACHE_TEST_MUTEX.lock().unwrap();
        invalidate_argo_applications_cache();
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));

        let key = ArgoAppsCacheKey {
            current_context: "cache-hit-ctx".to_string(),
            hub_context: None,
            current_cluster_name: None,
            current_server_url: None,
            target_namespace: None,
        };
        let cached_result = ArgoApplicationsFetchResult {
            all_apps: vec![],
            filtered_apps: vec![],
            is_remote_hub: false,
        };
        if let Ok(mut guard) = ARGO_APPS_CACHE.write() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(key, (Instant::now(), cached_result.clone()));
        }

        // No client is configured on this cache, so a non-error result here
        // proves the cache-hit fast path returned before any cluster contact.
        let result = fetch_argo_applications_cached(
            &cache,
            "cache-hit-ctx",
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();
        assert_eq!(result, cached_result);

        invalidate_argo_applications_cache();
    }

    #[tokio::test]
    async fn fetch_argo_applications_cached_reports_connect_failure_on_cache_miss() {
        let _lock = CACHE_TEST_MUTEX.lock().unwrap();
        invalidate_argo_applications_cache();
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));

        let err = fetch_argo_applications_cached(
            &cache,
            "nonexistent-ctx-for-test",
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert!(err.contains("Failed to connect to cluster"));
    }

    #[tokio::test]
    async fn fetch_argo_applications_passthrough_and_actions_connect_failures() {
        let _lock = CACHE_TEST_MUTEX.lock().unwrap();
        invalidate_argo_applications_cache();
        let cache = ClientCache::new(std::path::PathBuf::from("/dev/null"));

        // 1. fetch_argo_applications wrapper calls cached with false
        let err = fetch_argo_applications(&cache, "nonexistent-ctx", None, None, None, None, false)
            .await
            .unwrap_err();
        assert!(err.contains("Failed to connect to cluster"));

        // 2. fetch_argo_application_detail reports connect error
        let err = fetch_argo_application_detail(&cache, "nonexistent-ctx", "my-app", "argocd")
            .await
            .unwrap_err();
        assert!(err.contains("Failed to connect to cluster"));

        // 3. trigger_argo_sync reports connect error
        let err = trigger_argo_sync(&cache, "nonexistent-ctx", "my-app", "argocd", false, false)
            .await
            .unwrap_err();
        assert!(err.contains("Failed to connect to cluster"));

        // 4. toggle_argo_auto_sync reports connect error (enable true and false)
        let err = toggle_argo_auto_sync(&cache, "nonexistent-ctx", "my-app", "argocd", true)
            .await
            .unwrap_err();
        assert!(err.contains("Failed to connect to cluster"));

        let err = toggle_argo_auto_sync(&cache, "nonexistent-ctx", "my-app", "argocd", false)
            .await
            .unwrap_err();
        assert!(err.contains("Failed to connect to cluster"));

        // 5. trigger_argo_hard_refresh reports connect error
        let err = trigger_argo_hard_refresh(&cache, "nonexistent-ctx", "my-app", "argocd")
            .await
            .unwrap_err();
        assert!(err.contains("Failed to connect to cluster"));
    }

    #[test]
    fn matches_destination_name_delimiters_and_mapping_edges() {
        let mut app = ArgoApplication::from_json(&serde_json::json!({}));
        app.destination_name = "prod-cluster".to_string();

        // Delimiter matching (_ / -)
        assert!(matches_destination(
            &app,
            "gke_prod-cluster",
            None,
            None,
            None,
            false
        ));
        assert!(matches_destination(
            &app,
            "my-org/prod-cluster",
            None,
            None,
            None,
            false
        ));
        assert!(matches_destination(
            &app,
            "east-prod-cluster",
            None,
            None,
            None,
            false
        ));

        // Mapping with server URL matching
        let mut mapping = ArgoClusterMapping::new();
        mapping.insert("staging-cluster", "https://k8s.staging.example.com:6443");
        app.destination_name = "staging-cluster".to_string();

        assert!(matches_destination(
            &app,
            "arbitrary-ctx",
            None,
            Some("https://k8s.staging.example.com:6443/"),
            Some(&mapping),
            false,
        ));

        // App destination_server matching mapped cluster
        app.destination_name = String::new();
        app.destination_server = "https://k8s.staging.example.com:6443".to_string();
        assert!(matches_destination(
            &app,
            "staging-cluster",
            None,
            None,
            Some(&mapping),
            false,
        ));
    }
}
