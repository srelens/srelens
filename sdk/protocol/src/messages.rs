//! The params and result of every method. Wire names are the protocol's
//! camelCase. What srelens sends a sidecar is read past fields a later
//! srelens adds; what a sidecar sends srelens (`context` and the `host/*`
//! calls) takes no field it does not name, as srelens refuses one.

#[cfg(feature = "schema")]
use schemars::{gen::SchemaGenerator, schema::Schema, JsonSchema};
use serde::{Deserialize, Deserializer, Serialize};
#[cfg(feature = "schema")]
use serde_json::json;
use serde_json::Value;

use crate::bounds::MAX_CLUSTER_ID_BYTES;
use crate::{MAX_TUNNEL_CHUNK_TEXT, TUNNEL_WINDOW_BYTES};
// What only the schema's patterns and limits read.
#[cfg(feature = "schema")]
use crate::bounds::{
    MAX_CALL_FIELD_BYTES, MAX_IDENTIFIER_LEN, MAX_NAMESPACE_LEN, MAX_OBJECT_NAME_LEN, MAX_TOKEN_LEN,
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

#[cfg(feature = "schema")]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Peer {
    pub name: String,
    pub version: String,
}

/// The limits a sidecar runs under, so an SDK can hold itself to them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// The version it chose, one of those offered.
    pub api_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<Peer>,
}

/// The `data` of error -32001, the answer to `initialize` from a sidecar that
/// speaks none of the offered versions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct UnsupportedApiVersion {
    /// The versions it does speak.
    pub supported: Vec<String>,
}

/// The params and result of `activate`, `health`, `deactivate` and
/// `shutdown`: `{}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Empty {}

/// `$/cancelRequest`'s params: stop working on request `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CancelParams {
    pub id: RequestId,
}

/// `stream/open`'s params: open stream `stream`, streaming `method`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StreamOpenParams {
    /// The id srelens chose; every frame of the stream carries it.
    pub stream: u64,
    pub method: String,
    pub params: Value,
}

/// `stream/data`'s params: one frame of stream `stream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StreamDataParams {
    pub stream: u64,
    pub data: Value,
}

/// `stream/close`'s params: stream `stream` ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StreamCloseParams {
    pub stream: u64,
}

/// `stream/error`'s params: stream `stream` failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StreamErrorParams {
    pub stream: u64,
    pub message: String,
}

/// `stream/cancel`'s params: stop sending stream `stream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StreamCancelParams {
    pub stream: u64,
}

/// The cluster and namespace one call names. Every call from a sidecar
/// carries one: srelens has no current cluster to assume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "CallContext"))]
pub struct CallContext {
    /// A cluster as srelens names it: the `context` of the request the
    /// sidecar is serving, a kubeconfig context's stable ID, pinned ID or
    /// name. At most 4096 bytes as UTF-8, and not blank.
    #[cfg_attr(feature = "schema", schemars(schema_with = "cluster_id"))]
    pub cluster_id: String,
    /// A namespace, or null for every namespace or a cluster-scoped kind.
    /// Required, though it may be null: "no namespace" is said, not assumed.
    #[serde(deserialize_with = "explicit")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "namespace"))]
    pub namespace: Option<String>,
}

/// `host/read`'s params: read one of the app's declared readers, or one of
/// its `network.http` requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "HostReadParams"))]
pub struct HostReadParams {
    pub context: CallContext,
    #[cfg_attr(feature = "schema", schemars(schema_with = "identifier"))]
    pub capability: String,
    /// Opaque continuation from this binding's previous page (API 0.2).
    #[serde(
        default,
        deserialize_with = "read_cursor",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema", schemars(schema_with = "cursor_schema"))]
    pub cursor: Option<String>,
}

fn read_cursor<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let cursor = String::deserialize(d)?;
    if cursor.len() > 8192 || !cursor.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(serde::de::Error::custom(
            "cursor needs at most 8192 ASCII graphic bytes",
        ));
    }
    Ok(Some(cursor))
}

