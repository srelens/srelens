//! The `k8s.getManifest` capability — fetch any supported resource as YAML via
//! kube-rs's dynamic API, so a single capability serves every resource type.

use std::sync::Arc;

use kube::api::{Api, DynamicObject, ListParams, Patch, PatchParams, ValidationDirective};
use kube::core::{ApiResource, GroupVersionKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Identifying fields of a manifest document.
#[derive(Debug, Clone)]
pub struct ResourceRef {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

/// Pull the identifying fields from a parsed manifest document.
pub fn resource_ref(value: &serde_json::Value) -> Option<ResourceRef> {
    let s = |path: &[&str]| -> Option<String> {
        let mut cur = value;
        for key in path {
            cur = cur.get(key)?;
        }
        cur.as_str().map(String::from)
    };
    Some(ResourceRef {
        api_version: s(&["apiVersion"])?,
        kind: s(&["kind"])?,
        name: s(&["metadata", "name"])?,
        namespace: s(&["metadata", "namespace"]),
    })
}

/// Side-by-side alignment tag for one diff row.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DiffTag {
    Same,
    Insert,
    Delete,
    Replace,
}

/// One aligned row of a side-by-side diff. `left` is the current (live) line,
/// `right` the proposed line; either is `None` for a pure insert/delete.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DiffRow {
    pub tag: DiffTag,
    pub left: Option<String>,
    pub right: Option<String>,
}

/// Compute line-aligned side-by-side rows. Consecutive deletes followed by
/// inserts pair into `Replace` rows; leftovers become `Delete`/`Insert`.
pub fn align_rows(current: &str, proposed: &str) -> Vec<DiffRow> {
    let diff = TextDiff::from_lines(current, proposed);
    let mut rows = Vec::new();
    let mut pending: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let flush = |rows: &mut Vec<DiffRow>, pending: &mut std::collections::VecDeque<String>| {
        while let Some(left) = pending.pop_front() {
            rows.push(DiffRow {
                tag: DiffTag::Delete,
                left: Some(left),
                right: None,
            });
        }
    };
    for change in diff.iter_all_changes() {
        let line = change
            .value()
            .strip_suffix('\n')
            .unwrap_or(change.value())
            .to_string();
        match change.tag() {
            ChangeTag::Equal => {
                flush(&mut rows, &mut pending);
                rows.push(DiffRow {
                    tag: DiffTag::Same,
                    left: Some(line.clone()),
                    right: Some(line),
                });
            }
            ChangeTag::Delete => pending.push_back(line),
            ChangeTag::Insert => {
                if let Some(left) = pending.pop_front() {
                    rows.push(DiffRow {
                        tag: DiffTag::Replace,
                        left: Some(left),
                        right: Some(line),
                    });
                } else {
                    rows.push(DiffRow {
                        tag: DiffTag::Insert,
                        left: None,
                        right: Some(line),
                    });
                }
            }
        }
    }
    flush(&mut rows, &mut pending);
    rows
}

/// Split a multi-document YAML string into parsed JSON values, skipping empty
/// or whitespace/comment-only documents. Single source of truth for apply-all.
pub fn split_documents(yaml: &str) -> Result<Vec<serde_json::Value>, CapabilityError> {
    let mut out = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let value = serde_json::Value::deserialize(doc)
            .map_err(|e| CapabilityError::Handler(format!("parse yaml: {e}")))?;
        if value.is_null() {
            continue;
        }
        out.push(value);
    }
    Ok(out)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ManifestIn {
    pub context: String,
    /// Kubernetes Kind, e.g. "Pod", "Deployment", "Node".
    pub kind: String,
    /// Namespace (ignored for cluster-scoped kinds).
    #[serde(default)]
    pub namespace: Option<String>,
    pub name: String,
    // Optional dynamic GVK + plural for custom resources not in `gvk_for`.
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub plural: Option<String>,
}

