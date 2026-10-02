//! The params and result of every method. Wire names are the protocol's
//! camelCase. What srelens sends a sidecar is read past fields a later
//! srelens adds; what a sidecar sends srelens (`context` and the `host/*`
//! calls) takes no field it does not name, as srelens refuses one.

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::bounds::{
    MAX_CALL_FIELD_BYTES, MAX_CLUSTER_ID_BYTES, MAX_IDENTIFIER_LEN, MAX_NAMESPACE_LEN,
    MAX_OBJECT_NAME_LEN, MAX_TOKEN_LEN,
};

/// A request's id. srelens numbers its own from 1; a sidecar may use any
/// number or string for its calls. `1` and `"1"` are different ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(serde_json::Number),
    String(String),
}

impl From<u64> for RequestId {
    fn from(id: u64) -> RequestId {
        RequestId::Number(id.into())
    }
}

impl JsonSchema for RequestId {
    fn schema_name() -> String {
        "RequestId".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schema(json!({
            "description": format!(
                "A request's id: a number, or a string of at most {MAX_CALL_FIELD_BYTES} bytes as UTF-8. srelens numbers its own requests from 1; `1` and `\"1\"` are different ids."
            ),
            "anyOf": [
                {"type": "number"},
                {"type": "string", "maxLength": MAX_CALL_FIELD_BYTES},
            ],
        }))
    }
}

/// One end of the conversation: srelens, or the sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Peer {
    pub name: String,
    pub version: String,
}

/// The limits a sidecar runs under, so an SDK can hold itself to them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InitializeLimits {
    /// How long srelens waits for the answer to one request, in milliseconds.
    pub request_timeout_ms: u64,
    /// Requests srelens has in flight at once; the next is refused, not queued.
    pub max_concurrent_requests: u64,
    /// Streams open at once.
    pub max_streams: u64,
    /// The memory limit the sandbox enforces, in bytes.
    pub memory_bytes: u64,
    /// The CPU limit the sandbox enforces, in CPUs.
    pub cpus: f64,
    /// The data directory's size limit, in bytes.
    pub data_bytes: u64,
    /// The data directory's limit on files, directories and links.
    pub data_entries: u64,
}

/// `initialize`'s params: every sidecar API version srelens speaks, the
/// limits in force, and the one directory the sidecar may write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// Every sidecar API version srelens speaks, oldest first.
    pub api_versions: Vec<String>,
    pub host: Peer,
    pub limits: InitializeLimits,
    /// The data directory: the one path the sidecar may write, and its
    /// working directory.
    pub data_directory: String,
}

/// A sidecar's answer to `initialize`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// The version it chose, one of those offered.
    pub api_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<Peer>,
}

/// The `data` of error -32001, the answer to `initialize` from a sidecar that
/// speaks none of the offered versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnsupportedApiVersion {
    /// The versions it does speak.
    pub supported: Vec<String>,
}

/// The params and result of `activate`, `health`, `deactivate` and
/// `shutdown`: `{}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Empty {}

/// `$/cancelRequest`'s params: stop working on request `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CancelParams {
    pub id: RequestId,
}

/// `stream/open`'s params: open stream `stream`, streaming `method`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamOpenParams {
    /// The id srelens chose; every frame of the stream carries it.
    pub stream: u64,
    pub method: String,
    pub params: Value,
}

/// `stream/data`'s params: one frame of stream `stream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamDataParams {
    pub stream: u64,
    pub data: Value,
}

/// `stream/close`'s params: stream `stream` ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamCloseParams {
    pub stream: u64,
}

/// `stream/error`'s params: stream `stream` failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamErrorParams {
    pub stream: u64,
    pub message: String,
}

/// `stream/cancel`'s params: stop sending stream `stream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamCancelParams {
    pub stream: u64,
}

