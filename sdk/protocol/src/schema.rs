//! The protocol as one JSON Schema (draft-07) document, generated from this
//! crate's types and committed as `schemas/sidecar-protocol.v<line>.json`.
//!
//! - `definitions`: every params and result type, and `HostMessage` and
//!   `SidecarMessage`, a `oneOf` over every line each side writes.
//! - The root is `anyOf` the two: a line of either kind validates. They
//!   overlap on responses, which is why it is not `oneOf`.
//! - `x-srelens-methods`, `x-srelens-errorCodes`, `x-srelens-apiVersion` and
//!   `x-srelens-maxMessageBytes`: what an SDK in another language generates
//!   its constants from, so it needs nothing but this file.
//!
//! A response's result cannot be tied to its request inside one line, which
//! does not carry the method: the method table is where result types live.

use schemars::gen::SchemaSettings;
use serde_json::{json, Map, Value};

use crate::methods::{Kind, METHODS};
use crate::{code, is_reserved, RequestId, RpcError, MAX_MESSAGE_BYTES, SIDECAR_API_VERSIONS};

/// The newest sidecar API line, `0.1` for 0.1.0.
fn line() -> String {
    let newest = SIDECAR_API_VERSIONS.last().expect("at least one version");
    newest.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// The committed file's name, one per API line: `sidecar-protocol.v0.1.json`.
pub fn schema_file() -> String {
    format!("sidecar-protocol.v{}.json", line())
}

/// The whole schema document.
pub fn schema() -> Value {
    let mut generator = SchemaSettings::draft07().into_generator();
    let request_id = to_value(generator.subschema_for::<RequestId>());
    let rpc_error = to_value(generator.subschema_for::<RpcError>());
    // srelens numbers its own requests from 1.
    let host_id = json!({"type": "integer", "minimum": 1});

    let mut table = Map::new();
    let mut host = Vec::new();
    let mut sidecar = Vec::new();
    for spec in METHODS {
        let params = to_value((spec.params)(&mut generator));
        let mut entry = json!({
            "direction": spec.direction.wire_name(),
            "kind": spec.kind.wire_name(),
            "params": params,
        });
        if let Some(result) = spec.result {
            entry["result"] = to_value(result(&mut generator));
        }
        table.insert(spec.name.to_owned(), entry);

        let method = json!({"const": spec.name});
        if spec.direction.from_host() {
            host.push(match spec.kind {
                Kind::Request => request(method.clone(), params.clone(), host_id.clone()),
                Kind::Notification => notification(method.clone(), params.clone(), true),
            });
        }
        if spec.direction.from_sidecar() {
            sidecar.push(match spec.kind {
                Kind::Request => request(method, params, request_id.clone()),
                Kind::Notification => notification(method, params, true),
            });
        }
    }

    // An app operation: a request from srelens under any name it does not reserve.
    let reserved: Vec<&str> = METHODS
        .iter()
        .map(|spec| spec.name)
        .filter(|name| is_reserved(name))
        .collect();
    host.push(request(
        json!({
            "type": "string",
            "not": {"anyOf": [
                {"enum": reserved},
                {"pattern": "^\\$/"},
                {"pattern": "^stream/"},
            ]},
        }),
        json!({"type": ["object", "array"]}),
        host_id.clone(),
    ));
    // srelens's answer to a sidecar's call, under the sidecar's own id.
    host.push(response(request_id, rpc_error.clone()));

    // A notification from a newer SDK, which srelens ignores.
    let sidecar_notifications: Vec<&str> = METHODS
        .iter()
        .filter(|spec| spec.kind == Kind::Notification && spec.direction.from_sidecar())
        .map(|spec| spec.name)
        .collect();
    sidecar.push(notification(
        json!({"type": "string", "not": {"enum": sidecar_notifications}}),
        json!({"type": ["object", "array"]}),
        false,
    ));
    // The sidecar's answer to one of srelens's requests.
    sidecar.push(response(host_id, rpc_error));

    let mut definitions = to_value(generator.take_definitions());
    definitions["HostMessage"] = json!({
        "description": "One line srelens writes to a sidecar's stdin.",
        "oneOf": host,
    });
    definitions["SidecarMessage"] = json!({
        "description": "One line a sidecar writes to its stdout.",
        "oneOf": sidecar,
    });
    let errors: Map<String, Value> = code::ALL
        .iter()
        .map(|(name, value)| ((*name).to_owned(), json!(value)))
        .collect();

    let mut document = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": format!(
            "https://raw.githubusercontent.com/srelens/srelens/main/schemas/{}",
            schema_file()
        ),
        "title": "srelens sidecar protocol",
        "description": "JSON-RPC 2.0 between srelens and an executable extension's sidecar, one message per line on its stdin and stdout. The prose is docs/extensions/sidecar-protocol.md.",
        "anyOf": [
            {"$ref": "#/definitions/HostMessage"},
            {"$ref": "#/definitions/SidecarMessage"},
        ],
        "definitions": definitions,
        "x-srelens-apiVersion": SIDECAR_API_VERSIONS.last().expect("at least one version"),
        "x-srelens-maxMessageBytes": MAX_MESSAGE_BYTES,
        "x-srelens-errorCodes": errors,
        "x-srelens-methods": table,
    });
    strip_nonstandard_formats(&mut document);
    document
}

fn to_value(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("a schema is plain JSON")
}

fn jsonrpc() -> Value {
    json!({"const": "2.0"})
}

/// A request: it always has an id, a method and params.
fn request(method: Value, params: Value, id: Value) -> Value {
    json!({
        "type": "object",
        "required": ["jsonrpc", "id", "method", "params"],
        "properties": {"jsonrpc": jsonrpc(), "id": id, "method": method, "params": params},
    })
}

/// A notification: never an id.
fn notification(method: Value, params: Value, params_required: bool) -> Value {
    let mut required = vec!["jsonrpc", "method"];
    if params_required {
        required.push("params");
    }
    json!({
        "type": "object",
        "required": required,
        "properties": {"jsonrpc": jsonrpc(), "method": method, "params": params},
        "not": {"required": ["id"]},
    })
}

/// An answer: exactly one of `result` and `error`, and never a method, so it
/// cannot be mistaken for a request.
fn response(id: Value, error: Value) -> Value {
    json!({
        "type": "object",
        "required": ["jsonrpc", "id"],
        "properties": {"jsonrpc": jsonrpc(), "id": id, "error": error},
        "oneOf": [{"required": ["result"]}, {"required": ["error"]}],
        "not": {"required": ["method"]},
    })
}

/// The `format` values JSON Schema draft-07 defines. schemars also writes
/// Rust's own (`uint64`, `double`), which other validators reject or ignore;
/// the manifest schema strips them the same way
/// (`crates/plugin-host/src/manifest.rs`, which this crate cannot reach).
const STANDARD_FORMATS: &[&str] = &[
    "date-time",
    "date",
    "time",
    "email",
    "idn-email",
    "hostname",
    "idn-hostname",
    "ipv4",
    "ipv6",
    "uri",
    "uri-reference",
    "iri",
    "iri-reference",
    "uri-template",
    "json-pointer",
    "relative-json-pointer",
    "regex",
];

fn strip_nonstandard_formats(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // A string `format` is the keyword; a property named `format` is an object.
            if map
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(|format| !STANDARD_FORMATS.contains(&format))
            {
                map.remove("format");
            }
            map.values_mut().for_each(strip_nonstandard_formats);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_nonstandard_formats),
        _ => {}
    }
}