#[cfg(feature = "schema")]
fn cursor_schema(_: &mut SchemaGenerator) -> Schema {
    schema(json!({"type":"string","maxLength":8192,"pattern":"^[!-~]*$"}))
}

/// Discover only the resource bindings declared and granted to this app (API 0.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "HostBindingAvailabilityParams"))]
pub struct HostBindingAvailabilityParams {
    pub context: CallContext,
    #[serde(deserialize_with = "read_binding_names")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "binding_names"))]
    pub bindings: Vec<String>,
}

/// Run a declared, digest-pinned namespace Job after host confirmation (API 0.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "HostRunJobParams"))]
pub struct HostRunJobParams {
    pub context: JobContext,
    #[cfg_attr(feature = "schema", schemars(schema_with = "identifier"))]
    pub capability: String,
    #[serde(deserialize_with = "read_job_inputs")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "job_inputs"))]
    pub inputs: std::collections::BTreeMap<String, String>,
}

/// Job execution always names one namespace; null never means a cluster scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "JobContext"))]
pub struct JobContext {
    #[cfg_attr(feature = "schema", schemars(schema_with = "cluster_id"))]
    pub cluster_id: String,
    #[cfg_attr(feature = "schema", schemars(schema_with = "scoped_namespace"))]
    pub namespace: String,
}

#[cfg(feature = "schema")]
fn scoped_namespace(generator: &mut SchemaGenerator) -> Schema {
    let mut value = serde_json::to_value(namespace(generator)).unwrap();
    value["type"] = json!("string");
    schema(value)
}

fn read_job_inputs<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<std::collections::BTreeMap<String, String>, D::Error> {
    let inputs = std::collections::BTreeMap::<String, String>::deserialize(d)?;
    if inputs.len() > 16
        || inputs.iter().any(|(name, value)| {
            !crate::shape::is_identifier(name)
                || value.is_empty()
                || value.starts_with('-')
                || value.len() > 512
                || !value.bytes().all(|b| (b' '..=b'~').contains(&b))
        })
    {
        return Err(serde::de::Error::custom(
            "Job inputs need at most sixteen declared names and 1–512 printable ASCII bytes each",
        ));
    }
    Ok(inputs)
}

#[cfg(feature = "schema")]
fn job_inputs(generator: &mut SchemaGenerator) -> Schema {
    schema(
        json!({"type":"object", "maxProperties":16, "propertyNames":identifier(generator),
        "additionalProperties":{"type":"string","minLength":1,"maxLength":512,"pattern":"^[\\x20-\\x2c\\x2e-\\x7e][ -~]{0,511}$"}}),
    )
}

fn read_binding_names<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let bindings = Vec::<String>::deserialize(d)?;
    let mut seen = std::collections::HashSet::new();
    if bindings.is_empty()
        || bindings.len() > 16
        || bindings
            .iter()
            .any(|name| !crate::shape::is_identifier(name) || !seen.insert(name))
    {
        return Err(serde::de::Error::custom(
            "Choose 1–16 unique declared binding names",
        ));
    }
    Ok(bindings)
}

#[cfg(feature = "schema")]
fn binding_names(generator: &mut SchemaGenerator) -> Schema {
    schema(
        json!({"type":"array", "minItems":1, "maxItems":16, "uniqueItems":true, "items": identifier(generator)}),
    )
}

/// Open a TCP connection to a pod's port as tunnel `tunnel`, under the
/// declared `capability` (API 0.3). Names exactly one of `pod` and `service`;
/// `name` is the object whose selector is the scope; leave it out for a
/// namespace-granted binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "HostConnectParams"))]
pub struct HostConnectParams {
    pub context: JobContext,
    #[cfg_attr(feature = "schema", schemars(schema_with = "identifier"))]
    pub capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "object_name"))]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "object_name"))]
    pub pod: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "object_name"))]
    pub service: Option<String>,
    #[serde(deserialize_with = "read_tunnel")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "tunnel_id"))]
    pub tunnel: u64,
}

