//! The wire protocol between srelens and an executable extension's sidecar
//! (#572, #573): JSON-RPC 2.0, one message per line on the sidecar's stdin
//! and stdout.
//!
//! The contract both sides build on. The host (`srelens-plugin-host`) and the
//! SDKs (#576) share these constants and types, and the committed
//! `schemas/sidecar-protocol.v0.1.json` is generated from them. The prose is
//! `docs/extensions/sidecar-protocol.md`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

mod bounds;
mod messages;
mod methods;
mod schema;

pub use bounds::{
    MAX_CALL_FIELD_BYTES, MAX_CLUSTER_ID_BYTES, MAX_IDENTIFIER_LEN, MAX_NAMESPACE_LEN,
    MAX_OBJECT_NAME_LEN, MAX_TOKEN_LEN,
};
pub use methods::{is_reserved, Direction, Kind, MethodSpec, METHODS};
pub use schema::{schema, schema_file};

pub use messages::{
    CallContext, CancelParams, Empty, HostActionParams, HostReadParams, HostResourceParams,
    InitializeLimits, InitializeParams, InitializeResult, Peer, RequestId, StreamCancelParams,
    StreamCloseParams, StreamDataParams, StreamErrorParams, StreamOpenParams,
    UnsupportedApiVersion,
};

/// Sidecar API versions this host speaks, oldest first. `initialize` offers
/// all of them and the sidecar answers with the one it chose.
///
/// Its own line, not the extension API's (`SUPPORTED_API_VERSIONS`): no
/// manifest kind runs a sidecar yet, so there is no extension API version for
/// it to belong to. The executable kind (#574) kept the two apart.
pub const SIDECAR_API_VERSIONS: &[&str] = &["0.1.0"];

/// The longest line a sidecar may write, not counting its line ending. The
/// same bound as one MCP request (`MAX_REQUEST_BYTES` in `srelens-mcp`).
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Method names. The lifecycle methods and everything under `$/` and
/// `stream/` belong to the host: [`is_reserved`] refuses them as app requests.
pub mod method {
    /// Host → sidecar, first: version negotiation and the limits in force.
    pub const INITIALIZE: &str = "initialize";
    /// Host → sidecar, after `initialize`: start serving requests.
    pub const ACTIVATE: &str = "activate";
    /// Host → sidecar: stop serving; no new requests follow.
    pub const DEACTIVATE: &str = "deactivate";
    /// Host → sidecar, periodically: answer to show it is not hung.
    pub const HEALTH: &str = "health";
    /// Host → sidecar, last: exit. The host kills it if it does not.
    pub const SHUTDOWN: &str = "shutdown";
    /// Host → sidecar notification: `{"id": n}`, stop working on request n.
    pub const CANCEL: &str = "$/cancelRequest";
    /// Host → sidecar request: `{"stream": n, "method": …, "params": …}`.
    pub const STREAM_OPEN: &str = "stream/open";
    /// Sidecar → host notification: `{"stream": n, "data": …}`.
    pub const STREAM_DATA: &str = "stream/data";
    /// Sidecar → host notification: `{"stream": n}`, the stream ended.
    pub const STREAM_CLOSE: &str = "stream/close";
    /// Sidecar → host notification: `{"stream": n, "message": …}`, it failed.
    pub const STREAM_ERROR: &str = "stream/error";
    /// Host → sidecar notification: `{"stream": n}`, stop sending it.
    pub const STREAM_CANCEL: &str = "stream/cancel";
    /// Sidecar → host request (#573): read one of the app's declared readers
    /// or `network.http` requests, through `extensions.read`.
    pub const HOST_READ: &str = "host/read";
    /// Sidecar → host request (#573): inspect one resource of a declared
    /// custom-resource reader, through `extensions.resource`.
    pub const HOST_RESOURCE: &str = "host/resource";
    /// Sidecar → host request (#573): run one of the app's declared actions,
    /// through `extensions.action`, once a person has confirmed it.
    pub const HOST_ACTION: &str = "host/action";
}

/// Error codes. The first five are JSON-RPC 2.0's own.
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// A sidecar's answer to `initialize` when it speaks none of the offered
    /// versions. Its `data` may carry `{"supported": [...]}`.
    pub const UNSUPPORTED_API_VERSION: i64 = -32001;
    /// The answer to a request the host cancelled, or to a sidecar's call it
    /// cancelled itself. The value LSP uses.
    pub const REQUEST_CANCELLED: i64 = -32800;
    /// A sidecar's call that needed a person's confirmation did not get it: a
    /// person declined, or no one could be asked (#573). Nothing ran.
    pub const CONSENT_DENIED: i64 = -32002;
    /// A sidecar's call reached the host capability, which refused it or
    /// failed: the app is not enabled for the cluster, a grant is missing, the
    /// cluster said no. The message is the capability's own (#573).
    pub const CAPABILITY_FAILED: i64 = -32003;

    /// Every code, by the name the schema gives it (`x-srelens-errorCodes`),
    /// so an SDK in another language generates the same constants.
    pub const ALL: &[(&str, i64)] = &[
        ("parseError", PARSE_ERROR),
        ("invalidRequest", INVALID_REQUEST),
        ("methodNotFound", METHOD_NOT_FOUND),
        ("invalidParams", INVALID_PARAMS),
        ("internalError", INTERNAL_ERROR),
        ("unsupportedApiVersion", UNSUPPORTED_API_VERSION),
        ("consentDenied", CONSENT_DENIED),
        ("capabilityFailed", CAPABILITY_FAILED),
        ("requestCancelled", REQUEST_CANCELLED),
    ];
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn every_error_code_has_one_name_and_one_value() {
        let names: HashSet<_> = code::ALL.iter().map(|(name, _)| name).collect();
        let values: HashSet<_> = code::ALL.iter().map(|(_, value)| value).collect();
        assert_eq!(code::ALL.len(), 9);
        assert_eq!(names.len(), code::ALL.len());
        assert_eq!(values.len(), code::ALL.len());
        assert!(code::ALL.contains(&("requestCancelled", -32800)));
        assert!(code::ALL.contains(&("unsupportedApiVersion", -32001)));
    }

    #[test]
    fn an_error_object_leaves_out_data_it_does_not_have() {
        assert_eq!(
            serde_json::to_value(RpcError::new(code::METHOD_NOT_FOUND, "no")).unwrap(),
            json!({"code": -32601, "message": "no"})
        );
        let parsed: RpcError = serde_json::from_value(
            json!({"code": -32001, "message": "m", "data": {"supported": ["1.0.0"]}}),
        )
        .unwrap();
        assert_eq!(parsed.data, Some(json!({"supported": ["1.0.0"]})));
        assert_eq!(parsed.to_string(), "m (code -32001)");
    }
}