/// The cluster and namespace one call names. Every call from a sidecar
/// carries one: srelens has no current cluster to assume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[schemars(rename = "CallContext")]
pub struct CallContext {
    /// A cluster as srelens names it: the `context` of the request the
    /// sidecar is serving, a kubeconfig context's stable ID, pinned ID or
    /// name. At most 4096 bytes as UTF-8, and not blank.
    #[schemars(schema_with = "cluster_id")]
    pub cluster_id: String,
    /// A namespace, or null for every namespace or a cluster-scoped kind.
    /// Required, though it may be null: "no namespace" is said, not assumed.
    #[serde(deserialize_with = "explicit")]
    #[schemars(schema_with = "namespace")]
    pub namespace: Option<String>,
}

/// `host/read`'s params: read one of the app's declared readers, or one of
/// its `network.http` requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[schemars(rename = "HostReadParams")]
pub struct HostReadParams {
    pub context: CallContext,
    #[schemars(schema_with = "identifier")]
    pub capability: String,
}

/// `host/resource`'s params: inspect object `name` of a declared
/// custom-resource reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[schemars(rename = "HostResourceParams")]
pub struct HostResourceParams {
    pub context: CallContext,
    #[schemars(schema_with = "identifier")]
    pub capability: String,
    #[schemars(schema_with = "object_name")]
    pub name: String,
}

/// `host/action`'s params: run one of the app's declared actions on object
/// `name`, as read (`uid`, `resourceVersion`), once a person has confirmed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[schemars(rename = "HostActionParams")]
pub struct HostActionParams {
    pub context: CallContext,
    #[schemars(schema_with = "identifier")]
    pub capability: String,
    #[schemars(schema_with = "object_name")]
    pub name: String,
    #[schemars(schema_with = "identifier")]
    pub action: String,
    #[schemars(schema_with = "token")]
    pub uid: String,
    #[schemars(schema_with = "token")]
    pub resource_version: String,
}

/// Why [`CallContext::new`] refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextError {
    ClusterId,
    Namespace(String),
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextError::ClusterId => write!(
                f,
                "`clusterId` must name a cluster, in at most {MAX_CLUSTER_ID_BYTES} bytes"
            ),
            ContextError::Namespace(namespace) => write!(
                f,
                "`namespace` must be a Kubernetes namespace name, or none; not {:?}",
                namespace.chars().take(64).collect::<String>()
            ),
        }
    }
}

impl std::error::Error for ContextError {}

impl CallContext {
    /// The cluster and namespace a call names, held to the shapes srelens
    /// checks: a cluster that is not blank and at most 4096 bytes, and a
    /// Kubernetes namespace name or none.
    pub fn new(
        cluster: impl Into<String>,
        namespace: Option<&str>,
    ) -> Result<CallContext, ContextError> {
        let cluster_id = cluster.into();
        if !crate::shape::is_cluster_id(&cluster_id) {
            return Err(ContextError::ClusterId);
        }
        if let Some(namespace) = namespace {
            if !crate::shape::is_namespace(namespace) {
                return Err(ContextError::Namespace(namespace.to_owned()));
            }
        }
        Ok(CallContext {
            cluster_id,
            namespace: namespace.map(str::to_owned),
        })
    }
}

/// serde's derive also reads a struct from a JSON array (`["prod", "team"]`),
/// which the protocol never allows and srelens refuses. These types derive
/// with `#[serde(remote = "Self")]`, which makes the derived code inherent
/// functions, and implement the traits here: writing is unchanged, and
/// reading takes an object only, then runs the derived reader on it.
///
/// Reading through a `serde_json::Map` first changes what the text readers
/// (`serde_json::from_str`, `from_slice`, `from_reader`) do: duplicate keys
/// resolve last-wins instead of erroring, a parse error's line and column are
/// dropped, and a value that is not an object says "expected a map" rather
/// than naming the type. The host is unaffected, because it reads these
/// types from an already-parsed `Value`.
///
/// The derive also generates an inherent `T::deserialize(..)` function (from
/// `remote = "Self"`), which is `pub` and still reads the array form: call
/// `<T as Deserialize>::deserialize` or one of the `serde_json` functions
/// instead, never the inherent one.
macro_rules! object_only {
    ($($ty:ident),*) => {$(
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $ty::serialize(self, serializer)
            }
        }
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let fields = serde_json::Map::<String, Value>::deserialize(deserializer)?;
                $ty::deserialize(Value::Object(fields)).map_err(serde::de::Error::custom)
            }
        }
    )*};
}