// Written out, not in `object_only!`: reading also checks that exactly one of
// `pod` and `service` is named.
impl Serialize for HostConnectParams {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        HostConnectParams::serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for HostConnectParams {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = serde_json::Map::<String, Value>::deserialize(deserializer)?;
        let params = HostConnectParams::deserialize(Value::Object(fields))
            .map_err(serde::de::Error::custom)?;
        if params.pod.is_some() == params.service.is_some() {
            return Err(serde::de::Error::custom(
                "name exactly one of `pod` and `service`",
            ));
        }
        Ok(params)
    }
}

/// The answer to `host/connect`: the pod the tunnel reaches (a service's
/// resolves to one of its ready pods).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct HostConnectResult {
    pub pod: String,
}

/// `tunnel/data`'s params: bytes for tunnel `tunnel`, in base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "TunnelDataParams"))]
pub struct TunnelDataParams {
    #[serde(deserialize_with = "read_tunnel")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "tunnel_id"))]
    pub tunnel: u64,
    #[serde(deserialize_with = "read_base64")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "base64_text"))]
    pub data: String,
}

/// `tunnel/credit`'s params: the receiver of this frame may send `bytes` more
/// on tunnel `tunnel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "TunnelCreditParams"))]
pub struct TunnelCreditParams {
    #[serde(deserialize_with = "read_tunnel")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "tunnel_id"))]
    pub tunnel: u64,
    #[serde(deserialize_with = "read_credit")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "credit_bytes"))]
    pub bytes: u32,
}

/// `tunnel/close`'s params: tunnel `tunnel` ended, and why if it failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "TunnelCloseParams"))]
pub struct TunnelCloseParams {
    #[serde(deserialize_with = "read_tunnel")]
    #[cfg_attr(feature = "schema", schemars(schema_with = "tunnel_id"))]
    pub tunnel: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn read_tunnel<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    match u64::deserialize(d)? {
        0 => Err(serde::de::Error::custom("a tunnel is numbered from 1")),
        tunnel => Ok(tunnel),
    }
}

fn read_credit<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let bytes = u32::deserialize(d)?;
    if !(1..=TUNNEL_WINDOW_BYTES).contains(&bytes) {
        return Err(serde::de::Error::custom("credit is 1 to 262144 bytes"));
    }
    Ok(bytes)
}

fn read_base64<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let data = String::deserialize(d)?;
    if data.len() > MAX_TUNNEL_CHUNK_TEXT || !crate::shape::is_base64(&data) {
        return Err(serde::de::Error::custom(
            "data is base64 text of at most 65536 bytes",
        ));
    }
    Ok(data)
}

#[cfg(feature = "schema")]
fn tunnel_id(_: &mut SchemaGenerator) -> Schema {
    schema(json!({"type": "integer", "minimum": 1}))
}

#[cfg(feature = "schema")]
fn credit_bytes(_: &mut SchemaGenerator) -> Schema {
    schema(json!({"type": "integer", "minimum": 1, "maximum": TUNNEL_WINDOW_BYTES}))
}

#[cfg(feature = "schema")]
fn base64_text(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "pattern": "^([A-Za-z0-9+/]{4})*([A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$",
        "maxLength": MAX_TUNNEL_CHUNK_TEXT,
    }))
}

/// `host/resource`'s params: inspect object `name` of a declared
/// custom-resource reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "HostResourceParams"))]
pub struct HostResourceParams {
    pub context: CallContext,
    #[cfg_attr(feature = "schema", schemars(schema_with = "identifier"))]
    pub capability: String,
    #[cfg_attr(feature = "schema", schemars(schema_with = "object_name"))]
    pub name: String,
}

/// `host/action`'s params: run one of the app's declared actions on object
/// `name`, as read (`uid`, `resourceVersion`), once a person has confirmed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields, remote = "Self")]
#[cfg_attr(feature = "schema", schemars(rename = "HostActionParams"))]
pub struct HostActionParams {
    pub context: CallContext,
    #[cfg_attr(feature = "schema", schemars(schema_with = "identifier"))]
    pub capability: String,
    #[cfg_attr(feature = "schema", schemars(schema_with = "object_name"))]
    pub name: String,
    #[cfg_attr(feature = "schema", schemars(schema_with = "identifier"))]
    pub action: String,
    #[cfg_attr(feature = "schema", schemars(schema_with = "token"))]
    pub uid: String,
    #[cfg_attr(feature = "schema", schemars(schema_with = "token"))]
    pub resource_version: String,
}

