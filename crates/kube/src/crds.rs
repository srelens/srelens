//! Custom Resource Definition discovery + dynamic listing, so the UI can browse
//! any installed CRD (Gateway API, cert-manager, …) without a static GVK table.

use std::sync::Arc;

use kube::api::{Api, DynamicObject, ListParams};
use kube::core::{ApiResource, GroupVersionKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListCrdsIn {
    pub context: String,
}

/// One column a CRD asks tools to display, from
/// `spec.versions[].additionalPrinterColumns` -- the same metadata `kubectl get`
/// renders. Without these a custom resource list can only show name/namespace/age,
/// because nothing else is common to every kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrinterColumn {
    /// Column heading, e.g. "Health".
    pub name: String,
    /// Restricted JSONPath into the resource, e.g. ".status.health".
    pub json_path: String,
    /// OpenAPI type: string, integer, number, boolean or date.
    #[serde(rename = "type", default)]
    pub column_type: String,
}

/// A discovered CustomResourceDefinition, enough to list its instances.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CrdDescriptor {
    /// Metadata name, e.g. "gateways.gateway.networking.k8s.io".
    pub name: String,
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub namespaced: bool,
    /// Every version this CRD serves, in declaration order.
    pub versions: Vec<String>,
    /// The version objects are stored as. Empty when the CRD names none.
    pub storage_version: String,
    /// Columns this CRD asks to have displayed, in declaration order.
    pub printer_columns: Vec<PrinterColumn>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListCrdsOut {
    pub crds: Vec<CrdDescriptor>,
}

fn handler_err(e: impl ToString) -> CapabilityError {
    CapabilityError::Handler(e.to_string())
}

/// The storage version, else the first served version, else the first.
fn chosen_version(spec: &serde_json::Value) -> Option<&serde_json::Value> {
    let versions = spec["versions"].as_array()?;
    versions
        .iter()
        .find(|v| v["storage"].as_bool().unwrap_or(false))
        .or_else(|| {
            versions
                .iter()
                .find(|v| v["served"].as_bool().unwrap_or(false))
        })
        .or_else(|| versions.first())
}

