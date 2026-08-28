//! Helm 3 release capabilities. Helm stores each release revision as a Secret
//! (type `helm.sh/release.v1`, label `owner=helm`) whose `release` field is
//! `base64(gzip(json))`. We list those secrets, decode them, and expose release
//! summaries and details (values, manifest, history) — no Helm binary needed.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::Secret;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client_cache::ClientCache;

fn handler_err(e: impl ToString) -> CapabilityError {
    CapabilityError::Handler(e.to_string())
}

/// Decode a Helm release Secret's `release` field: `base64(gzip(json))`. Older
/// releases may be un-gzipped, so we sniff the gzip magic bytes.
pub fn decode_release(raw: &[u8]) -> Result<Value, String> {
    let decoded = STANDARD.decode(raw).map_err(|e| e.to_string())?;
    let bytes = if decoded.len() >= 2 && decoded[0] == 0x1f && decoded[1] == 0x8b {
        let mut gz = GzDecoder::new(&decoded[..]);
        let mut out = Vec::new();
        gz.read_to_end(&mut out).map_err(|e| e.to_string())?;
        out
    } else {
        decoded
    };
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn s(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        cur = match cur.get(p) {
            Some(next) => next,
            None => return String::new(),
        };
    }
    cur.as_str().unwrap_or("").to_string()
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelmReleaseSummary {
    pub name: String,
    pub namespace: String,
    pub revision: i64,
    pub status: String,
    pub chart: String,
    pub chart_version: String,
    pub app_version: String,
    pub updated: String,
}

/// Summarise a decoded release object into list-view fields.
pub fn summarise_release(v: &Value) -> HelmReleaseSummary {
    HelmReleaseSummary {
        name: s(v, &["name"]),
        namespace: s(v, &["namespace"]),
        revision: v.get("version").and_then(Value::as_i64).unwrap_or(0),
        status: s(v, &["info", "status"]),
        chart: s(v, &["chart", "metadata", "name"]),
        chart_version: s(v, &["chart", "metadata", "version"]),
        app_version: s(v, &["chart", "metadata", "appVersion"]),
        updated: s(v, &["info", "last_deployed"]),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListHelmReleasesIn {
    pub context: String,
    /// Namespace to scope to; empty/absent means all namespaces.
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListHelmReleasesOut {
    pub releases: Vec<HelmReleaseSummary>,
}

async fn list_release_secrets(
    cache: &Arc<ClientCache>,
    context: &str,
    namespace: &str,
    label: &str,
) -> Result<Vec<Secret>, CapabilityError> {
    let client = cache.get(context).await.map_err(CapabilityError::Handler)?;
    let api: Api<Secret> = if namespace.is_empty() {
        Api::all(client)
    } else {
        Api::namespaced(client, namespace)
    };
    let list = api
        .list(&ListParams::default().labels(label))
        .await
        .map_err(handler_err)?;
    Ok(list.items)
}

/// `k8s.listHelmReleases` — latest revision of each Helm release in scope.
pub fn list_helm_releases_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListHelmReleasesIn, ListHelmReleasesOut, _, _>(
        "k8s.listHelmReleases",
        "list installed Helm releases (latest revision of each)",
        Annotations::READ_ONLY,
        move |input: ListHelmReleasesIn| {
            let cache = cache.clone();
            async move {
                let ns = input.namespace.unwrap_or_default();
                let secrets = list_release_secrets(&cache, &input.context, &ns, "owner=helm").await?;
                // Keep the highest revision per (namespace, name).
                let mut latest: BTreeMap<(String, String), HelmReleaseSummary> = BTreeMap::new();
                for secret in &secrets {
                    let Some(raw) = secret.data.as_ref().and_then(|d| d.get("release")) else {
                        continue;
                    };
                    let Ok(rel) = decode_release(&raw.0) else { continue };
                    let sum = summarise_release(&rel);
                    let key = (sum.namespace.clone(), sum.name.clone());
                    match latest.get(&key) {
                        Some(existing) if existing.revision >= sum.revision => {}
                        _ => {
                            latest.insert(key, sum);
                        }
                    }
                }
                Ok(ListHelmReleasesOut {
                    releases: latest.into_values().collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetHelmReleaseIn {
    pub context: String,
    pub namespace: String,
    pub name: String,
    /// The revision to read. Omitted means the current one, exactly as
    /// before this field existed — every caller that doesn't know about
    /// revisions keeps getting what it always got.
    #[serde(default)]
    pub revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelmRevision {
    pub revision: i64,
    pub status: String,
    pub updated: String,
    pub chart_version: String,
    pub description: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelmReleaseDetail {
    pub name: String,
    pub namespace: String,
    pub revision: i64,
    pub status: String,
    pub chart: String,
    pub chart_version: String,
    pub app_version: String,
    pub updated: String,
    /// User-supplied values, rendered as YAML.
    pub values_yaml: String,
    /// The rendered manifest for the current revision.
    pub manifest: String,
    pub notes: String,
    /// All revisions, newest first.
    pub history: Vec<HelmRevision>,
}

/// Pick one revision's decoded release object out of a release's history
/// (`revisions` is newest first). `None` asks for the current revision —
/// `revisions[0]`, exactly what this capability returned before it could
/// be asked for anything else. `Some(n)` for a revision that isn't in the
/// list is an error, not a fallback to the current one: silently returning
/// the current revision would make a diff of two revisions compare a
/// manifest with itself and report no changes on a release that changed.
fn pick_revision(revisions: &[Value], requested: Option<i64>) -> Result<&Value, CapabilityError> {
    match requested {
        None => revisions
            .first()
            .ok_or_else(|| CapabilityError::Handler("no revisions available".to_string())),
        Some(rev) => revisions
            .iter()
            .find(|v| v.get("version").and_then(Value::as_i64) == Some(rev))
            .ok_or_else(|| CapabilityError::Handler(format!("revision {rev} not found"))),
    }
}

/// `k8s.getHelmRelease` — full detail of a release: values, manifest, history.
pub fn get_helm_release_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<GetHelmReleaseIn, HelmReleaseDetail, _, _>(
        "k8s.getHelmRelease",
        "fetch a Helm release's values, manifest, and revision history",
        Annotations::READ_ONLY,
        move |input: GetHelmReleaseIn| {
            let cache = cache.clone();
            async move {
                let label = format!("owner=helm,name={}", input.name);
                let secrets =
                    list_release_secrets(&cache, &input.context, &input.namespace, &label).await?;
                let mut revisions: Vec<Value> = secrets
                    .iter()
                    .filter_map(|s| s.data.as_ref().and_then(|d| d.get("release")))
                    .filter_map(|b| decode_release(&b.0).ok())
                    .collect();
                if revisions.is_empty() {
                    return Err(CapabilityError::Handler(format!(
                        "no Helm release named {} in {}",
                        input.name, input.namespace
                    )));
                }
                // Newest revision first.
                revisions.sort_by_key(|v| -v.get("version").and_then(Value::as_i64).unwrap_or(0));
                let history = revisions
                    .iter()
                    .map(|v| HelmRevision {
                        revision: v.get("version").and_then(Value::as_i64).unwrap_or(0),
                        status: s(v, &["info", "status"]),
                        updated: s(v, &["info", "last_deployed"]),
                        chart_version: s(v, &["chart", "metadata", "version"]),
                        description: s(v, &["info", "description"]),
                    })
                    .collect();

                let current = pick_revision(&revisions, input.revision)?;
                let sum = summarise_release(current);
                let values_yaml = match current.get("config") {
                    Some(cfg) if !cfg.is_null() => serde_yaml::to_string(cfg).unwrap_or_default(),
                    _ => String::new(),
                };
                Ok(HelmReleaseDetail {
                    name: sum.name,
                    namespace: sum.namespace,
                    revision: sum.revision,
                    status: sum.status,
                    chart: sum.chart,
                    chart_version: sum.chart_version,
                    app_version: sum.app_version,
                    updated: sum.updated,
                    values_yaml,
                    manifest: s(current, &["manifest"]),
                    notes: s(current, &["info", "notes"]),
                    history,
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn encode(json: &str) -> Vec<u8> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(json.as_bytes()).unwrap();
        let gzipped = gz.finish().unwrap();
        STANDARD.encode(gzipped).into_bytes()
    }

    #[test]
    fn decodes_gzipped_release() {
        let raw = encode(r#"{"name":"redis","version":3}"#);
        let v = decode_release(&raw).unwrap();
        assert_eq!(v["name"], "redis");
        assert_eq!(v["version"], 3);
    }

    #[test]
    fn decodes_plain_base64_release() {
        // Older Helm data may not be gzipped — plain base64(json).
        let raw = STANDARD.encode(r#"{"name":"nginx","version":1}"#).into_bytes();
        let v = decode_release(&raw).unwrap();
        assert_eq!(v["name"], "nginx");
    }

    fn revision_json(version: i64, manifest: &str) -> Value {
        serde_json::from_str(&format!(
            r#"{{"name":"redis","namespace":"cache","version":{version},
                "info":{{"status":"deployed","last_deployed":"2026-07-01T00:00:00Z"}},
                "manifest":"{manifest}"}}"#
        ))
        .unwrap()
    }

    #[test]
    fn get_helm_release_in_defaults_revision_to_none_when_absent() {
        // The wire shape existing callers already send — no `revision` key at
        // all. This must keep deserialising exactly as it did before this
        // field existed.
        let input: GetHelmReleaseIn = serde_json::from_str(
            r#"{"context":"kind-dev","namespace":"cache","name":"redis"}"#,
        )
        .unwrap();
        assert_eq!(input.revision, None);
    }

    #[test]
    fn pick_revision_returns_the_named_revision() {
        let revisions = vec![revision_json(119, "manifest-119"), revision_json(118, "manifest-118")];
        let picked = pick_revision(&revisions, Some(118)).unwrap();
        assert_eq!(picked["manifest"], "manifest-118");
        assert_eq!(picked["version"], 118);
    }

    #[test]
    fn pick_revision_omitted_returns_the_current_one() {
        // Newest-first, exactly as `k8s.getHelmRelease` has always returned
        // when no revision was asked for.
        let revisions = vec![revision_json(119, "manifest-119"), revision_json(118, "manifest-118")];
        let picked = pick_revision(&revisions, None).unwrap();
        assert_eq!(picked["manifest"], "manifest-119");
        assert_eq!(picked["version"], 119);
    }

    #[test]
    fn pick_revision_missing_reports_the_error_instead_of_falling_back() {
        // The property that matters most: a revision that does not exist
        // must not silently resolve to the current one. That would make the
        // diff pane compare a manifest with itself and render "no changes"
        // about a release that changed.
        let revisions = vec![revision_json(119, "manifest-119"), revision_json(118, "manifest-118")];
        let err = pick_revision(&revisions, Some(42)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("42"), "error should name the missing revision: {msg}");

        // The load-bearing part: never the current revision's manifest.
        assert!(pick_revision(&revisions, Some(42)).is_err());
    }

    #[test]
    fn summarises_release_fields() {
        let v: Value = serde_json::from_str(
            r#"{
                "name":"redis","namespace":"cache","version":2,
                "info":{"status":"deployed","last_deployed":"2026-07-01T00:00:00Z"},
                "chart":{"metadata":{"name":"redis","version":"19.0.1","appVersion":"7.2.4"}}
            }"#,
        )
        .unwrap();
        let sum = summarise_release(&v);
        assert_eq!(sum.name, "redis");
        assert_eq!(sum.namespace, "cache");
        assert_eq!(sum.revision, 2);
        assert_eq!(sum.status, "deployed");
        assert_eq!(sum.chart, "redis");
        assert_eq!(sum.chart_version, "19.0.1");
        assert_eq!(sum.app_version, "7.2.4");
    }
}
