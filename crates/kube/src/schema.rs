//! `k8s.openApiSchema` — fetch the OpenAPI v3 schema for a resource kind so the
//! editor can offer field autocomplete. We hit the cluster's `/openapi/v3`
//! (works for built-ins AND CRDs), locate the kind by its
//! `x-kubernetes-group-version-kind`, and return just that schema plus the
//! types it references (transitively) — pruned so the payload stays small.

use std::collections::BTreeSet;
use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use http::Request;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::manifest::parse_api_version;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenApiSchemaIn {
    pub context: String,
    /// `apiVersion` on the wire, as every caller has always sent it.
    ///
    /// Without this rename the field was `api_version`, which nothing sends:
    /// `core`'s `openApiSchema` wrapper passes `apiVersion`, so every call
    /// failed to deserialize and the editor's field autocomplete was dead —
    /// silently, because the caller turns any failure into "no schema". Its
    /// neighbours (`ListEventsIn`, `ListReplicaSetsIn`, `PodsForSelectorIn`)
    /// each rename their own multi-word fields the same way; this one did not.
    /// The e2e case did not catch it because it spoke the STRUCT's spelling
    /// rather than the app's — it sends `apiVersion` now.
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenApiSchemaOut {
    /// The pruned `{schemaKey: schema}` map, JSON-encoded (the frontend parses).
    pub schemas: String,
    /// The schema key for the requested kind (the completion root).
    pub key: Option<String>,
}

/// Find the components.schemas key whose `x-kubernetes-group-version-kind`
/// matches (group, version, kind).
pub fn find_schema_key(schemas: &Value, group: &str, version: &str, kind: &str) -> Option<String> {
    for (key, schema) in schemas.as_object()? {
        let gvks = schema
            .get("x-kubernetes-group-version-kind")
            .and_then(Value::as_array);
        for gvk in gvks.into_iter().flatten() {
            let g = gvk.get("group").and_then(Value::as_str).unwrap_or("");
            let v = gvk.get("version").and_then(Value::as_str).unwrap_or("");
            let k = gvk.get("kind").and_then(Value::as_str).unwrap_or("");
            if g == group && v == version && k == kind {
                return Some(key.clone());
            }
        }
    }
    None
}

fn collect_refs(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "$ref" {
                    if let Some(name) = v.as_str().and_then(|s| s.strip_prefix("#/components/schemas/")) {
                        out.push(name.to_string());
                    }
                } else {
                    collect_refs(v, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_refs(v, out)),
        _ => {}
    }
}

/// All schema keys reachable from `root` by following `$ref`s (including root).
pub fn collect_reachable(schemas: &Value, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![root.to_string()];
    while let Some(key) = stack.pop() {
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some(schema) = schemas.get(&key) {
            collect_refs(schema, &mut stack);
        }
    }
    seen
}

/// `k8s.openApiSchema` — schema for a kind (+ referenced types) for autocomplete.
pub fn open_api_schema_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<OpenApiSchemaIn, OpenApiSchemaOut, _, _>(
        "k8s.openApiSchema",
        "fetch the OpenAPI schema for a resource kind (for field autocomplete)",
        Annotations::READ_ONLY,
        move |input: OpenApiSchemaIn| {
            let cache = cache.clone();
            async move {
                let (group, version) = parse_api_version(&input.api_version);
                let path = if group.is_empty() {
                    format!("/openapi/v3/api/{version}")
                } else {
                    format!("/openapi/v3/apis/{group}/{version}")
                };
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let req = Request::get(&path)
                    .body(Vec::new())
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let doc: Value = tokio::time::timeout(request_timeout(), client.request(req))
                    .await
                    .map_err(|_| CapabilityError::Handler("openapi fetch timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;

                let schemas = doc.pointer("/components/schemas").cloned().unwrap_or(Value::Null);
                let key = find_schema_key(&schemas, &group, &version, &input.kind);

                // Prune to just the kind's schema + everything it references.
                let pruned = match &key {
                    Some(root) => {
                        let mut map = Map::new();
                        for name in collect_reachable(&schemas, root) {
                            if let Some(s) = schemas.get(&name) {
                                map.insert(name, s.clone());
                            }
                        }
                        Value::Object(map)
                    }
                    None => Value::Object(Map::new()),
                };

                Ok(OpenApiSchemaOut {
                    schemas: serde_json::to_string(&pruned).unwrap_or_else(|_| "{}".into()),
                    key,
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "io.k8s.api.apps.v1.Deployment": {
                "x-kubernetes-group-version-kind": [{ "group": "apps", "version": "v1", "kind": "Deployment" }],
                "properties": {
                    "spec": { "$ref": "#/components/schemas/io.k8s.api.apps.v1.DeploymentSpec" }
                }
            },
            "io.k8s.api.apps.v1.DeploymentSpec": {
                "properties": {
                    "replicas": { "type": "integer" },
                    "template": { "$ref": "#/components/schemas/io.k8s.api.core.v1.PodTemplateSpec" }
                }
            },
            "io.k8s.api.core.v1.PodTemplateSpec": { "properties": { "spec": { "type": "object" } } },
            "io.k8s.api.core.v1.Unrelated": { "properties": {} }
        })
    }

    #[test]
    fn finds_key_by_gvk() {
        assert_eq!(
            find_schema_key(&sample(), "apps", "v1", "Deployment").as_deref(),
            Some("io.k8s.api.apps.v1.Deployment")
        );
        assert!(find_schema_key(&sample(), "apps", "v1", "Missing").is_none());
    }

    /// The wire shape, which is not the struct's field spelling.
    ///
    /// `core`'s `openApiSchema` wrapper sends `apiVersion`; the field is
    /// `api_version`. Without the rename every call failed to deserialize and
    /// the editor's field autocomplete was dead — silently, because the caller
    /// reads any failure as "this kind has no schema". The e2e case sent the
    /// struct's own spelling and so agreed with the bug.
    #[test]
    fn deserializes_the_payload_callers_actually_send() {
        let input: OpenApiSchemaIn = serde_json::from_value(serde_json::json!({
            "context": "prod-eu",
            "apiVersion": "v1",
            "kind": "Secret",
        }))
        .expect("the wrapper's own payload must deserialize");
        assert_eq!(input.api_version, "v1");
        assert_eq!(input.kind, "Secret");
        // And the struct spelling is NOT also accepted, so nothing drifts back.
        assert!(serde_json::from_value::<OpenApiSchemaIn>(serde_json::json!({
            "context": "prod-eu",
            "api_version": "v1",
            "kind": "Secret",
        }))
        .is_err());
    }

    #[test]
    fn collects_reachable_refs_and_prunes_unrelated() {
        let reachable = collect_reachable(&sample(), "io.k8s.api.apps.v1.Deployment");
        assert!(reachable.contains("io.k8s.api.apps.v1.Deployment"));
        assert!(reachable.contains("io.k8s.api.apps.v1.DeploymentSpec"));
        assert!(reachable.contains("io.k8s.api.core.v1.PodTemplateSpec"));
        assert!(!reachable.contains("io.k8s.api.core.v1.Unrelated"));
    }
}