/// Choose the storage version, else the first served version, else the first.
fn pick_version(spec: &serde_json::Value) -> String {
    chosen_version(spec)
        .and_then(|v| v["name"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// Every version this CRD serves, in the order it declares them.
fn served_versions(spec: &serde_json::Value) -> Vec<String> {
    spec["versions"]
        .as_array()
        .map(|vs| {
            vs.iter()
                .filter(|v| v["served"].as_bool().unwrap_or(false))
                .filter_map(|v| v["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The one version objects are persisted as. Empty when the CRD names none.
fn storage_version(spec: &serde_json::Value) -> String {
    spec["versions"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["storage"].as_bool().unwrap_or(false)))
        .and_then(|v| v["name"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// The printer columns worth showing for the chosen version.
///
/// Drops two kinds that would only add noise: `priority > 0` is `kubectl -o
/// wide` territory, and a column reading `.metadata.creationTimestamp` merely
/// duplicates the Age column every list already renders.
fn printer_columns(spec: &serde_json::Value) -> Vec<PrinterColumn> {
    let Some(version) = chosen_version(spec) else {
        return Vec::new();
    };
    version_printer_columns(version)
}
fn version_printer_columns(version: &serde_json::Value) -> Vec<PrinterColumn> {
    let Some(columns) = version["additionalPrinterColumns"].as_array() else {
        return Vec::new();
    };
    columns
        .iter()
        .filter(|c| c["priority"].as_i64().unwrap_or(0) == 0)
        .filter_map(|c| {
            let json_path = c["jsonPath"].as_str()?.to_string();
            if json_path == ".metadata.creationTimestamp" {
                return None;
            }
            Some(PrinterColumn {
                name: c["name"].as_str()?.to_string(),
                json_path,
                column_type: c["type"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Read a CRD `jsonPath` out of a resource, rendering the leaf as display text.
///
/// CRDs use a small JSONPath subset rather than the full grammar, so this walks
/// it directly instead of pulling in an engine. Supported segments:
///
/// - `.foo`, `.a\.b` and `['foo']` / `["foo"]` — object keys; both the escape
///   and the bracket form let a key contain dots, as label keys do
/// - `[0]` — array index
/// - `[?(@.type=="Ready")]` — first array element whose field equals a literal,
///   which is how Flux, cert-manager and most operators surface a condition
///
/// Anything absent, null, or not a scalar renders empty — an empty cell reads
/// better than a blob of JSON.
/// Restricted scalar projection shared by CRD printer columns and host-owned app columns.
pub fn resolve_json_path(value: &serde_json::Value, path: &str) -> String {
    let mut current = value;
    let mut rest = path.trim_start_matches('.');
    while !rest.is_empty() {
        let (segment, remainder) = match rest.strip_prefix('[') {
            Some(open) => {
                let Some(close) = open.find(']') else {
                    return String::new();
                };
                (
                    Segment::bracket(&open[..close]),
                    open[close + 1..].trim_start_matches('.'),
                )
            }
            None => {
                // Scan to the next *unescaped* separator. `\.` keeps a literal dot
                // inside the key, which is how a CRD can address a label such as
                // `app\.kubernetes\.io/name` without bracket notation.
                let mut key = String::new();
                let mut end = rest.len();
                let mut chars = rest.char_indices();
                while let Some((index, ch)) = chars.next() {
                    match ch {
                        '\\' => {
                            if let Some((_, escaped)) = chars.next() {
                                key.push(escaped);
                            }
                        }
                        '.' | '[' => {
                            end = index;
                            break;
                        }
                        _ => key.push(ch),
                    }
                }
                (Segment::Key(key), rest[end..].trim_start_matches('.'))
            }
        };
        let Some(next) = segment.apply(current) else {
            return String::new();
        };
        current = next;
        rest = remainder;
    }
    match current {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// One step of a CRD jsonPath.
enum Segment {
    Key(String),
    Index(usize),
    /// `[?(@.field=="literal")]` — the first array element that matches.
    Filter {
        field: String,
        literal: String,
    },
}

impl Segment {
    /// Parse the inside of a `[...]`.
    fn bracket(inner: &str) -> Segment {
        let trimmed = inner.trim();
        if let Some(expression) = trimmed.strip_prefix("?(").and_then(|e| e.strip_suffix(')')) {
            if let Some((left, right)) = expression.split_once("==") {
                return Segment::Filter {
                    field: left
                        .trim()
                        .trim_start_matches('@')
                        .trim_start_matches('.')
                        .to_string(),
                    literal: unquote(right.trim()).to_string(),
                };
            }
        }
        if let Ok(index) = trimmed.parse::<usize>() {
            return Segment::Index(index);
        }
        Segment::Key(unquote(trimmed).to_string())
    }

    fn apply<'v>(&self, value: &'v serde_json::Value) -> Option<&'v serde_json::Value> {
        match self {
            Segment::Key(key) => value.get(key),
            Segment::Index(index) => value.get(index),
            Segment::Filter { field, literal } => value
                .as_array()?
                .iter()
                .find(|item| resolve_json_path(item, field) == *literal),
        }
    }
}

fn unquote(text: &str) -> &str {
    text.trim_matches(|c| c == '\'' || c == '"')
}

/// Put a DynamicObject back together as one JSON value.
///
/// `kube` splits an object across three places -- `types` (apiVersion, kind),
/// typed `metadata`, and everything else in `data` -- but a printer column
/// addresses the resource as the API serves it, so all three have to be
/// reunited or paths like `.metadata.labels[...]` and `.kind` resolve empty.
fn whole_object(object: &DynamicObject) -> serde_json::Value {
    let mut value = object.data.clone();
    let Some(map) = value.as_object_mut() else {
        return value;
    };
    if let Ok(metadata) = serde_json::to_value(&object.metadata) {
        map.insert("metadata".to_string(), metadata);
    }
    // TypeMeta serializes flat, as apiVersion + kind beside the other fields.
    if let Some(types) = object.types.as_ref() {
        if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(types) {
            map.extend(fields);
        }
    }
    value
}

/// The raw value behind a cell, where rendering it would lose ordering.
fn column_sort_key(object: &serde_json::Value, column: &PrinterColumn) -> String {
    if column.column_type == "date" {
        resolve_json_path(object, &column.json_path)
    } else {
        String::new()
    }
}

/// Render one cell, humanizing `type: date` columns the way ages are shown
/// elsewhere so a raw RFC 3339 timestamp never reaches the table.
fn render_column(object: &serde_json::Value, column: &PrinterColumn) -> String {
    let raw = resolve_json_path(object, &column.json_path);
    if column.column_type != "date" || raw.is_empty() {
        return raw;
    }
    match raw.parse::<k8s_openapi::jiff::Timestamp>() {
        Ok(ts) => crate::humanize_age(Some(&k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
            ts,
        ))),
        Err(_) => raw,
    }
}

/// `k8s.listCRDs` — discover installed CustomResourceDefinitions.
pub fn list_crds_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListCrdsIn, ListCrdsOut, _, _>(
        "k8s.listCRDs",
        "list installed CustomResourceDefinitions (group, kind, plural, scope)",
        Annotations::READ_ONLY,
        move |input: ListCrdsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let gvk =
                    GroupVersionKind::gvk("apiextensions.k8s.io", "v1", "CustomResourceDefinition");
                let ar = ApiResource::from_gvk(&gvk);
                let api: Api<DynamicObject> = Api::all_with(client, &ar);
                let list =
                    tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                        .await
                        .map_err(|_| CapabilityError::Handler("list CRDs timed out".into()))?
                        .map_err(handler_err)?;

                let mut crds: Vec<CrdDescriptor> = list
                    .items
                    .into_iter()
                    .filter_map(|o| {
                        let spec = &o.data["spec"];
                        let group = spec["group"].as_str().unwrap_or_default().to_string();
                        let kind = spec["names"]["kind"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        let plural = spec["names"]["plural"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        let namespaced =
                            spec["scope"].as_str().unwrap_or("Namespaced") == "Namespaced";
                        let version = pick_version(spec);
                        if group.is_empty()
                            || kind.is_empty()
                            || plural.is_empty()
                            || version.is_empty()
                        {
                            return None;
                        }
                        Some(CrdDescriptor {
                            name: o.metadata.name.unwrap_or_default(),
                            group,
                            version,
                            kind,
                            plural,
                            namespaced,
                            versions: served_versions(spec),
                            storage_version: storage_version(spec),
                            printer_columns: printer_columns(spec),
                        })
                    })
                    .collect();
                crds.sort_by(|a, b| (&a.group, &a.kind).cmp(&(&b.group, &b.kind)));
                Ok(ListCrdsOut { crds })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListCustomIn {
    #[serde(default, rename = "useCrdColumns")]
    pub use_crd_columns: bool,
    pub context: String,
    pub group: String,
    pub version: String,
    pub plural: String,
    pub kind: String,
    pub namespaced: bool,
    #[serde(default)]
    pub namespace: String,
    /// Columns to resolve per item, from the CRD's `additionalPrinterColumns`.
    /// Callers that omit these get just name/namespace/age, as before.
    #[serde(default)]
    pub printer_columns: Vec<PrinterColumn>,
    /// An app's status rules for this kind (#541), bound by the host from the
    /// manifest's `statusResolvers`; each row then carries its `status`.
    /// Evaluated here because this is where the whole object is: the rows
    /// that leave are summaries.
    #[serde(default)]
    pub status_rules: Vec<srelens_capability::status::StatusRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomRow {
    pub name: String,
    pub namespace: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
    /// Values for the requested printer columns, in the order they were asked
    /// for. Empty when none were requested.
    pub columns: Vec<String>,
    /// Unrendered values for the columns whose display text loses ordering
    /// information -- `type: date`, where timestamps 65 and 115 minutes old both
    /// render "1h" and would otherwise tie. Empty where the text sorts fine.
    pub sort_keys: Vec<String>,
    /// The resolved status, when the request carried status rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<srelens_capability::status::ResolvedStatus>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListCustomOut {
    #[serde(rename = "printerColumns", skip_serializing_if = "Option::is_none")]
    pub printer_columns: Option<Vec<PrinterColumn>>,
    #[serde(rename = "columnsError", skip_serializing_if = "Option::is_none")]
    pub columns_error: Option<String>,
    pub items: Vec<CustomRow>,
    /// True when the list was cut at [`crate::list_cap::APP_LIST_CAP`] and more
    /// resources remain on the API server (#609).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Build a dynamic ApiResource for an arbitrary CRD GVK + plural.
pub(crate) fn custom_api_resource(
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
) -> ApiResource {
    let api_version = if group.is_empty() {
        version.to_string()
    } else {
        format!("{group}/{version}")
    };
    ApiResource {
        group: group.to_string(),
        version: version.to_string(),
        api_version,
        kind: kind.to_string(),
        plural: plural.to_string(),
    }
}

async fn discover_columns(
    client: kube::Client,
    group: &str,
    plural: &str,
    version: &str,
) -> Result<Vec<PrinterColumn>, String> {
    let ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "apiextensions.k8s.io",
        "v1",
        "CustomResourceDefinition",
    ));
    let api: Api<DynamicObject> = Api::all_with(client, &ar);
    let crd = tokio::time::timeout(request_timeout(), api.get(&format!("{plural}.{group}")))
        .await
        .map_err(|_| "CRD column discovery timed out".to_string())?
        .map_err(|e| e.to_string())?;
    columns_for_named_version(&crd.data["spec"], version)
}

/// Whether a CustomResourceDefinition named `{plural}.{group}` serves `version` of that
/// group and plural, so a caller can tell a custom resource from a built-in or aggregated
/// API before reading it. A CRD that exists but does not serve the version is not enough:
/// the same group and plural at another version may be served by something else.
/// `Ok(false)` means only that the API server answered and no such CRD serves it; a failed
/// lookup is an error, never an absence.
pub async fn custom_resource_serves(
    client: kube::Client,
    group: &str,
    version: &str,
    plural: &str,
) -> Result<bool, String> {
    let ar = ApiResource::from_gvk(&GroupVersionKind::gvk(
        "apiextensions.k8s.io",
        "v1",
        "CustomResourceDefinition",
    ));
    let api: Api<DynamicObject> = Api::all_with(client, &ar);
    let found = tokio::time::timeout(request_timeout(), api.get_opt(&format!("{plural}.{group}")))
        .await
        .map_err(|_| "CustomResourceDefinition lookup timed out".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(found.is_some_and(|crd| crd_serves(&crd.data["spec"], group, version, plural)))
}
/// Whether a CRD `spec` declares this group and plural and serves this version.
fn crd_serves(spec: &serde_json::Value, group: &str, version: &str, plural: &str) -> bool {
    spec["group"] == group
        && spec["names"]["plural"] == plural
        && spec["versions"].as_array().is_some_and(|versions| {
            versions
                .iter()
                .any(|v| v["name"] == version && v["served"] == true)
        })
}
fn columns_for_named_version(
    spec: &serde_json::Value,
    version: &str,
) -> Result<Vec<PrinterColumn>, String> {
    let definition = spec["versions"]
        .as_array()
        .and_then(|versions| {
            versions
                .iter()
                .find(|v| v["name"] == version && v["served"] == true)
        })
        .ok_or_else(|| format!("CRD has no served version {version}"))?;
    Ok(version_printer_columns(definition))
}

/// `k8s.listCustomResource` — list instances of a CRD by its GVK + plural.
pub fn list_custom_resource_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListCustomIn, ListCustomOut, _, _>(
        "k8s.listCustomResource",
        "list instances of a custom resource by group/version/plural",
        Annotations::READ_ONLY,
        move |input: ListCustomIn| {
            let cache = cache.clone();
            async move {
                // Checked before the cluster is asked: a rule list the host
                // would refuse at install is a bad request, not a column of
                // "Unknown" rows that look like an answer.
                if !input.status_rules.is_empty() {
                    if let Some((path, why)) =
                        srelens_capability::status::rule_problems(&input.status_rules)
                            .into_iter()
                            .next()
                    {
                        return Err(CapabilityError::InvalidInput(format!(
                            "statusRules: {path}: {why}"
                        )));
                    }
                }
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let ar =
                    custom_api_resource(&input.group, &input.version, &input.kind, &input.plural);
                let api: Api<DynamicObject> = if input.namespaced && !input.namespace.is_empty() {
                    Api::namespaced_with(client.clone(), &input.namespace, &ar)
                } else {
                    Api::all_with(client.clone(), &ar)
                };
                // No outer timeout: `list_capped` spends `request_timeout()` on
                // each page, which is what that budget measures. Wrapping the
                // walk gave four pages one request's time, and a cluster large
                // enough to need paging was the one most likely to be cut off.
                // The error mapping — an API error's own words, a sentence of
                // ours only for a timeout — is `into_capability_error`'s.
                let (objects, truncated) =
                    crate::list_cap::list_capped(&api, ListParams::default())
                        .await
                        .map_err(|e| e.into_capability_error("list custom resource"))?;
                let (columns, columns_error) = if input.use_crd_columns {
                    match discover_columns(client, &input.group, &input.plural, &input.version)
                        .await
                    {
                        Ok(columns) => (columns, None),
                        Err(error) => (input.printer_columns, Some(error)),
                    }
                } else {
                    (input.printer_columns, None)
                };
                // Cap columns evaluated per row as well as those a binding may
                // declare (#609) — a CRD can declare more than the manifest cap.
                // Keep in sync with `MAX_PRINTER_COLUMNS` in plugin-host.
                const MAX_COLUMNS: usize = 32;
                let columns: Vec<_> = columns.into_iter().take(MAX_COLUMNS).collect();
                let printer_columns = input.use_crd_columns.then(|| columns.clone());
                let items = objects
                    .into_iter()
                    .map(|o| {
                        let object = (!columns.is_empty() || !input.status_rules.is_empty())
                            .then(|| whole_object(&o));
                        let (values, sort_keys) = match &object {
                            Some(object) if !columns.is_empty() => columns
                                .iter()
                                .map(|c| (render_column(object, c), column_sort_key(object, c)))
                                .unzip(),
                            _ => (Vec::new(), Vec::new()),
                        };
                        let status = object
                            .as_ref()
                            .filter(|_| !input.status_rules.is_empty())
                            .map(|object| {
                                srelens_capability::status::resolve_status(
                                    &input.status_rules,
                                    object,
                                )
                            });
                        CustomRow {
                            name: o.metadata.name.clone().unwrap_or_default(),
                            namespace: o.metadata.namespace.clone().unwrap_or_default(),
                            created: crate::creation_rfc3339(
                                o.metadata.creation_timestamp.as_ref(),
                            ),
                            age: crate::humanize_age(o.metadata.creation_timestamp.as_ref()),
                            columns: values,
                            sort_keys,
                            status,
                        }
                    })
                    .collect();
                Ok(ListCustomOut {
                    items,
                    printer_columns,
                    columns_error,
                    truncated,
                })
            }
        },
    )
}

/// Host-only join read. Raw resources stay in the broker and only resolved scalar cells
/// leave `extensions.resolveColumns`; an app's reader capability still returns summaries.
pub async fn list_custom_resource_join_objects(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    namespaced: bool,
) -> Result<(Vec<serde_json::Value>, bool), CapabilityError> {
    let client = cache.get(context).await.map_err(CapabilityError::Handler)?;
    let ar = custom_api_resource(group, version, kind, plural);
    let api: Api<DynamicObject> = if namespaced && !namespace.is_empty() {
        Api::namespaced_with(client, namespace, &ar)
    } else {
        Api::all_with(client, &ar)
    };
    let (objects, truncated) = crate::list_cap::list_capped(&api, ListParams::default())
        .await
        .map_err(|error| error.into_capability_error("list joined custom resources"))?;
    Ok((objects.iter().map(whole_object).collect(), truncated))
}

/// Host-only read of a built-in kind's row metadata, for app badges (#541).
///
/// `kind` is qualified (`apps/Deployment`, `/Pod`) and must be a built-in kind
/// this host knows in exactly that group. Only identity, labels, annotations
/// and owner references are kept: a badge without a join reads its row's
/// metadata and nothing else, so an app never reaches a spec or a status it
/// holds no reader for. Secrets are refused outright — their annotation values
/// are redacted on every ungated read, and a badge must not become a way
/// around that.
pub async fn list_builtin_metadata(
    cache: &ClientCache,
    context: &str,
    namespace: &str,
    kind: &str,
) -> Result<(Vec<serde_json::Value>, bool), CapabilityError> {
    let refuse = |why: &str| Err(CapabilityError::InvalidInput(format!("{kind}: {why}")));
    let Some((group, name)) = kind.split_once('/') else {
        return refuse("qualify a kind with its API group");
    };
    if group.is_empty() && name == "Secret" {
        return refuse("Secret metadata is never read for an app");
    }
    let Some((gvk, namespaced)) = crate::manifest::gvk_for(name) else {
        return refuse("not a built-in kind this host reads");
    };
    if gvk.group != group || gvk.kind != name {
        return refuse("not a built-in kind this host reads");
    }
    let client = cache.get(context).await.map_err(CapabilityError::Handler)?;
    let resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = if namespaced && !namespace.is_empty() {
        Api::namespaced_with(client, namespace, &resource)
    } else {
        Api::all_with(client, &resource)
    };
    let (objects, truncated) = crate::list_cap::list_capped(&api, ListParams::default())
        .await
        .map_err(|error| error.into_capability_error("list built-in resource metadata"))?;
    let metadata = objects
        .iter()
        .map(|object| {
            let meta = &object.metadata;
            serde_json::json!({"metadata": {
                "name": meta.name,
                "namespace": meta.namespace,
                "uid": meta.uid,
                "labels": meta.labels,
                "annotations": meta.annotations,
                "ownerReferences": meta.owner_references,
            }})
        })
        .collect();
    Ok((metadata, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn capabilities_have_ids() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(list_crds_capability(cache.clone()).id, "k8s.listCRDs");
        assert_eq!(
            list_custom_resource_capability(cache).id,
            "k8s.listCustomResource"
        );
    }

    #[tokio::test]
    async fn capabilities_reject_invalid_and_missing_contexts() {
        let cache = ClientCache::new(PathBuf::from("/x"));

        // 1. list_crds_capability
        let list_crds = list_crds_capability(cache.clone());
        let err = (list_crds.handler)(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::InvalidInput(_)
        ));

        let err = (list_crds.handler)(serde_json::json!({ "context": "missing-cluster" }))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::Handler(_)
        ));

        // 2. list_custom_resource_capability
        let list_custom = list_custom_resource_capability(cache);
        let err = (list_custom.handler)(serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::InvalidInput(_)
        ));

        let err = (list_custom.handler)(serde_json::json!({
            "context": "missing-cluster",
            "group": "example.com",
            "version": "v1",
            "plural": "widgets",
            "kind": "Widget",
            "namespaced": true
        }))
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            srelens_capability::CapabilityError::Handler(_)
        ));
    }

    #[test]
    fn reads_served_versions_in_order_and_the_storage_one() {
        let spec = serde_json::json!({
            "group": "example.com",
            "names": { "kind": "Widget", "plural": "widgets" },
            "scope": "Namespaced",
            "versions": [
                { "name": "v1beta1", "served": true,  "storage": false },
                { "name": "v1",      "served": true,  "storage": true  },
                { "name": "v1alpha1","served": false, "storage": false }
            ]
        });
        assert_eq!(served_versions(&spec), vec!["v1beta1", "v1"]);
        assert_eq!(storage_version(&spec), "v1");
    }

    #[test]
    fn picks_storage_version() {
        let spec = serde_json::json!({
            "versions": [
                {"name": "v1alpha1", "served": true, "storage": false},
                {"name": "v1beta1", "served": true, "storage": true},
            ]
        });
        assert_eq!(pick_version(&spec), "v1beta1");
    }

    fn obj() -> serde_json::Value {
        serde_json::json!({
            "spec": { "version": "4.1.2", "nodes": 3, "paused": false },
            "status": { "health": "GREEN", "conditions": [{"type": "Ready"}], "empty": null },
            "metadata": { "labels": { "app.kubernetes.io/name": "cassandra" } },
        })
    }

    #[test]
    fn resolves_dotted_paths_to_scalars() {
        assert_eq!(resolve_json_path(&obj(), ".status.health"), "GREEN");
        assert_eq!(resolve_json_path(&obj(), ".spec.version"), "4.1.2");
        // Numbers and bools render as text, not JSON-quoted.
        assert_eq!(resolve_json_path(&obj(), ".spec.nodes"), "3");
        assert_eq!(resolve_json_path(&obj(), ".spec.paused"), "false");
    }

    #[test]
    fn resolves_backslash_escaped_dots_in_key_names() {
        // A CRD may address a dotted label key without bracket notation.
        assert_eq!(
            resolve_json_path(&obj(), r".metadata.labels.app\.kubernetes\.io/name"),
            "cassandra"
        );
    }

    #[test]
    fn resolves_bracket_segments_for_keys_containing_dots() {
        assert_eq!(
            resolve_json_path(&obj(), ".metadata.labels['app.kubernetes.io/name']"),
            "cassandra"
        );
    }

    /// Flux's HelmRelease, cert-manager and many operators address a condition
    /// by type rather than by index, so the filter form is not exotic.
    fn fluxish() -> serde_json::Value {
        serde_json::json!({
            "status": { "conditions": [
                {"type": "Stalled", "status": "False", "message": "nope"},
                {"type": "Ready", "status": "True", "message": "Release reconciliation succeeded"},
            ]},
            "spec": { "ports": [{"port": 80}, {"port": 443}] },
        })
    }

    #[test]
    fn resolves_filter_expressions_by_field_equality() {
        assert_eq!(
            resolve_json_path(
                &fluxish(),
                ".status.conditions[?(@.type==\"Ready\")].status"
            ),
            "True"
        );
        assert_eq!(
            resolve_json_path(
                &fluxish(),
                ".status.conditions[?(@.type==\"Ready\")].message"
            ),
            "Release reconciliation succeeded"
        );
    }

    #[test]
    fn filter_expressions_accept_single_quotes_and_spacing() {
        assert_eq!(
            resolve_json_path(
                &fluxish(),
                ".status.conditions[?(@.type == 'Stalled')].status"
            ),
            "False"
        );
    }

    #[test]
    fn a_filter_matching_nothing_renders_empty() {
        assert_eq!(
            resolve_json_path(
                &fluxish(),
                ".status.conditions[?(@.type==\"Missing\")].status"
            ),
            ""
        );
    }

    #[test]
    fn resolves_numeric_array_indexes() {
        assert_eq!(resolve_json_path(&fluxish(), ".spec.ports[1].port"), "443");
        assert_eq!(resolve_json_path(&fluxish(), ".spec.ports[9].port"), "");
    }

    #[test]
    fn missing_null_and_non_scalar_paths_render_empty() {
        assert_eq!(resolve_json_path(&obj(), ".status.nope"), "");
        assert_eq!(resolve_json_path(&obj(), ".status.empty"), "");
        // kubectl renders arrays/objects poorly; an empty cell beats noise.
        assert_eq!(resolve_json_path(&obj(), ".status.conditions"), "");
        assert_eq!(resolve_json_path(&obj(), ".spec"), "");
    }

    fn spec_with_columns() -> serde_json::Value {
        serde_json::json!({
            "versions": [{
                "name": "v1", "served": true, "storage": true,
                "additionalPrinterColumns": [
                    {"name": "Health", "type": "string", "jsonPath": ".status.health"},
                    {"name": "Version", "type": "string", "jsonPath": ".spec.version"},
                    {"name": "Verbose", "type": "string", "jsonPath": ".spec.x", "priority": 1},
                    {"name": "Age", "type": "date", "jsonPath": ".metadata.creationTimestamp"},
                ],
            }]
        })
    }

    #[test]
    fn app_columns_use_the_requested_served_version_and_standard_filters() {
        let spec = serde_json::json!({"versions":[
            {"name":"v1","served":true,"additionalPrinterColumns":[{"name":"Source","jsonPath":".spec.sourceRef.name","type":"string"},{"name":"Age","jsonPath":".metadata.creationTimestamp","type":"date"},{"name":"Hidden","jsonPath":".status.secret","priority":1}]},
            {"name":"v2","served":true,"storage":true,"additionalPrinterColumns":[{"name":"Revision","jsonPath":".status.revision","type":"string"}]},
            {"name":"v3","served":true},
            {"name":"v4","served":false}
        ]});
        let columns = columns_for_named_version(&spec, "v1").unwrap();
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].name, "Source");
        assert_eq!(
            render_column(
                &serde_json::json!({"spec":{"sourceRef":{"name":"platform"}}}),
                &columns[0]
            ),
            "platform"
        );
        assert_eq!(
            columns_for_named_version(&spec, "v2").unwrap()[0].name,
            "Revision"
        );
        assert!(columns_for_named_version(&spec, "v3").unwrap().is_empty());
        assert!(columns_for_named_version(&spec, "v4").is_err());
        assert!(columns_for_named_version(&spec, "missing").is_err());
    }
    #[test]
    fn takes_printer_columns_from_the_chosen_version() {
        let cols = printer_columns(&spec_with_columns());
        // Priority > 0 is `kubectl -o wide` only, and the table already has its
        // own Age column, so neither should reach the UI.
        assert_eq!(
            cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["Health", "Version"]
        );
        assert_eq!(cols[0].json_path, ".status.health");
    }

    #[test]
    fn a_crd_without_printer_columns_yields_none() {
        let spec = serde_json::json!({"versions": [{"name": "v1", "storage": true}]});
        assert!(printer_columns(&spec).is_empty());
    }

    #[test]
    fn date_columns_render_as_an_age_not_a_timestamp() {
        let object = serde_json::json!({"status": {"since": "2020-01-01T00:00:00Z"}});
        let column = PrinterColumn {
            name: "Since".into(),
            json_path: ".status.since".into(),
            column_type: "date".into(),
        };
        let rendered = render_column(&object, &column);
        // Compact age (e.g. "6y"), never the raw RFC 3339 string.
        assert!(!rendered.contains("2020-01-01"), "got {rendered}");
        assert!(rendered.ends_with('y'), "got {rendered}");
    }

    #[test]
    fn date_columns_carry_the_raw_timestamp_for_sorting() {
        let object = serde_json::json!({
            "status": {"since": "2020-01-01T00:00:00Z", "health": "GREEN"}
        });
        let date = PrinterColumn {
            name: "Since".into(),
            json_path: ".status.since".into(),
            column_type: "date".into(),
        };
        let text = PrinterColumn {
            name: "Health".into(),
            json_path: ".status.health".into(),
            column_type: "string".into(),
        };
        // Two timestamps inside the same displayed unit render alike, so the
        // raw value has to travel alongside for the table to order them.
        assert_eq!(column_sort_key(&object, &date), "2020-01-01T00:00:00Z");
        // Text columns already sort by what is shown; no second value needed.
        assert_eq!(column_sort_key(&object, &text), "");
    }

    #[test]
    fn unparseable_date_falls_back_to_the_raw_value() {
        let object = serde_json::json!({"status": {"since": "not a date"}});
        let column = PrinterColumn {
            name: "Since".into(),
            json_path: ".status.since".into(),
            column_type: "date".into(),
        };
        assert_eq!(render_column(&object, &column), "not a date");
    }

    #[test]
    fn metadata_paths_resolve_even_though_kube_splits_metadata_out() {
        let object: DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "db.example.com/v1",
            "kind": "Cluster",
            "metadata": {"name": "c1", "labels": {"app.kubernetes.io/name": "cassandra"}},
            "spec": {"version": "4.1.2"},
        }))
        .expect("dynamic object");
        let whole = whole_object(&object);
        assert_eq!(
            resolve_json_path(&whole, ".metadata.labels['app.kubernetes.io/name']"),
            "cassandra"
        );
        assert_eq!(resolve_json_path(&whole, ".spec.version"), "4.1.2");
    }

    /// End-to-end over Flux's HelmRelease, which is where the empty Ready and
    /// Status cells were first seen: its columns are filter expressions, and it
    /// declares its own Age that would otherwise duplicate the built-in one.
    #[test]
    fn renders_flux_helmrelease_columns() {
        let spec = serde_json::json!({
            "versions": [{
                "name": "v2", "served": true, "storage": true,
                "additionalPrinterColumns": [
                    {"name": "Age", "type": "date", "jsonPath": ".metadata.creationTimestamp"},
                    {"name": "Ready", "type": "string",
                     "jsonPath": ".status.conditions[?(@.type==\"Ready\")].status"},
                    {"name": "Status", "type": "string",
                     "jsonPath": ".status.conditions[?(@.type==\"Ready\")].message"},
                ],
            }]
        });
        let columns = printer_columns(&spec);
        assert_eq!(
            columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["Ready", "Status"],
        );

        let release = serde_json::json!({
            "status": {"conditions": [
                {"type": "Released", "status": "True", "message": "Helm install succeeded"},
                {"type": "Ready", "status": "True", "message": "Release reconciliation succeeded"},
            ]}
        });
        let rendered: Vec<String> = columns.iter().map(|c| render_column(&release, c)).collect();
        assert_eq!(rendered, vec!["True", "Release reconciliation succeeded"]);
    }

    #[test]
    fn type_paths_resolve_even_though_kube_splits_them_out_too() {
        // kube keeps apiVersion/kind in `types`, separate from both `data` and
        // `metadata`, so a column addressing them needs all three put back.
        let object: DynamicObject = serde_json::from_value(serde_json::json!({
            "apiVersion": "db.example.com/v1",
            "kind": "Cluster",
            "metadata": {"name": "c1"},
            "spec": {"version": "4.1.2"},
        }))
        .expect("dynamic object");
        let whole = whole_object(&object);
        assert_eq!(resolve_json_path(&whole, ".kind"), "Cluster");
        assert_eq!(
            resolve_json_path(&whole, ".apiVersion"),
            "db.example.com/v1"
        );
        // The other two sources still resolve.
        assert_eq!(resolve_json_path(&whole, ".metadata.name"), "c1");
        assert_eq!(resolve_json_path(&whole, ".spec.version"), "4.1.2");
    }

    /// The whole-walk timeout this PR removed, re-created at the capability
    /// level: three pages that each answer inside the per-request budget but
    /// together outlast it. `k8s.listCustomResource` completes; a handler that
    /// wrapped the walk in one `request_timeout()` would have cut it off.
    #[tokio::test]
    async fn list_custom_resource_walks_pages_that_together_outlast_one_request_budget() {
        let _budget = crate::list_cap::test_support::hold_request_timeout(1);
        let per_page = std::time::Duration::from_millis(450);
        let widget = |name: &str| {
            serde_json::json!({"apiVersion":"example.io/v1","kind":"Widget",
                "metadata":{"name":name,"namespace":"default"}})
        };
        let page = |items: Vec<serde_json::Value>, next: Option<&str>| {
            serde_json::json!({"apiVersion":"example.io/v1","kind":"WidgetList",
                "metadata":{"continue":next},"items":items})
        };
        let (client, uris) = crate::list_cap::test_support::mock_slow_pages(
            vec![
                page(vec![widget("w1")], Some("p2")),
                page(vec![widget("w2")], Some("p3")),
                page(vec![widget("w3")], None),
            ],
            per_page,
        );
        let cache = ClientCache::new(PathBuf::from("/x"));
        cache.preload("fake", client).await;
        let capability = list_custom_resource_capability(cache);

        let started = std::time::Instant::now();
        let out = (capability.handler)(serde_json::json!({
            "context": "fake", "group": "example.io", "version": "v1",
            "plural": "widgets", "kind": "Widget", "namespaced": false
        }))
        .await
        .expect("every page answered inside the per-request budget");

        assert!(
            started.elapsed() > crate::connect::request_timeout(),
            "the walk must have outlasted one request's budget to prove anything"
        );
        // A false `truncated` is omitted on the wire (`skip_serializing_if`).
        assert_ne!(out["truncated"], true, "{out}");
        assert_eq!(out["items"].as_array().map(Vec::len), Some(3), "{out}");
        assert_eq!(uris.lock().unwrap().len(), 3);
    }

    #[test]
    fn builds_namespaced_api_version() {
        let ar = custom_api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
        assert_eq!(ar.api_version, "gateway.networking.k8s.io/v1");
        assert_eq!(ar.plural, "gateways");
    }

    /// A client whose API server answers every request with `status`, recording each path.
    fn answering(status: u16) -> (kube::Client, Arc<std::sync::Mutex<Vec<String>>>) {
        let paths = Arc::new(std::sync::Mutex::new(vec![]));
        let seen = paths.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            seen.lock().unwrap().push(request.uri().path().to_owned());
            async move {
                let body = if status == 200 {
                    serde_json::json!({"apiVersion":"apiextensions.k8s.io/v1",
                        "kind":"CustomResourceDefinition",
                        "metadata":{"name":"applications.argoproj.io"},
                        "spec":{"group":"argoproj.io","names":{"plural":"applications","kind":"Application"},
                            "scope":"Namespaced",
                            "versions":[{"name":"v1alpha1","served":true,"storage":true},
                                {"name":"v1beta1","served":false,"storage":false}]}})
                } else {
                    let reason = if status == 404 {
                        "NotFound"
                    } else {
                        "Forbidden"
                    };
                    serde_json::json!({"apiVersion":"v1","kind":"Status","status":"Failure",
                        "code":status,"reason":reason,"message":"rejected"})
                };
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        (kube::Client::new(service, "default"), paths)
    }

    #[tokio::test]
    async fn join_list_scopes_requests_and_reconstructs_complete_custom_resources() {
        let object = serde_json::json!({"apiVersion":"example.io/v1","kind":"Widget",
            "metadata":{"name":"report","namespace":"team","labels":{"target":"api"}},
            "report":{"critical":3}});
        let page = serde_json::json!({"apiVersion":"example.io/v1","kind":"WidgetList",
            "metadata":{},"items":[object]});
        for (namespace, namespaced, expected_path) in [
            ("team", true, "/apis/example.io/v1/namespaces/team/widgets"),
            ("team", false, "/apis/example.io/v1/widgets"),
        ] {
            let (client, uris) = crate::list_cap::test_support::mock_slow_pages(
                vec![page.clone()],
                std::time::Duration::ZERO,
            );
            let cache = ClientCache::new_many(vec![]);
            cache.preload("fake", client).await;
            let (objects, truncated) = list_custom_resource_join_objects(
                &cache,
                "fake",
                namespace,
                "example.io",
                "v1",
                "Widget",
                "widgets",
                namespaced,
            )
            .await
            .unwrap();
            assert!(!truncated);
            assert_eq!(objects.len(), 1);
            assert_eq!(objects[0]["apiVersion"], "example.io/v1");
            assert_eq!(objects[0]["kind"], "Widget");
            assert_eq!(objects[0]["metadata"]["labels"]["target"], "api");
            assert_eq!(objects[0]["report"]["critical"], 3);
            assert!(uris.lock().unwrap()[0].starts_with(expected_path));
        }
    }

    /// Three Flux Kustomizations as the API server lists them.
    fn kustomization_page() -> serde_json::Value {
        let item = |name: &str, suspend: bool, ready: &str, message: &str| {
            serde_json::json!({"apiVersion":"kustomize.toolkit.fluxcd.io/v1","kind":"Kustomization",
                "metadata":{"name":name,"namespace":"flux-system"},
                "spec":{"suspend":suspend},
                "status":{"conditions":[{"type":"Ready","status":ready,"message":message}]}})
        };
        serde_json::json!({"apiVersion":"kustomize.toolkit.fluxcd.io/v1","kind":"KustomizationList",
        "metadata":{},"items":[
            item("apps", false, "True", "Applied revision: main@sha1:abc"),
            item("infra", true, "True", "Applied"),
            item("broken", false, "False", "kustomize build failed"),
        ]})
    }

    /// The payload `extensions.read` sends once the host has bound a
    /// resolver's rules: the caller's camelCase spelling.
    fn kustomizations_with_rules() -> serde_json::Value {
        serde_json::json!({
            "context":"fake","group":"kustomize.toolkit.fluxcd.io","version":"v1",
            "plural":"kustomizations","kind":"Kustomization","namespaced":true,"namespace":"flux-system",
            "statusRules":[
                {"when":[{"jsonPath":".spec.suspend","equals":true}],"status":"suspended","label":"Suspended"},
                {"when":[{"jsonPath":".status.conditions[?(@.type==\"Ready\")].status","equals":"True"}],
                 "status":"healthy","label":"Ready"},
                {"when":[{"jsonPath":".status.conditions[?(@.type==\"Ready\")].status","equals":"False"}],
                 "status":"error","label":"Not ready","reason":".status.conditions[?(@.type==\"Ready\")].message"}
            ]
        })
    }

    #[tokio::test]
    async fn a_list_with_status_rules_resolves_each_row_against_the_whole_object() {
        let (client, _) = crate::list_cap::test_support::mock_slow_pages(
            vec![kustomization_page()],
            std::time::Duration::ZERO,
        );
        let cache = ClientCache::new_many(vec![]);
        cache.preload("fake", client).await;
        let list = list_custom_resource_capability(cache);
        let out = (list.handler)(kustomizations_with_rules()).await.unwrap();
        let statuses: Vec<_> = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["status"].clone())
            .collect();
        assert_eq!(
            statuses,
            vec![
                serde_json::json!({"status":"healthy","label":"Ready"}),
                serde_json::json!({"status":"suspended","label":"Suspended"}),
                serde_json::json!({"status":"error","label":"Not ready","reason":"kustomize build failed"}),
            ]
        );
    }

    #[tokio::test]
    async fn a_list_without_status_rules_carries_no_status() {
        let (client, _) = crate::list_cap::test_support::mock_slow_pages(
            vec![kustomization_page()],
            std::time::Duration::ZERO,
        );
        let cache = ClientCache::new_many(vec![]);
        cache.preload("fake", client).await;
        let mut payload = kustomizations_with_rules();
        payload.as_object_mut().unwrap().remove("statusRules");
        let out = (list_custom_resource_capability(cache).handler)(payload)
            .await
            .unwrap();
        assert!(out["items"][0].get("status").is_none(), "{out}");
    }

    #[tokio::test]
    async fn status_rules_the_host_would_refuse_are_invalid_input_not_unknown_rows() {
        let cache = ClientCache::new_many(vec![]);
        let mut payload = kustomizations_with_rules();
        payload["statusRules"][0]["when"][0]["jsonPath"] = serde_json::json!(".spec.*");
        let error = (list_custom_resource_capability(cache).handler)(payload)
            .await
            .unwrap_err();
        assert!(matches!(error, CapabilityError::InvalidInput(_)), "{error}");
    }

    #[tokio::test]
    async fn builtin_metadata_is_read_for_a_qualified_kind_and_nothing_else_of_it_is_kept() {
        let page = serde_json::json!({"apiVersion":"apps/v1","kind":"DeploymentList","metadata":{},
            "items":[{"apiVersion":"apps/v1","kind":"Deployment",
                "metadata":{"name":"api","namespace":"team","uid":"u1",
                    "labels":{"kustomize.toolkit.fluxcd.io/name":"apps"},
                    "annotations":{"argocd.argoproj.io/tracking-id":"guestbook:apps/Deployment:team/api"}},
                "spec":{"replicas":3,"template":{"spec":{"containers":[{"name":"api","image":"x"}]}}},
                "status":{"readyReplicas":3}}]});
        let (client, uris) =
            crate::list_cap::test_support::mock_slow_pages(vec![page], std::time::Duration::ZERO);
        let cache = ClientCache::new_many(vec![]);
        cache.preload("fake", client).await;
        let (objects, truncated) = list_builtin_metadata(&cache, "fake", "team", "apps/Deployment")
            .await
            .unwrap();
        assert!(!truncated);
        assert_eq!(
            uris.lock().unwrap()[0].split('?').next(),
            Some("/apis/apps/v1/namespaces/team/deployments")
        );
        assert_eq!(objects.len(), 1);
        assert_eq!(
            objects[0]["metadata"]["labels"]["kustomize.toolkit.fluxcd.io/name"],
            "apps"
        );
        assert_eq!(objects[0]["metadata"]["uid"], "u1");
        assert!(
            objects[0].get("spec").is_none() && objects[0].get("status").is_none(),
            "{}",
            objects[0]
        );
    }

    #[tokio::test]
    async fn builtin_metadata_refuses_secrets_unknown_kinds_and_a_group_that_does_not_match() {
        let cache = ClientCache::new_many(vec![]);
        for kind in [
            "/Secret",
            "acme.io/Widget",
            "acme.io/Deployment",
            "Deployment",
            "/Nope",
        ] {
            let error = list_builtin_metadata(&cache, "fake", "team", kind)
                .await
                .err()
                .unwrap_or_else(|| panic!("{kind} must be refused"));
            assert!(
                matches!(error, CapabilityError::InvalidInput(_)),
                "{kind}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn join_list_reports_truncation_at_the_shared_list_cap() {
        let page = |start: usize, next: Option<&str>| {
            serde_json::json!({
                "apiVersion":"example.io/v1","kind":"WidgetList",
                "metadata":{"continue":next},
                "items":(start..start+500).map(|i| serde_json::json!({
                    "apiVersion":"example.io/v1","kind":"Widget","metadata":{"name":format!("w{i}")}
                })).collect::<Vec<_>>(),
            })
        };
        let (client, uris) = crate::list_cap::test_support::mock_slow_pages(
            vec![
                page(0, Some("p2")),
                page(500, Some("p3")),
                page(1000, Some("p4")),
                page(1500, Some("p5")),
            ],
            std::time::Duration::ZERO,
        );
        let cache = ClientCache::new_many(vec![]);
        cache.preload("fake", client).await;
        let (objects, truncated) = list_custom_resource_join_objects(
            &cache,
            "fake",
            "",
            "example.io",
            "v1",
            "Widget",
            "widgets",
            false,
        )
        .await
        .unwrap();
        assert_eq!(objects.len(), crate::list_cap::APP_LIST_CAP);
        assert!(truncated);
        assert_eq!(uris.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn join_list_preserves_api_failure_as_an_error() {
        let (client, paths) = answering(403);
        let cache = ClientCache::new_many(vec![]);
        cache.preload("fake", client).await;
        let error = list_custom_resource_join_objects(
            &cache,
            "fake",
            "team",
            "example.io",
            "v1",
            "Widget",
            "widgets",
            true,
        )
        .await
        .err()
        .expect("Forbidden must not become an empty list");
        assert!(matches!(error, CapabilityError::Handler(_)));
        assert!(error.to_string().contains("Forbidden"), "{error}");
        assert_eq!(
            paths.lock().unwrap()[0],
            "/apis/example.io/v1/namespaces/team/widgets"
        );
    }

    #[tokio::test]
    async fn a_crd_lookup_checks_the_served_version_and_tells_absence_from_failure() {
        let (client, paths) = answering(200);
        assert_eq!(
            custom_resource_serves(client, "argoproj.io", "v1alpha1", "applications").await,
            Ok(true)
        );
        assert_eq!(
            paths.lock().unwrap().as_slice(),
            ["/apis/apiextensions.k8s.io/v1/customresourcedefinitions/applications.argoproj.io"]
        );
        // The CRD exists, but another API may serve this version of the group.
        let (client, _) = answering(200);
        assert_eq!(
            custom_resource_serves(client, "argoproj.io", "v1beta1", "applications").await,
            Ok(false)
        );
        let (client, _) = answering(200);
        assert_eq!(
            custom_resource_serves(client, "argoproj.io", "v1", "applications").await,
            Ok(false)
        );
        let (client, _) = answering(404);
        assert_eq!(
            custom_resource_serves(client, "apps", "v1", "deployments").await,
            Ok(false)
        );
        // Forbidden is not "no such CRD".
        let (client, _) = answering(403);
        assert!(
            custom_resource_serves(client, "argoproj.io", "v1alpha1", "applications")
                .await
                .is_err()
        );
    }

    #[test]
    fn a_crd_serves_only_its_own_group_plural_and_served_versions() {
        let spec = serde_json::json!({"group":"argoproj.io","names":{"plural":"applications"},
            "versions":[{"name":"v1alpha1","served":true},{"name":"v1beta1","served":false},
                {"name":"v1"}]});
        assert!(crd_serves(&spec, "argoproj.io", "v1alpha1", "applications"));
        assert!(!crd_serves(&spec, "argoproj.io", "v1beta1", "applications"));
        assert!(!crd_serves(&spec, "argoproj.io", "v1", "applications"));
        assert!(!crd_serves(&spec, "argoproj.io", "v2", "applications"));
        assert!(!crd_serves(&spec, "argoproj.io", "v1alpha1", "appprojects"));
        assert!(!crd_serves(&spec, "other.io", "v1alpha1", "applications"));
        assert!(!crd_serves(
            &serde_json::json!({}),
            "argoproj.io",
            "v1alpha1",
            "applications"
        ));
    }

    #[test]
    fn list_custom_out_omits_truncated_when_false() {
        let raw = serde_json::to_value(ListCustomOut {
            printer_columns: None,
            columns_error: None,
            items: vec![],
            truncated: false,
        })
        .unwrap();
        assert!(raw.get("truncated").is_none());
        let cut = serde_json::to_value(ListCustomOut {
            printer_columns: None,
            columns_error: None,
            items: vec![],
            truncated: true,
        })
        .unwrap();
        assert_eq!(cut["truncated"], true);
    }
}