/// Resolve the (ApiResource, namespaced) for a request: a dynamic CRD GVK if
/// group/version/plural are supplied, else the static `gvk_for` table.
fn resolve_api_resource(input: &ManifestIn) -> Result<(ApiResource, bool), CapabilityError> {
    if let (Some(g), Some(v), Some(p)) = (
        input.group.as_deref(),
        input.version.as_deref(),
        input.plural.as_deref(),
    ) {
        let namespaced = input.namespace.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
        Ok((crate::crds::custom_api_resource(g, v, &input.kind, p), namespaced))
    } else {
        let (gvk, namespaced) = gvk_for(&input.kind)
            .ok_or_else(|| CapabilityError::Handler(format!("unsupported kind: {}", input.kind)))?;
        Ok((ApiResource::from_gvk(&gvk), namespaced))
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ManifestOut {
    pub yaml: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectOut {
    pub object: serde_json::Value,
}

/// `k8s.getObject` — fetch a resource as a structured JSON object (for rich
/// detail rendering, vs. `k8s.getManifest` which returns YAML).
pub fn get_object_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ManifestIn, ObjectOut, _, _>(
        "k8s.getObject",
        "fetch a resource as a structured JSON object (any supported kind)",
        Annotations::READ_ONLY,
        move |input: ManifestIn| {
            let cache = cache.clone();
            async move {
                let (ar, namespaced) = resolve_api_resource(&input)?;
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api: Api<DynamicObject> = if namespaced {
                    let ns = input.namespace.as_deref().filter(|s| !s.is_empty()).unwrap_or("default");
                    Api::namespaced_with(client, ns, &ar)
                } else {
                    Api::all_with(client, &ar)
                };
                let mut obj = tokio::time::timeout(request_timeout(), api.get(&input.name))
                    .await
                    .map_err(|_| CapabilityError::Handler("get object timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                obj.metadata.managed_fields = None;
                let mut object = serde_json::to_value(obj)
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                // Never return Secret values through the generic path; the UI
                // reads them via the dedicated, consent-gateable `k8s.getSecret`.
                if ar.kind == "Secret" && ar.group.is_empty() {
                    crate::secrets::redact_secret_data(&mut object);
                }
                Ok(ObjectOut { object })
            }
        },
    )
}

/// Map a server-side-apply patch outcome to a per-document result: success,
/// a structured 409 conflict, or a cleaned error. Pure, so it is unit-testable.
fn apply_doc_from_result(kind: String, name: String, result: Result<(), kube::Error>) -> ApplyDoc {
    match result {
        Ok(()) => ApplyDoc {
            kind,
            name,
            applied: true,
            conflict: None,
            error: None,
        },
        Err(kube::Error::Api(resp)) if resp.code == 409 => ApplyDoc {
            kind,
            name,
            applied: false,
            conflict: Some(parse_conflict(&resp.message)),
            error: None,
        },
        Err(e) => ApplyDoc {
            kind,
            name,
            applied: false,
            conflict: None,
            error: Some(clean_kube_error(e)),
        },
    }
}

/// Map a supported Kind to its GroupVersionKind and whether it is namespaced.
pub fn gvk_for(kind: &str) -> Option<(GroupVersionKind, bool)> {
    let (group, version, k, namespaced) = match kind {
        "Pod" => ("", "v1", "Pod", true),
        "Service" => ("", "v1", "Service", true),
        "ConfigMap" => ("", "v1", "ConfigMap", true),
        "Secret" => ("", "v1", "Secret", true),
        "Namespace" => ("", "v1", "Namespace", false),
        "Node" => ("", "v1", "Node", false),
        "Deployment" => ("apps", "v1", "Deployment", true),
        "StatefulSet" => ("apps", "v1", "StatefulSet", true),
        "DaemonSet" => ("apps", "v1", "DaemonSet", true),
        "ReplicaSet" => ("apps", "v1", "ReplicaSet", true),
        "Job" => ("batch", "v1", "Job", true),
        "CronJob" => ("batch", "v1", "CronJob", true),
        "Ingress" => ("networking.k8s.io", "v1", "Ingress", true),
        "NetworkPolicy" => ("networking.k8s.io", "v1", "NetworkPolicy", true),
        "Endpoints" => ("", "v1", "Endpoints", true),
        "Event" => ("", "v1", "Event", true),
        "ServiceAccount" => ("", "v1", "ServiceAccount", true),
        "PersistentVolumeClaim" => ("", "v1", "PersistentVolumeClaim", true),
        "PersistentVolume" => ("", "v1", "PersistentVolume", false),
        "Role" => ("rbac.authorization.k8s.io", "v1", "Role", true),
        "RoleBinding" => ("rbac.authorization.k8s.io", "v1", "RoleBinding", true),
        "ClusterRole" => ("rbac.authorization.k8s.io", "v1", "ClusterRole", false),
        "ClusterRoleBinding" => ("rbac.authorization.k8s.io", "v1", "ClusterRoleBinding", false),
        // Config
        "ResourceQuota" => ("", "v1", "ResourceQuota", true),
        "LimitRange" => ("", "v1", "LimitRange", true),
        "HorizontalPodAutoscaler" => ("autoscaling", "v2", "HorizontalPodAutoscaler", true),
        "PodDisruptionBudget" => ("policy", "v1", "PodDisruptionBudget", true),
        "PriorityClass" => ("scheduling.k8s.io", "v1", "PriorityClass", false),
        "RuntimeClass" => ("node.k8s.io", "v1", "RuntimeClass", false),
        "Lease" => ("coordination.k8s.io", "v1", "Lease", true),
        "MutatingWebhookConfiguration" => {
            ("admissionregistration.k8s.io", "v1", "MutatingWebhookConfiguration", false)
        }
        "ValidatingWebhookConfiguration" => {
            ("admissionregistration.k8s.io", "v1", "ValidatingWebhookConfiguration", false)
        }
        // Network
        "EndpointSlice" => ("discovery.k8s.io", "v1", "EndpointSlice", true),
        "IngressClass" => ("networking.k8s.io", "v1", "IngressClass", false),
        // Storage
        "StorageClass" => ("storage.k8s.io", "v1", "StorageClass", false),
        _ => return None,
    };
    Some((GroupVersionKind::gvk(group, version, k), namespaced))
}

/// `k8s.getManifest` — return a resource's manifest as YAML.
pub fn get_manifest_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ManifestIn, ManifestOut, _, _>(
        "k8s.getManifest",
        "fetch a resource's manifest as YAML (any supported kind)",
        Annotations::READ_ONLY,
        move |input: ManifestIn| {
            let cache = cache.clone();
            async move {
                let (ar, namespaced) = resolve_api_resource(&input)?;
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<DynamicObject> = if namespaced {
                    let ns = input.namespace.as_deref().filter(|s| !s.is_empty()).unwrap_or("default");
                    Api::namespaced_with(client, ns, &ar)
                } else {
                    Api::all_with(client, &ar)
                };
                let mut obj = tokio::time::timeout(request_timeout(), api.get(&input.name))
                    .await
                    .map_err(|_| CapabilityError::Handler("get manifest timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                // Drop noisy server-managed fields for a readable manifest.
                obj.metadata.managed_fields = None;
                let yaml = serde_yaml::to_string(&obj)
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ManifestOut { yaml })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourceIn {
    pub context: String,
    /// Kubernetes Kind, e.g. "ConfigMap", "Job".
    pub kind: String,
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ResourceRow {
    pub name: String,
    pub namespace: String,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResourceOut {
    pub items: Vec<ResourceRow>,
}

/// `k8s.listResource` — list any supported kind generically (name + namespace).
pub fn list_resource_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListResourceIn, ListResourceOut, _, _>(
        "k8s.listResource",
        "list any supported resource kind (name + namespace)",
        Annotations::READ_ONLY,
        move |input: ListResourceIn| {
            let cache = cache.clone();
            async move {
                let (gvk, namespaced) = gvk_for(&input.kind)
                    .ok_or_else(|| CapabilityError::Handler(format!("unsupported kind: {}", input.kind)))?;
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let ar = ApiResource::from_gvk(&gvk);
                let ns = input.namespace.as_deref().unwrap_or("");
                // Empty namespace on a namespaced kind => all namespaces.
                let api: Api<DynamicObject> = if namespaced && !ns.is_empty() {
                    Api::namespaced_with(client, ns, &ar)
                } else {
                    Api::all_with(client, &ar)
                };
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list resource timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let items = list
                    .items
                    .into_iter()
                    .map(|o| ResourceRow {
                        name: o.metadata.name.unwrap_or_default(),
                        namespace: o.metadata.namespace.unwrap_or_default(),
                        age: crate::humanize_age(o.metadata.creation_timestamp.as_ref()),
                    })
                    .collect();
                Ok(ListResourceOut { items })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApplyIn {
    pub context: String,
    /// One or more resource manifests as YAML (documents separated by `---`).
    pub yaml: String,
    /// Force apply, taking ownership of fields held by other managers.
    #[serde(default)]
    pub force: bool,
}

/// Fields owned by other managers that block a non-forced apply.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Conflict {
    pub managers: Vec<String>,
    pub fields: Vec<String>,
    pub message: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ApplyDoc {
    pub kind: String,
    pub name: String,
    pub applied: bool,
    pub conflict: Option<Conflict>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ApplyOut {
    pub documents: Vec<ApplyDoc>,
    /// True only when every document applied with no conflict or error.
    pub applied: bool,
}

/// Best-effort parse of an apiserver SSA conflict message into managers
/// (quoted names) and field paths (leading-dot tokens).
pub fn parse_conflict(message: &str) -> Conflict {
    // Managers are the quoted names in the "conflict(s) with \"…\"" clause, up
    // to the ':' that ends that clause (e.g. `conflicts with "kubectl" and
    // "helm" using v1: .spec.replicas`). Scoping to this region avoids
    // mislabeling a quoted resource name/value elsewhere in the message as a
    // manager.
    let mut managers = Vec::new();
    if let Some(w) = message.find("with \"") {
        let region_end = message[w..]
            .find(':')
            .map(|i| w + i)
            .unwrap_or(message.len());
        let region = &message[w..region_end];
        let mut i = 0;
        while let Some(s) = region[i..].find('"') {
            let abs = i + s + 1;
            if let Some(e) = region[abs..].find('"') {
                managers.push(region[abs..abs + e].to_string());
                i = abs + e + 1;
            } else {
                break;
            }
        }
    }
    let fields = message
        .split_whitespace()
        .map(|t| t.trim_start_matches('-').trim())
        .filter(|t| t.starts_with('.') && t.len() > 1)
        .map(String::from)
        .collect();
    Conflict {
        managers,
        fields,
        message: message.to_string(),
    }
}

/// Split an `apiVersion` into (group, version). Core resources ("v1") have an
/// empty group.
pub fn parse_api_version(api_version: &str) -> (String, String) {
    match api_version.split_once('/') {
        Some((group, version)) => (group.to_string(), version.to_string()),
        None => (String::new(), api_version.to_string()),
    }
}

/// True only when there is at least one document and every one applied.
fn overall_applied(docs: &[ApplyDoc]) -> bool {
    !docs.is_empty() && docs.iter().all(|d| d.applied)
}

/// `k8s.applyManifest` — server-side apply one or more YAML documents with field
/// manager `srelens`. Non-forcing by default: field conflicts come back as
/// structured `Conflict` data (per document) rather than a raw 409.
pub fn apply_manifest_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ApplyIn, ApplyOut, _, _>(
        "k8s.applyManifest",
        "server-side apply resource manifests (YAML, multi-doc); creates or updates",
        Annotations {
            read_only: false,
            destructive: false,
            requires_confirm: true,
            sensitive: false,
        },
        move |input: ApplyIn| {
            let cache = cache.clone();
            async move {
                let docs = split_documents(&input.yaml)?;
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let mut documents = Vec::with_capacity(docs.len());
                for value in docs {
                    let r = match resource_ref(&value) {
                        Some(r) => r,
                        None => {
                            documents.push(ApplyDoc {
                                kind: String::new(),
                                name: String::new(),
                                applied: false,
                                conflict: None,
                                error: Some(
                                    "document missing apiVersion/kind/metadata.name".into(),
                                ),
                            });
                            continue;
                        }
                    };
                    let (group, version) = parse_api_version(&r.api_version);
                    let ar =
                        ApiResource::from_gvk(&GroupVersionKind::gvk(&group, &version, &r.kind));
                    let api: Api<DynamicObject> = match &r.namespace {
                        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
                        None => Api::all_with(client.clone(), &ar),
                    };
                    let mut params = PatchParams::apply("srelens");
                    if input.force {
                        params = params.force();
                    }
                    let result = match tokio::time::timeout(
                        request_timeout(),
                        api.patch(&r.name, &params, &Patch::Apply(&value)),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => {
                            documents.push(ApplyDoc {
                                kind: r.kind,
                                name: r.name,
                                applied: false,
                                conflict: None,
                                error: Some("apply timed out".into()),
                            });
                            continue;
                        }
                    };
                    let doc = apply_doc_from_result(r.kind, r.name, result.map(|_| ()));
                    documents.push(doc);
                }
                let applied = overall_applied(&documents);
                Ok(ApplyOut { documents, applied })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateIn {
    pub context: String,
    pub yaml: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidateOut {
    /// True when the API server accepts the manifest (dry-run).
    pub valid: bool,
    /// Validation errors (empty when valid).
    pub errors: Vec<ValidateError>,
}

/// A single validation error, tagged with the document it belongs to so the
/// editor can surface it against the right document in a multi-doc manifest.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidateError {
    /// Index of the document (in split order, skipping empty docs) the error
    /// belongs to. Serializes as `docIndex`.
    pub doc_index: usize,
    pub message: String,
}

/// Extract a clean, human message from a kube error — the API server's
/// `message` for API errors (dropping the `ErrorResponse {…}` debug noise),
/// and a plain string otherwise.
fn clean_kube_error(e: kube::Error) -> String {
    match e {
        kube::Error::Api(resp) if !resp.message.is_empty() => resp.message,
        other => other.to_string(),
    }
}

/// Validate one already-parsed document against the API server (dry-run,
/// strict field validation). `None` means the document has no
/// apiVersion/kind/metadata.name yet — nothing to validate, not an error.
/// `Some(Err(_))` carries the cleaned server error message.
async fn validate_document(
    client: &kube::Client,
    value: &serde_json::Value,
) -> Option<Result<(), String>> {
    let r = resource_ref(value)?;
    let (group, version) = parse_api_version(&r.api_version);
    let gvk = GroupVersionKind::gvk(&group, &version, &r.kind);
    let ar = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = match &r.namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };
    let params = PatchParams {
        field_manager: Some("srelens".into()),
        dry_run: true,
        force: true,
        field_validation: Some(ValidationDirective::Strict),
    };
    let result = match tokio::time::timeout(
        request_timeout(),
        api.patch(&r.name, &params, &Patch::Apply(value)),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => return Some(Err("validation timed out".into())),
    };
    Some(result.map(|_| ()).map_err(clean_kube_error))
}

/// Build the validation result from per-document outcomes. `None` = the
/// document had no identity yet (skipped, not a failure); `Some(Ok(()))` =
/// valid; a `Some(Err(message))` becomes a `ValidateError` tagged with its
/// doc index. Pure, so it is unit-testable independent of the API server.
fn aggregate_validation(results: Vec<(usize, Option<Result<(), String>>)>) -> ValidateOut {
    let mut errors = Vec::new();
    for (doc_index, r) in results {
        if let Some(Err(message)) = r {
            errors.push(ValidateError { doc_index, message });
        }
    }
    ValidateOut {
        valid: errors.is_empty(),
        errors,
    }
}

/// `k8s.validateManifest` — server-side dry-run apply with strict field
/// validation, one or more `---`-separated documents. Returns the API
/// server's real verdict (unknown fields, wrong types, invalid values,
/// admission errors) as data for every document, so the editor can surface
/// Kubernetes-aware diagnostics anywhere in a multi-document manifest — CRDs
/// included, no bundled schema needed. `valid` is true only when every
/// document validates (documents without enough identity to validate yet are
/// skipped, not treated as failures).
pub fn validate_manifest_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ValidateIn, ValidateOut, _, _>(
        "k8s.validateManifest",
        "validate a resource manifest against the API server (dry-run, strict)",
        Annotations::READ_ONLY,
        move |input: ValidateIn| {
            let cache = cache.clone();
            async move {
                let docs = match split_documents(&input.yaml) {
                    Ok(docs) => docs,
                    Err(e) => {
                        return Ok(ValidateOut {
                            valid: false,
                            errors: vec![ValidateError {
                                doc_index: 0,
                                message: e.to_string(),
                            }],
                        })
                    }
                };
                let mut client: Option<kube::Client> = None;
                let mut results: Vec<(usize, Option<Result<(), String>>)> = Vec::new();
                for (doc_index, value) in docs.iter().enumerate() {
                    if resource_ref(value).is_none() {
                        // Nothing to validate against the server yet; not an error.
                        results.push((doc_index, None));
                        continue;
                    }
                    if client.is_none() {
                        client = Some(
                            cache
                                .get(&input.context)
                                .await
                                .map_err(CapabilityError::Handler)?,
                        );
                    }
                    results.push((
                        doc_index,
                        validate_document(client.as_ref().unwrap(), value).await,
                    ));
                }
                Ok(aggregate_validation(results))
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffIn {
    pub context: String,
    pub yaml: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiffDoc {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
    /// False when the resource does not exist yet (a create).
    pub exists: bool,
    /// True when current and proposed differ.
    pub changed: bool,
    pub rows: Vec<DiffRow>,
    /// Live resourceVersion, for stale-edit detection (None when creating).
    /// Serializes as `currentResourceVersion` (camelCase) for the frontend.
    pub current_resource_version: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiffOut {
    pub documents: Vec<DiffDoc>,
}

/// Drop server-managed noise and status so the diff shows only meaningful
/// spec/metadata changes. Redacts Secret values when `is_secret`.
pub fn normalize_for_diff(value: &mut serde_json::Value, is_secret: bool) {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("status");
        if let Some(meta) = obj.get_mut("metadata").and_then(|m| m.as_object_mut()) {
            for k in [
                "managedFields",
                "resourceVersion",
                "generation",
                "uid",
                "creationTimestamp",
            ] {
                meta.remove(k);
            }
        }
    }
    if is_secret {
        crate::secrets::redact_secret_data(value);
    }
    canonicalize_diff_strings(value);
}

/// For diff display only: strip trailing whitespace from each line of every
/// multi-line string value, recursively. YAML block scalars (`|`) cannot carry
/// trailing spaces, so serde_yaml falls back to an escaped single-line quoted
/// string on whichever side has them — producing a spurious whole-block diff.
/// Canonicalizing both sides keeps equal multi-line strings in readable block
/// style and drops whitespace-only noise.
fn canonicalize_diff_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) if s.contains('\n') => {
            *s = s
                .split('\n')
                .map(|line| line.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(canonicalize_diff_strings),
        serde_json::Value::Object(o) => o.values_mut().for_each(canonicalize_diff_strings),
        _ => {}
    }
}

/// Build one document's diff from its already-fetched current + proposed JSON.
/// Pure (no I/O) so it is unit-testable; the capability supplies the fetched
/// values. `current_json` is `json!({})` when the resource does not exist.
fn diff_document(
    kind: String,
    name: String,
    namespace: Option<String>,
    exists: bool,
    current_resource_version: Option<String>,
    mut current_json: serde_json::Value,
    mut proposed_json: serde_json::Value,
    is_secret: bool,
) -> Result<DiffDoc, CapabilityError> {
    normalize_for_diff(&mut current_json, is_secret);
    normalize_for_diff(&mut proposed_json, is_secret);
    let current_yaml = if exists {
        serde_yaml::to_string(&current_json).map_err(|e| CapabilityError::Handler(e.to_string()))?
    } else {
        String::new()
    };
    let proposed_yaml = serde_yaml::to_string(&proposed_json)
        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
    let rows = align_rows(&current_yaml, &proposed_yaml);
    let changed = rows.iter().any(|row| row.tag != DiffTag::Same);
    Ok(DiffDoc {
        kind,
        name,
        namespace,
        exists,
        changed,
        rows,
        current_resource_version,
    })
}

/// `k8s.diffManifest` — for each document, diff the live object against the
/// server dry-run apply result (current | proposed) as aligned rows.
pub fn diff_manifest_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<DiffIn, DiffOut, _, _>(
        "k8s.diffManifest",
        "diff a manifest against the cluster via server dry-run apply (per document)",
        Annotations {
            read_only: true,
            destructive: false,
            requires_confirm: false,
            sensitive: true,
        },
        move |input: DiffIn| {
            let cache = cache.clone();
            async move {
                let docs = split_documents(&input.yaml)?;
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let mut documents = Vec::with_capacity(docs.len());
                for value in docs {
                    let r = match resource_ref(&value) {
                        Some(r) => r,
                        // Incomplete document (e.g. still being typed) — skip
                        // it rather than aborting the whole diff.
                        None => continue,
                    };
                    let (group, version) = parse_api_version(&r.api_version);
                    let gvk = GroupVersionKind::gvk(&group, &version, &r.kind);
                    let ar = ApiResource::from_gvk(&gvk);
                    let api: Api<DynamicObject> = match &r.namespace {
                        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
                        None => Api::all_with(client.clone(), &ar),
                    };
                    let is_secret = r.kind == "Secret" && group.is_empty();

                    // Current live object (may not exist).
                    let current = tokio::time::timeout(request_timeout(), api.get_opt(&r.name))
                        .await
                        .map_err(|_| CapabilityError::Handler("diff get timed out".into()))?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                    let current_resource_version = current
                        .as_ref()
                        .and_then(|o| o.metadata.resource_version.clone());
                    let exists = current.is_some();
                    let current_json = match current {
                        Some(o) => serde_json::to_value(o)
                            .map_err(|e| CapabilityError::Handler(e.to_string()))?,
                        None => serde_json::json!({}),
                    };

                    // Proposed = server dry-run apply (force to compute merged end-state).
                    let params = PatchParams {
                        field_manager: Some("srelens".into()),
                        dry_run: true,
                        force: true,
                        field_validation: None,
                    };
                    let proposed = tokio::time::timeout(
                        request_timeout(),
                        api.patch(&r.name, &params, &Patch::Apply(&value)),
                    )
                    .await
                    .map_err(|_| CapabilityError::Handler("diff dry-run timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(clean_kube_error(e)))?;
                    let proposed_json = serde_json::to_value(proposed)
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;

                    documents.push(diff_document(
                        r.kind,
                        r.name,
                        r.namespace,
                        exists,
                        current_resource_version,
                        current_json,
                        proposed_json,
                        is_secret,
                    )?);
                }
                Ok(DiffOut { documents })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_api_version_groups() {
        assert_eq!(parse_api_version("v1"), ("".to_string(), "v1".to_string()));
        assert_eq!(parse_api_version("apps/v1"), ("apps".to_string(), "v1".to_string()));
    }

    #[test]
    fn apply_capability_requires_confirm_and_is_not_read_only() {
        let cap = apply_manifest_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.applyManifest");
        assert!(cap.annotations.requires_confirm);
        assert!(!cap.annotations.read_only);
    }

    #[test]
    fn maps_known_kinds_with_scope() {
        let (_, ns) = gvk_for("Pod").unwrap();
        assert!(ns);
        let (gvk, ns) = gvk_for("Node").unwrap();
        assert!(!ns);
        assert_eq!(gvk.kind, "Node");
        let (gvk, _) = gvk_for("Deployment").unwrap();
        assert_eq!(gvk.group, "apps");
        assert!(gvk_for("Bogus").is_none());
    }

    #[test]
    fn capability_has_expected_id() {
        let cap = get_manifest_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.getManifest");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn splits_multiple_documents_skipping_empty() {
        let yaml = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: a
---

---
apiVersion: v1
kind: ConfigMap
metadata:
  name: b
";
        let docs = split_documents(yaml).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0]["metadata"]["name"], "a");
        assert_eq!(docs[1]["metadata"]["name"], "b");
    }

    #[test]
    fn resource_ref_pulls_identity() {
        let value: serde_json::Value = serde_yaml::from_str(
            "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: prod\n",
        )
        .unwrap();
        let r = resource_ref(&value).unwrap();
        assert_eq!(r.api_version, "apps/v1");
        assert_eq!(r.kind, "Deployment");
        assert_eq!(r.name, "web");
        assert_eq!(r.namespace.as_deref(), Some("prod"));
    }

    #[test]
    fn align_rows_pairs_a_changed_line() {
        let rows = align_rows("spec:\n  replicas: 3\n", "spec:\n  replicas: 5\n");
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0].tag, DiffTag::Same));
        assert_eq!(rows[0].left.as_deref(), Some("spec:"));
        assert!(matches!(rows[1].tag, DiffTag::Replace));
        assert_eq!(rows[1].left.as_deref(), Some("  replicas: 3"));
        assert_eq!(rows[1].right.as_deref(), Some("  replicas: 5"));
    }

    #[test]
    fn align_rows_marks_pure_insert_and_delete() {
        let rows = align_rows("a\n", "a\nb\n");
        assert!(matches!(rows[1].tag, DiffTag::Insert));
        assert_eq!(rows[1].left, None);
        assert_eq!(rows[1].right.as_deref(), Some("b"));

        let rows = align_rows("a\nb\n", "a\n");
        assert!(matches!(rows[1].tag, DiffTag::Delete));
        assert_eq!(rows[1].left.as_deref(), Some("b"));
        assert_eq!(rows[1].right, None);
    }

    #[test]
    fn normalize_for_diff_strips_noise_and_status() {
        let mut v: serde_json::Value = serde_yaml::from_str(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\n  resourceVersion: \"7\"\n  managedFields: [x]\n  uid: u\nstatus:\n  phase: Running\ndata:\n  k: v\n",
        ).unwrap();
        normalize_for_diff(&mut v, false);
        assert!(v.get("status").is_none());
        assert!(v["metadata"].get("resourceVersion").is_none());
        assert!(v["metadata"].get("managedFields").is_none());
        assert!(v["metadata"].get("uid").is_none());
        assert_eq!(v["data"]["k"], "v");
    }

    #[test]
    fn normalize_for_diff_redacts_secret_values() {
        let mut v: serde_json::Value = serde_yaml::from_str(
            "apiVersion: v1\nkind: Secret\nmetadata:\n  name: s\ndata:\n  token: c2VjcmV0\n",
        )
        .unwrap();
        normalize_for_diff(&mut v, true);
        assert_ne!(v["data"]["token"], "c2VjcmV0");
    }

    #[test]
    fn canonicalize_strips_trailing_line_whitespace() {
        let mut v = serde_json::json!({ "args": ["# c \nmkdir -p /app \nwhile true; do sleep 30; done;\n"] });
        canonicalize_diff_strings(&mut v);
        assert_eq!(
            v["args"][0],
            "# c\nmkdir -p /app\nwhile true; do sleep 30; done;\n"
        );
    }

    #[test]
    fn normalize_makes_multiline_serialize_as_block_not_quoted() {
        // Same logical script; one side has trailing spaces (would force serde_yaml
        // into escaped quoted style), the other is clean. After normalize_for_diff,
        // both must serialize identically in readable block style — no spurious diff.
        let mut clean = serde_json::json!({ "args": ["# c\nmkdir -p /app\ndone;\n"] });
        let mut spaced = serde_json::json!({ "args": ["# c \nmkdir -p /app \ndone;\n"] });
        normalize_for_diff(&mut clean, false);
        normalize_for_diff(&mut spaced, false);
        let cy = serde_yaml::to_string(&clean).unwrap();
        let sy = serde_yaml::to_string(&spaced).unwrap();
        assert_eq!(cy, sy);
        assert!(cy.contains("- |"), "expected block scalar, got:\n{cy}");
        // And align_rows over the two now shows no changes.
        let rows = align_rows(&cy, &sy);
        assert!(rows.iter().all(|r| matches!(r.tag, DiffTag::Same)));
    }

    #[test]
    fn diff_capability_is_read_only() {
        let cap = diff_manifest_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.diffManifest");
        assert!(cap.annotations.read_only);
        assert!(cap.annotations.sensitive);
    }

    #[test]
    fn parse_conflict_extracts_managers_and_fields() {
        let msg = "Apply failed with 2 conflicts: conflicts with \"kubectl\" using apps/v1:\n- .spec.replicas\n- .spec.template.spec.containers[0].image";
        let c = parse_conflict(msg);
        assert!(c.managers.contains(&"kubectl".to_string()));
        assert!(c.fields.iter().any(|f| f == ".spec.replicas"));
        assert!(c
            .fields
            .iter()
            .any(|f| f == ".spec.template.spec.containers[0].image"));
        assert_eq!(c.message, msg);
    }

    #[test]
    fn parse_conflict_scopes_managers_to_the_with_clause() {
        // Two managers in the "conflicts with ... and ..." clause, plus a
        // quoted value AFTER the clause-ending ':' that must NOT be picked up
        // as a manager.
        let msg = "Apply failed with 2 conflicts: conflicts with \"kubectl\" and \"helm\" using apps/v1:\n- .spec.replicas\n- .metadata.labels.\"app\": \"web\"";
        let c = parse_conflict(msg);
        assert!(c.managers.contains(&"kubectl".to_string()));
        assert!(c.managers.contains(&"helm".to_string()));
        assert_eq!(c.managers.len(), 2);
        assert!(!c.managers.contains(&"web".to_string()));
        assert!(!c.managers.contains(&"app".to_string()));
        assert_eq!(c.message, msg);
    }

    #[test]
    fn overall_applied_requires_nonempty_all_applied() {
        assert!(!overall_applied(&[]));
        let ok = ApplyDoc {
            kind: "Pod".into(),
            name: "a".into(),
            applied: true,
            conflict: None,
            error: None,
        };
        let bad = ApplyDoc {
            kind: "Pod".into(),
            name: "b".into(),
            applied: false,
            conflict: None,
            error: Some("x".into()),
        };
        assert!(overall_applied(std::slice::from_ref(&ok)));
        assert!(!overall_applied(&[ok, bad]));
    }

    #[test]
    fn apply_in_defaults_force_false() {
        let input: ApplyIn = serde_json::from_value(serde_json::json!({
            "context": "c", "yaml": "kind: Pod"
        }))
        .unwrap();
        assert!(!input.force);
    }

    // -- aggregate_validation -----------------------------------------------

    #[test]
    fn aggregate_validation_empty_input_is_valid() {
        let out = aggregate_validation(vec![]);
        assert!(out.valid);
        assert!(out.errors.is_empty());
    }

    #[test]
    fn aggregate_validation_identityless_and_valid_docs_are_valid() {
        let out = aggregate_validation(vec![(0, None), (1, Some(Ok(())))]);
        assert!(out.valid);
        assert!(out.errors.is_empty());
    }

    #[test]
    fn aggregate_validation_tags_error_with_doc_index() {
        let out = aggregate_validation(vec![(0, Some(Ok(()))), (2, Some(Err("boom".to_string())))]);
        assert!(!out.valid);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].doc_index, 2);
        assert_eq!(out.errors[0].message, "boom");
    }

    // -- diff_document -----------------------------------------------------

    #[test]
    fn diff_document_marks_changed_replica_update() {
        let current = serde_json::json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "web" },
            "spec": { "replicas": 3 }
        });
        let proposed = serde_json::json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": "web" },
            "spec": { "replicas": 5 }
        });
        let doc = diff_document(
            "Deployment".into(),
            "web".into(),
            None,
            true,
            Some("42".into()),
            current,
            proposed,
            false,
        )
        .unwrap();
        assert!(doc.exists);
        assert!(doc.changed);
        assert_eq!(doc.current_resource_version.as_deref(), Some("42"));
        assert!(doc.rows.iter().any(|r| matches!(r.tag, DiffTag::Replace)));
    }

    #[test]
    fn diff_document_handles_create_with_empty_current() {
        let proposed = serde_json::json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": { "name": "cfg" },
            "data": { "k": "v" }
        });
        let doc = diff_document(
            "ConfigMap".into(),
            "cfg".into(),
            None,
            false,
            None,
            serde_json::json!({}),
            proposed,
            false,
        )
        .unwrap();
        assert!(!doc.exists);
        assert!(doc.changed);
        assert!(doc.current_resource_version.is_none());
        // The current side is empty, so every row is an insert (or the doc
        // has no "same" rows at all since there's nothing to align against).
        assert!(doc.rows.iter().all(|r| r.left.is_none()));
    }

    #[test]
    fn diff_document_redacts_secret_values_on_both_sides() {
        let current = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": { "name": "s" },
            "data": { "token": "c2VjcmV0" }
        });
        let proposed = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": { "name": "s" },
            "data": { "token": "bmV3c2VjcmV0" }
        });
        let doc = diff_document(
            "Secret".into(),
            "s".into(),
            None,
            true,
            None,
            current,
            proposed,
            true,
        )
        .unwrap();
        for row in &doc.rows {
            if let Some(left) = &row.left {
                assert!(!left.contains("c2VjcmV0"));
            }
            if let Some(right) = &row.right {
                assert!(!right.contains("bmV3c2VjcmV0"));
            }
        }
    }

    // -- apply_doc_from_result ----------------------------------------------

    #[test]
    fn apply_doc_from_result_ok_is_applied() {
        let doc = apply_doc_from_result("Pod".into(), "web".into(), Ok(()));
        assert!(doc.applied);
        assert!(doc.conflict.is_none());
        assert!(doc.error.is_none());
    }

    #[test]
    fn apply_doc_from_result_409_becomes_conflict() {
        let err = kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".into(),
            message: "conflict with \"kubectl\": .spec.replicas".into(),
            reason: "Conflict".into(),
            code: 409,
        });
        let doc = apply_doc_from_result("Deployment".into(), "web".into(), Err(err));
        assert!(!doc.applied);
        assert!(doc.error.is_none());
        let conflict = doc.conflict.expect("expected a conflict");
        assert!(conflict.managers.contains(&"kubectl".to_string()));
    }

    #[test]
    fn apply_doc_from_result_other_error_is_cleaned() {
        let err = kube::Error::Api(kube::core::ErrorResponse {
            status: "Failure".into(),
            message: "boom".into(),
            reason: "InternalError".into(),
            code: 500,
        });
        let doc = apply_doc_from_result("Deployment".into(), "web".into(), Err(err));
        assert!(!doc.applied);
        assert!(doc.conflict.is_none());
        assert!(doc.error.as_deref().unwrap_or_default().contains("boom"));
    }

    // -- gvk_for extra arms ---------------------------------------------------

    #[test]
    fn gvk_for_covers_more_kinds() {
        let (gvk, ns) = gvk_for("Service").unwrap();
        assert_eq!(gvk.kind, "Service");
        assert!(ns);
        let (gvk, ns) = gvk_for("Ingress").unwrap();
        assert_eq!(gvk.group, "networking.k8s.io");
        assert!(ns);
        let (gvk, ns) = gvk_for("ClusterRole").unwrap();
        assert_eq!(gvk.group, "rbac.authorization.k8s.io");
        assert!(!ns);
        let (gvk, ns) = gvk_for("StorageClass").unwrap();
        assert_eq!(gvk.group, "storage.k8s.io");
        assert!(!ns);
        let (gvk, ns) = gvk_for("HorizontalPodAutoscaler").unwrap();
        assert_eq!(gvk.group, "autoscaling");
        assert!(ns);
        let (gvk, ns) = gvk_for("EndpointSlice").unwrap();
        assert_eq!(gvk.group, "discovery.k8s.io");
        assert!(ns);
        let (gvk, ns) = gvk_for("MutatingWebhookConfiguration").unwrap();
        assert_eq!(gvk.group, "admissionregistration.k8s.io");
        assert!(!ns);
    }

    // -- resolve_api_resource --------------------------------------------------

    #[test]
    fn resolve_api_resource_uses_static_table_when_no_crd_fields() {
        let input = ManifestIn {
            context: "c".into(),
            kind: "Deployment".into(),
            namespace: Some("prod".into()),
            name: "web".into(),
            group: None,
            version: None,
            plural: None,
        };
        let (ar, namespaced) = resolve_api_resource(&input).unwrap();
        assert_eq!(ar.group, "apps");
        assert_eq!(ar.kind, "Deployment");
        assert!(namespaced);
    }

    #[test]
    fn resolve_api_resource_uses_dynamic_crd_fields_when_supplied() {
        let input = ManifestIn {
            context: "c".into(),
            kind: "Widget".into(),
            namespace: Some("prod".into()),
            name: "w1".into(),
            group: Some("example.com".into()),
            version: Some("v1alpha1".into()),
            plural: Some("widgets".into()),
        };
        let (ar, namespaced) = resolve_api_resource(&input).unwrap();
        assert_eq!(ar.group, "example.com");
        assert_eq!(ar.version, "v1alpha1");
        assert_eq!(ar.kind, "Widget");
        assert_eq!(ar.plural, "widgets");
        assert!(namespaced);
    }

    #[test]
    fn resolve_api_resource_cluster_scoped_when_namespace_empty_for_crd() {
        let input = ManifestIn {
            context: "c".into(),
            kind: "Widget".into(),
            namespace: None,
            name: "w1".into(),
            group: Some("example.com".into()),
            version: Some("v1alpha1".into()),
            plural: Some("widgets".into()),
        };
        let (_, namespaced) = resolve_api_resource(&input).unwrap();
        assert!(!namespaced);
    }

    #[test]
    fn resolve_api_resource_rejects_unsupported_kind() {
        let input = ManifestIn {
            context: "c".into(),
            kind: "Bogus".into(),
            namespace: None,
            name: "x".into(),
            group: None,
            version: None,
            plural: None,
        };
        assert!(resolve_api_resource(&input).is_err());
    }

    // -- parse_api_version edge cases ------------------------------------------

    #[test]
    fn parse_api_version_handles_multi_segment_group() {
        // Only the first "/" is a separator; the rest of the string is the version.
        let (group, version) = parse_api_version("networking.k8s.io/v1");
        assert_eq!(group, "networking.k8s.io");
        assert_eq!(version, "v1");
    }

    #[test]
    fn parse_api_version_handles_empty_string() {
        let (group, version) = parse_api_version("");
        assert_eq!(group, "");
        assert_eq!(version, "");
    }

    // -- align_rows additional cases --------------------------------------------

    #[test]
    fn align_rows_pairs_multi_line_replace_group() {
        let current = "a\nb\nc\n";
        let proposed = "a\nx\ny\nc\n";
        let rows = align_rows(current, proposed);
        // First and last rows are unchanged; the two middle lines pair up as
        // Replace rows (delete "b"/insert "x" paired, "c" stays Same-ish since
        // there's no leftover "y" to pair — it becomes a pure Insert).
        assert!(matches!(rows[0].tag, DiffTag::Same));
        assert!(rows
            .iter()
            .any(|r| matches!(r.tag, DiffTag::Replace) && r.left.as_deref() == Some("b")));
        assert!(matches!(rows.last().unwrap().tag, DiffTag::Same));
    }

    #[test]
    fn align_rows_handles_asymmetric_insert_delete_counts() {
        // Two deletes, one insert: one Replace pair, one leftover Delete.
        let rows = align_rows("a\nb\nc\n", "a\nz\n");
        let replace_count = rows
            .iter()
            .filter(|r| matches!(r.tag, DiffTag::Replace))
            .count();
        let delete_count = rows
            .iter()
            .filter(|r| matches!(r.tag, DiffTag::Delete))
            .count();
        assert_eq!(replace_count, 1);
        assert_eq!(delete_count, 1);

        // One delete, two inserts: one Replace pair, one leftover Insert.
        let rows = align_rows("a\nb\n", "a\ny\nz\n");
        let replace_count = rows
            .iter()
            .filter(|r| matches!(r.tag, DiffTag::Replace))
            .count();
        let insert_count = rows
            .iter()
            .filter(|r| matches!(r.tag, DiffTag::Insert))
            .count();
        assert_eq!(replace_count, 1);
        assert_eq!(insert_count, 1);
    }

    // -- normalize_for_diff on stringData --------------------------------------

    #[test]
    fn normalize_for_diff_redacts_string_data() {
        // redact_secret_data blanks BOTH the `data` map and the `stringData`
        // map so no Secret material leaks through the diff path — only the
        // keys survive (so the UI can still show which fields changed).
        let mut v: serde_json::Value = serde_yaml::from_str(
            "apiVersion: v1\nkind: Secret\nmetadata:\n  name: s\n  uid: u\nstringData:\n  password: hunter2\ndata:\n  token: c2VjcmV0\nstatus:\n  phase: Active\n",
        )
        .unwrap();
        normalize_for_diff(&mut v, true);
        assert!(v.get("status").is_none());
        assert!(v["metadata"].get("uid").is_none());
        assert_eq!(v["data"]["token"], "");
        assert!(v["stringData"]
            .as_object()
            .unwrap()
            .contains_key("password"));
        assert_eq!(v["stringData"]["password"], "");
    }
}