/// Why [`CallContext::new`] or [`CallContext::validate`] refused.
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
        let context = CallContext {
            cluster_id: cluster.into(),
            namespace: namespace.map(str::to_owned),
        };
        context.validate()?;
        Ok(context)
    }

    /// Whether this context passes the checks [`CallContext::new`] makes.
    /// The fields are `pub`, so a context written as a struct literal or read
    /// from JSON can skip `new`: this holds it to the same shapes, for a
    /// caller that checks a call before sending it.
    pub fn validate(&self) -> Result<(), ContextError> {
        if !crate::shape::is_cluster_id(&self.cluster_id) {
            return Err(ContextError::ClusterId);
        }
        if let Some(namespace) = &self.namespace {
            if !crate::shape::is_namespace(namespace) {
                return Err(ContextError::Namespace(namespace.clone()));
            }
        }
        Ok(())
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
    HostActionParams,
    HostBindingAvailabilityParams,
    HostRunJobParams,
    JobContext,
    TunnelDataParams,
    TunnelCreditParams,
    TunnelCloseParams
);

/// A required field that may be null: absent is refused, null is `None`.
fn explicit<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

#[cfg(feature = "schema")]
fn schema(value: Value) -> Schema {
    serde_json::from_value(value).expect("a JSON Schema")
}

// The shapes srelens holds a call's fields to (`crates/plugin-host/src/sidecar/broker.rs`,
// whose conformance test holds these to it). Every pattern is RE2-compatible:
// no lookaround, so `.` and `..` are refused with `not` rather than a lookahead.

#[cfg(feature = "schema")]
fn identifier(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_IDENTIFIER_LEN,
        "pattern": format!("^[A-Za-z0-9-]{{1,{MAX_IDENTIFIER_LEN}}}$"),
    }))
}

#[cfg(feature = "schema")]
fn object_name(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_OBJECT_NAME_LEN,
        "pattern": format!("^[A-Za-z0-9.-]{{1,{MAX_OBJECT_NAME_LEN}}}$"),
        "not": {"enum": [".", ".."]},
    }))
}

#[cfg(feature = "schema")]
fn token(_: &mut SchemaGenerator) -> Schema {
    schema(json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_TOKEN_LEN,
        "pattern": format!("^[!-~]{{1,{MAX_TOKEN_LEN}}}$"),
    }))
}

#[cfg(feature = "schema")]
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
#[cfg(feature = "schema")]
const WHITE_SPACE: &str = "\t\n\u{0b}\u{0c}\r \u{85}\u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}";