object_only!(
    CallContext,
    HostReadParams,
    HostResourceParams,
    HostActionParams
);

/// A required field that may be null: absent is refused, null is `None`.
fn explicit<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

fn schema(value: Value) -> Schema {
    serde_json::from_value(value).expect("a JSON Schema")
}

// The shapes srelens holds a call's fields to (`crates/plugin-host/src/sidecar/broker.rs`,
// whose conformance test holds these to it). Every pattern is RE2-compatible:
// no lookaround, so `.` and `..` are refused with `not` rather than a lookahead.

fn identifier(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_IDENTIFIER_LEN,
        "pattern": format!("^[A-Za-z0-9-]{{1,{MAX_IDENTIFIER_LEN}}}$"),
    }))
}

fn object_name(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_OBJECT_NAME_LEN,
        "pattern": format!("^[A-Za-z0-9.-]{{1,{MAX_OBJECT_NAME_LEN}}}$"),
        "not": {"enum": [".", ".."]},
    }))
}

fn token(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_TOKEN_LEN,
        "pattern": format!("^[!-~]{{1,{MAX_TOKEN_LEN}}}$"),
    }))
}

fn namespace(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": ["string", "null"],
        "maxLength": MAX_NAMESPACE_LEN,
        "pattern": format!("^[a-z0-9]([a-z0-9-]{{0,{}}}[a-z0-9])?$", MAX_NAMESPACE_LEN - 2),
    }))
}

/// Every Unicode `White_Space` character: what `str::trim` trims, and so what
/// makes a `clusterId` blank ([`crate::shape::is_cluster_id`]). Written out,
/// because regex engines read `\s` differently: Go's RE2 as ASCII only, and
/// ECMA-262 with U+FEFF and without U+0085.
const WHITE_SPACE: &str = "\t\n\u{0b}\u{0c}\r \u{85}\u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}";

