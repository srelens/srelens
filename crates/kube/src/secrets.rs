//! The `k8s.listSecrets` capability.
//!
//! The summary deliberately carries only the Secret's **type** and **key
//! count** — never any key names or values — so listing Secrets can never leak
//! material. Values are only ever fetched (and masked) in the detail view.

use std::sync::Arc;

use std::collections::BTreeMap;

use srelens_capability::{Annotations, Capability, CapabilityError};
use base64::Engine;
use k8s_openapi::api::core::v1::Secret;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Blank the values of a serialized Secret's `data` and `stringData` maps in
/// place, keeping the keys. Every ungated reader of a Secret runs this —
/// `k8s.getObject`, `k8s.getManifest` and `k8s.diffManifest` — so no generic
/// path carries Secret material; values are only read through the dedicated,
/// consent-gateable `k8s.getSecret`.
///
/// **And every value under `metadata.annotations`, keys kept.** Blanking `data`
/// while leaving the annotations alone put the same values back two lines
/// further down: on an `apply`-managed Secret,
/// `kubectl.kubernetes.io/last-applied-configuration` holds the entire applied
/// manifest, base64 `data` map included. Both ungated readers of this path —
/// `k8s.getManifest` and `k8s.diffManifest` — returned it in the clear.
///
/// And `k8s.getManifest` went on doing so for another round (#661), because
/// the redaction went in beside it, in `k8s.getObject`, while the YAML path
/// serialized the fetched object straight out. Two doc comments and
/// `docs/MCP.md` all said it ran this. Nothing checked, so nothing noticed:
/// `get_manifest_redacts_a_secrets_values_and_annotations` now does.
///
/// The scope rule is **every annotation on a Secret**, not "the one annotation
/// kubectl writes", and the difference is the point: any controller can echo a
/// Secret's material into an annotation of its own (a copy for a sidecar, a
/// "previous value" left by a rotation, a checksum over the plaintext), and
/// nothing here can tell those from a harmless one by looking at the key. A
/// rule naming one well-known key would read as complete while covering one
/// carrier out of many.
///
/// This is the same rule, for the same reason, that `redactSecretManifest`
/// (`packages/core/src/lib/manifest.ts`) applies to the editor's YAML. The two
/// were one finding apart: the frontend was fixed, the backend that serves
/// every other caller — MCP clients included — was not.
///
/// Both sides of a diff run through this, so a *change* to a Secret's
/// annotation reads as unchanged. That is the trade `data` already makes, and
/// it is the right way round: a redactor that showed the change would show the
/// value.
pub(crate) fn redact_secret_data(object: &mut serde_json::Value) {
    for key in ["data", "stringData"] {
        if let Some(map) = object.get_mut(key).and_then(|d| d.as_object_mut()) {
            for value in map.values_mut() {
                *value = serde_json::Value::String(String::new());
            }
        }
    }
    if let Some(annotations) = object
        .get_mut("metadata")
        .and_then(|m| m.get_mut("annotations"))
    {
        match annotations.as_object_mut() {
            Some(map) => {
                for value in map.values_mut() {
                    *value = serde_json::Value::String(String::new());
                }
            }
            // Unreachable from either caller — both serialize a `DynamicObject`,
            // whose annotations are a string map or absent. Blanked whole
            // rather than passed through, because a redactor that hands back
            // whatever it did not recognise is not a redactor.
            None => *annotations = serde_json::Value::String(String::new()),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSecretsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SecretSummary {
    pub name: String,
    pub namespace: String,
    /// The Secret's `type` (e.g. `Opaque`, `kubernetes.io/tls`).
    #[serde(rename = "type")]
    pub type_: String,
    /// Number of keys — NOT their names or values.
    pub keys: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and a summary
    /// is only rebuilt when a watch event arrives for the object — so it goes
    /// stale (#405). Prefer this; `age` stays for callers that have no clock.
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListSecretsOut {
    pub secrets: Vec<SecretSummary>,
}

pub(crate) fn summarise(secret: Secret) -> SecretSummary {
    let keys = secret.data.as_ref().map_or(0, |d| d.len())
        + secret.string_data.as_ref().map_or(0, |d| d.len());
    SecretSummary {
        name: secret.metadata.name.clone().unwrap_or_default(),
        namespace: secret.metadata.namespace.clone().unwrap_or_default(),
        type_: secret.type_.clone().unwrap_or_default(),
        keys: keys as i32,
        created: crate::creation_rfc3339(secret.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(secret.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(secret.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listSecrets` — list Secrets in a namespace (type + key count only).
pub fn list_secrets_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListSecretsIn, ListSecretsOut, _, _>(
        "k8s.listSecrets",
        "list Secrets in a namespace (name, type, and key count only — no values)",
        Annotations::READ_ONLY,
        move |input: ListSecretsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Secret> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list secrets timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListSecretsOut {
                    secrets: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSecretIn {
    pub context: String,
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetSecretOut {
    /// Base64-encoded values, keyed by name (as stored in `Secret.data`).
    pub data: BTreeMap<String, String>,
}

/// `k8s.getSecret` — read a Secret's values. This is the **only** capability
/// that returns Secret material through the structured API (the generic
/// `k8s.getObject` redacts it), and it is annotated `sensitive` so a consent
/// policy can gate it separately from ordinary reads.
pub fn get_secret_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<GetSecretIn, GetSecretOut, _, _>(
        "k8s.getSecret",
        "read a Secret's values (sensitive; returns base64-encoded data)",
        Annotations::SENSITIVE_READ,
        move |input: GetSecretIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api: Api<Secret> = crate::scoped_api(client, &input.namespace);
                let secret = tokio::time::timeout(request_timeout(), api.get(&input.name))
                    .await
                    .map_err(|_| CapabilityError::Handler("get secret timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let data = secret
                    .data
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(k, v)| (k, base64::engine::general_purpose::STANDARD.encode(v.0)))
                    .collect();
                Ok(GetSecretOut { data })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_secrets_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listSecrets");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn get_secret_is_sensitive() {
        let cap = get_secret_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.getSecret");
        assert!(cap.annotations.read_only);
        assert!(cap.annotations.sensitive, "getSecret must be annotated sensitive for consent gating");
    }

    #[test]
    fn redaction_blanks_secret_values_but_keeps_keys() {
        let mut object = serde_json::json!({
            "kind": "Secret",
            "metadata": { "name": "web-tls" },
            "data": { "tls.crt": "U0VDUkVU", "tls.key": "TU9SRQ==" }
        });
        redact_secret_data(&mut object);
        let data = object["data"].as_object().unwrap();
        // Keys remain so the UI can list them...
        assert!(data.contains_key("tls.crt"));
        assert!(data.contains_key("tls.key"));
        // ...but every value is blanked — no material survives the generic path.
        assert_eq!(data["tls.crt"], serde_json::json!(""));
        assert_eq!(data["tls.key"], serde_json::json!(""));
        assert!(!object.to_string().contains("U0VDUkVU"));
    }

    #[test]
    fn redaction_blanks_string_data_values_but_keeps_keys() {
        let mut object = serde_json::json!({
            "kind": "Secret",
            "metadata": { "name": "web-creds" },
            "data": { "tls.crt": "U0VDUkVU" },
            "stringData": { "password": "hunter2", "username": "admin" }
        });
        redact_secret_data(&mut object);
        let data = object["data"].as_object().unwrap();
        assert_eq!(data["tls.crt"], serde_json::json!(""));
        let string_data = object["stringData"].as_object().unwrap();
        // Keys remain so the UI can list them...
        assert!(string_data.contains_key("password"));
        assert!(string_data.contains_key("username"));
        // ...but every value is blanked — no material survives the generic path.
        assert_eq!(string_data["password"], serde_json::json!(""));
        assert_eq!(string_data["username"], serde_json::json!(""));
        assert!(!object.to_string().contains("hunter2"));
    }

    /// The carrier that made blanking `data` alone useless: on an
    /// `apply`-managed Secret, `last-applied-configuration` holds the whole
    /// manifest, base64 `data` map included. Before this, `k8s.getManifest`
    /// and `k8s.diffManifest` — both ungated reads — returned it in the clear.
    #[test]
    fn redaction_blanks_a_data_map_hidden_in_the_last_applied_annotation() {
        let applied = serde_json::json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": { "name": "web-tls", "namespace": "prod" },
            "data": { "tls.key": "TU9SRQ==", "token": "U0VDUkVU" }
        })
        .to_string();
        let mut object = serde_json::json!({
            "kind": "Secret",
            "metadata": {
                "name": "web-tls",
                "annotations": {
                    "kubectl.kubernetes.io/last-applied-configuration": applied,
                    "reloader.stakater.com/match": "true"
                }
            },
            "data": { "tls.key": "TU9SRQ==", "token": "U0VDUkVU" }
        });
        redact_secret_data(&mut object);

        let serialized = object.to_string();
        assert!(
            !serialized.contains("U0VDUkVU"),
            "leaked through an annotation: {serialized}"
        );
        assert!(
            !serialized.contains("TU9SRQ=="),
            "leaked through an annotation: {serialized}"
        );

        // Keys survive, so a reader still sees which controllers touched the
        // Secret — the whole point of an annotation key.
        let annotations = object["metadata"]["annotations"].as_object().unwrap();
        assert!(annotations.contains_key("kubectl.kubernetes.io/last-applied-configuration"));
        assert!(annotations.contains_key("reloader.stakater.com/match"));
        // Every value, not just the well-known key: the scope rule is "every
        // annotation on a Secret", because any controller can be a carrier.
        for value in annotations.values() {
            assert_eq!(value, &serde_json::json!(""));
        }
    }

    /// Nothing to blank is not an error, and a shape this does not understand
    /// is blanked rather than passed through.
    #[test]
    fn redaction_fails_closed_on_annotations_it_does_not_understand() {
        let mut none = serde_json::json!({"kind":"Secret","metadata":{"name":"s"}});
        redact_secret_data(&mut none);
        assert_eq!(none["metadata"]["annotations"], serde_json::Value::Null);

        let mut wrong_shape =
            serde_json::json!({"kind":"Secret","metadata":{"name":"s","annotations":"U0VDUkVU"}});
        redact_secret_data(&mut wrong_shape);
        assert_eq!(
            wrong_shape["metadata"]["annotations"],
            serde_json::json!("")
        );
        assert!(!wrong_shape.to_string().contains("U0VDUkVU"));
    }

    #[test]
    fn summarises_type_and_key_count_without_values() {
        let mut data = BTreeMap::new();
        data.insert("tls.crt".to_string(), k8s_openapi::ByteString(b"SECRET-CERT".to_vec()));
        data.insert("tls.key".to_string(), k8s_openapi::ByteString(b"SECRET-KEY".to_vec()));
        let secret = Secret {
            metadata: kube::core::ObjectMeta {
                name: Some("web-tls".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            type_: Some("kubernetes.io/tls".into()),
            data: Some(data),
            ..Default::default()
        };
        let s = summarise(secret);
        assert_eq!(s.name, "web-tls");
        assert_eq!(s.type_, "kubernetes.io/tls");
        assert_eq!(s.keys, 2);
        // The summary carries no field that could hold key material.
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("SECRET"), "summary must not contain any secret material: {json}");
        assert!(!json.contains("tls.crt"), "summary must not contain key names: {json}");
    }
}