#[cfg(feature = "schema")]
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
        assert!(serde_json::from_value::<HostReadParams>(
            json!({"context":context,"capability":"apps","cursor":"next-page"})
        )
        .is_ok());
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

    #[test]
    fn a_context_built_by_hand_is_checked_by_validate_as_new_checks_its_arguments() {
        // The fields are `pub`, so a struct literal skips `new`'s checks.
        let by_hand = |cluster_id: &str, namespace: Option<&str>| CallContext {
            cluster_id: cluster_id.to_owned(),
            namespace: namespace.map(str::to_owned),
        };
        assert_eq!(by_hand("kind-dev", Some("team")).validate(), Ok(()));
        assert_eq!(by_hand("kind-dev", None).validate(), Ok(()));
        assert_eq!(by_hand(&"x".repeat(4096), None).validate(), Ok(()));
        for cluster in ["", "   ", &"x".repeat(4097), &"é".repeat(3000)] {
            assert_eq!(
                by_hand(cluster, None).validate(),
                Err(ContextError::ClusterId),
                "{:?}",
                cluster.chars().take(8).collect::<String>()
            );
        }
        for namespace in ["", "Team", "-team", "team-", "te.am", &"a".repeat(64)] {
            assert_eq!(
                by_hand("kind-dev", Some(namespace)).validate(),
                Err(ContextError::Namespace(namespace.to_owned())),
                "{namespace:?}"
            );
        }
        // The cluster is checked first, as `new` does.
        assert_eq!(
            by_hand("", Some("Team")).validate(),
            Err(ContextError::ClusterId)
        );
        // What `new` builds always validates.
        let built = CallContext::new("kind-dev", Some("team")).unwrap();
        assert_eq!(built.validate(), Ok(()));
    }

    fn connect() -> Value {
        json!({"context": {"clusterId": "prod", "namespace": "team"},
            "capability": "kafka", "pod": "broker-0", "tunnel": 1})
    }

    #[test]
    fn connect_takes_exactly_one_of_pod_and_service() {
        let ok: HostConnectParams = serde_json::from_value(connect()).unwrap();
        assert_eq!(ok.pod.as_deref(), Some("broker-0"));
        assert_eq!(serde_json::to_value(&ok).unwrap(), connect());
        let mut service = connect();
        service.as_object_mut().unwrap().remove("pod");
        service["service"] = json!("kafka");
        assert!(serde_json::from_value::<HostConnectParams>(service).is_ok());
        let mut both = connect();
        both["service"] = json!("kafka");
        assert!(serde_json::from_value::<HostConnectParams>(both).is_err());
        let mut neither = connect();
        neither.as_object_mut().unwrap().remove("pod");
        assert!(serde_json::from_value::<HostConnectParams>(neither).is_err());
    }

    #[test]
    fn connect_refuses_tunnel_zero() {
        let mut zero = connect();
        zero["tunnel"] = json!(0);
        assert!(serde_json::from_value::<HostConnectParams>(zero).is_err());
        assert!(
            serde_json::from_value::<TunnelDataParams>(json!({"tunnel": 0, "data": ""})).is_err()
        );
        assert!(serde_json::from_value::<TunnelCloseParams>(json!({"tunnel": 0})).is_err());
    }

    #[test]
    fn connect_refuses_a_null_namespace() {
        let mut null = connect();
        null["context"]["namespace"] = Value::Null;
        assert!(serde_json::from_value::<HostConnectParams>(null).is_err());
    }

    #[test]
    fn credit_is_1_to_the_window() {
        let credit = |bytes: u64| {
            serde_json::from_value::<TunnelCreditParams>(json!({"tunnel": 1, "bytes": bytes}))
        };
        assert!(credit(0).is_err());
        assert_eq!(credit(1).unwrap().bytes, 1);
        assert_eq!(credit(262_144).unwrap().bytes, TUNNEL_WINDOW_BYTES);
        assert!(credit(262_145).is_err());
    }

    #[test]
    fn tunnel_data_is_base64_text() {
        let data = |text: &str| {
            serde_json::from_value::<TunnelDataParams>(json!({"tunnel": 1, "data": text}))
        };
        for ok in ["", "aGk=", "aGkh", "aA=="] {
            assert!(data(ok).is_ok(), "{ok:?}");
        }
        for bad in ["aGk", "aGk=\n", "a=Gk", "a-Gk", "aGk=aGk=", "=", "é==="] {
            assert!(data(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn tunnel_data_is_at_most_a_chunk_of_base64() {
        let data = |len: usize| {
            serde_json::from_value::<TunnelDataParams>(
                json!({"tunnel": 1, "data": "A".repeat(len)}),
            )
        };
        assert!(data(87_384).is_ok());
        assert!(data(87_388).is_err());
    }

    #[test]
    fn tunnel_close_may_say_why() {
        let close: TunnelCloseParams = serde_json::from_value(json!({"tunnel": 2})).unwrap();
        assert_eq!(close.message, None);
        assert_eq!(serde_json::to_value(&close).unwrap(), json!({"tunnel": 2}));
        assert!(serde_json::from_value::<TunnelCloseParams>(
            json!({"tunnel": 2, "message": "eof", "x": 1})
        )
        .is_err());
    }
}