fn cluster_id(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "minLength": 1,
        // Characters, where srelens counts bytes: an SDK checks the bytes itself.
        "maxLength": MAX_CLUSTER_ID_BYTES,
        // Not blank: at least one character that is not white space, as a
        // class every engine reads the same way (see WHITE_SPACE).
        "pattern": format!("[^{WHITE_SPACE}]"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn limits() -> InitializeLimits {
        InitializeLimits {
            request_timeout_ms: 30_000,
            max_concurrent_requests: 8,
            max_streams: 5,
            memory_bytes: 256 * 1024 * 1024,
            cpus: 1.0,
            data_bytes: 1 << 30,
            data_entries: 100_000,
        }
    }

    fn initialize() -> InitializeParams {
        InitializeParams {
            api_versions: vec!["0.1.0".into()],
            host: Peer {
                name: "srelens".into(),
                version: "0.15.0".into(),
            },
            limits: limits(),
            data_directory: "/data/0123".into(),
        }
    }

    #[test]
    fn initialize_is_written_in_the_protocols_camel_case() {
        assert_eq!(
            serde_json::to_value(initialize()).unwrap(),
            json!({
                "apiVersions": ["0.1.0"],
                "host": {"name": "srelens", "version": "0.15.0"},
                "limits": {
                    "requestTimeoutMs": 30000,
                    "maxConcurrentRequests": 8,
                    "maxStreams": 5,
                    "memoryBytes": 268435456u64,
                    "cpus": 1.0,
                    "dataBytes": 1073741824u64,
                    "dataEntries": 100000,
                },
                "dataDirectory": "/data/0123",
            })
        );
    }

    #[test]
    fn what_the_host_sends_is_read_past_fields_a_later_host_adds() {
        let mut wire = serde_json::to_value(initialize()).unwrap();
        wire["future"] = json!(true);
        wire["limits"]["gpus"] = json!(1);
        assert_eq!(
            serde_json::from_value::<InitializeParams>(wire).unwrap(),
            initialize()
        );
        assert!(serde_json::from_value::<Empty>(json!({"reason": "update"})).is_ok());
        assert!(serde_json::from_value::<StreamOpenParams>(
            json!({"stream": 1, "method": "watch", "params": {}, "priority": 1})
        )
        .is_ok());
    }

    #[test]
    fn the_rust_spelling_is_not_the_wire_spelling() {
        assert!(
            serde_json::from_value::<InitializeResult>(json!({"api_version": "0.1.0"})).is_err()
        );
        assert!(serde_json::from_value::<CallContext>(
            json!({"cluster_id": "prod", "namespace": null})
        )
        .is_err());
        let action = json!({"context": {"clusterId": "prod", "namespace": null},
            "capability": "apps", "name": "web", "action": "sync", "uid": "u",
            "resource_version": "1"});
        assert!(serde_json::from_value::<HostActionParams>(action).is_err());

        let mut wire = serde_json::to_value(initialize()).unwrap();
        let api_versions = wire.as_object_mut().unwrap().remove("apiVersions").unwrap();
        wire["api_versions"] = api_versions;
        assert!(serde_json::from_value::<InitializeParams>(wire).is_err());

        let mut limits = serde_json::to_value(limits()).unwrap();
        let request_timeout_ms = limits
            .as_object_mut()
            .unwrap()
            .remove("requestTimeoutMs")
            .unwrap();
        limits["request_timeout_ms"] = request_timeout_ms;
        assert!(serde_json::from_value::<InitializeLimits>(limits).is_err());
    }

    #[test]
    fn a_call_names_its_namespace_even_when_it_is_none() {
        let context: CallContext =
            serde_json::from_value(json!({"clusterId": "prod", "namespace": null})).unwrap();
        assert_eq!(context.namespace, None);
        assert_eq!(
            serde_json::to_value(&context).unwrap(),
            json!({"clusterId": "prod", "namespace": null})
        );
        let missing =
            serde_json::from_value::<CallContext>(json!({"clusterId": "prod"})).unwrap_err();
        assert!(missing.to_string().contains("namespace"), "{missing}");
    }

    #[test]
    fn a_call_to_the_host_takes_no_field_it_does_not_name() {
        let context = json!({"clusterId": "prod", "namespace": "team"});
        assert!(serde_json::from_value::<CallContext>(
            json!({"clusterId": "prod", "namespace": null, "current": true})
        )
        .is_err());
        assert!(serde_json::from_value::<HostReadParams>(
            json!({"context": context, "capability": "apps", "revision": 1})
        )
        .is_err());
        assert!(serde_json::from_value::<HostResourceParams>(
            json!({"context": context, "capability": "apps", "name": "web", "id": "other"})
        )
        .is_err());
        let read: HostReadParams =
            serde_json::from_value(json!({"context": context, "capability": "apps"})).unwrap();
        assert_eq!(read.capability, "apps");
        let action: HostActionParams = serde_json::from_value(json!({"context": context,
            "capability": "apps", "name": "web", "action": "sync", "uid": "u-1",
            "resourceVersion": "42"}))
        .unwrap();
        assert_eq!(action.resource_version, "42");
    }

    #[test]
    fn a_request_id_is_a_number_or_a_string_and_the_two_differ() {
        let number: RequestId = serde_json::from_value(json!(1)).unwrap();
        let string: RequestId = serde_json::from_value(json!("1")).unwrap();
        assert_ne!(number, string);
        assert_eq!(number, RequestId::from(1));
        assert_eq!(serde_json::to_value(&string).unwrap(), json!("1"));
        assert!(serde_json::from_value::<RequestId>(Value::Null).is_err());
        assert!(serde_json::from_value::<RequestId>(json!({})).is_err());
    }

    #[test]
    fn a_stream_frame_carries_its_stream_and_what_it_says() {
        assert_eq!(
            serde_json::to_value(StreamDataParams {
                stream: 2,
                data: Value::Null
            })
            .unwrap(),
            json!({"stream": 2, "data": null})
        );
        assert!(
            serde_json::from_value::<StreamDataParams>(json!({"stream": 2})).is_err(),
            "a data frame without data"
        );
        assert!(serde_json::from_value::<StreamErrorParams>(json!({"stream": 2})).is_err());
        assert_eq!(
            serde_json::to_value(StreamCancelParams { stream: 3 }).unwrap(),
            json!({"stream": 3})
        );
        assert_eq!(
            serde_json::to_value(CancelParams {
                id: RequestId::from(4)
            })
            .unwrap(),
            json!({"id": 4})
        );
    }

    #[test]
    fn a_sidecar_may_leave_out_its_own_name() {
        let result: InitializeResult =
            serde_json::from_value(json!({"apiVersion": "0.1.0"})).unwrap();
        assert_eq!(result.sidecar, None);
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({"apiVersion": "0.1.0"})
        );
        let data: UnsupportedApiVersion =
            serde_json::from_value(json!({"supported": ["1.0.0"]})).unwrap();
        assert_eq!(data.supported, ["1.0.0"]);
    }

    #[test]
    fn a_call_to_the_host_is_an_object_never_a_list() {
        assert!(serde_json::from_value::<CallContext>(json!(["prod", "team"])).is_err());
        assert!(serde_json::from_value::<CallContext>(json!(["prod", null])).is_err());
        assert!(serde_json::from_value::<HostReadParams>(json!([["prod", null], "apps"])).is_err());
        assert!(serde_json::from_value::<HostReadParams>(
            json!({"context": ["prod", null], "capability": "apps"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<HostResourceParams>(json!([["p", null], "apps", "web"]))
                .is_err()
        );
        assert!(serde_json::from_value::<HostActionParams>(json!([
            ["p", null],
            "apps",
            "web",
            "sync",
            "u",
            "1"
        ]))
        .is_err());
        // The object form still reads, and the old messages stay.
        let context: CallContext =
            serde_json::from_value(json!({"clusterId": "prod", "namespace": null})).unwrap();
        assert_eq!(context.cluster_id, "prod");
        let missing =
            serde_json::from_value::<CallContext>(json!({"clusterId": "prod"})).unwrap_err();
        assert!(
            missing.to_string().contains("missing field `namespace`"),
            "{missing}"
        );
    }

    #[test]
    fn a_context_built_in_a_sidecar_is_held_to_the_shapes_srelens_checks() {
        let ok = CallContext::new("kind-dev", Some("team")).unwrap();
        assert_eq!(ok.cluster_id, "kind-dev");
        assert_eq!(ok.namespace.as_deref(), Some("team"));
        assert_eq!(CallContext::new("kind-dev", None).unwrap().namespace, None);
        assert_eq!(
            CallContext::new("x".repeat(4096), None).map(|c| c.cluster_id.len()),
            Ok(4096)
        );
        for cluster in ["", "   ", &"x".repeat(4097)] {
            assert_eq!(
                CallContext::new(cluster, None),
                Err(ContextError::ClusterId),
                "{cluster:?}"
            );
        }
        // 3000 two-byte characters: 6000 bytes, over the byte limit.
        assert_eq!(
            CallContext::new("é".repeat(3000), None),
            Err(ContextError::ClusterId)
        );
        for namespace in ["", "Team", "-team", "team-", "te.am", &"a".repeat(64)] {
            let refused = CallContext::new("kind-dev", Some(namespace)).unwrap_err();
            assert_eq!(refused, ContextError::Namespace(namespace.to_owned()));
            assert!(refused.to_string().contains("namespace"), "{refused}");
        }
    }
}
