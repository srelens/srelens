//! The committed schema is what an SDK generates its types from and a sidecar
//! validates against. It must equal what this crate's types generate.

use serde_json::Value;
use srelens_sidecar_protocol::{
    code, schema, schema_file, Kind, MAX_MESSAGE_BYTES, METHODS, SIDECAR_API_VERSIONS,
};
use std::path::PathBuf;

fn committed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas")
        .join(schema_file())
}

/// Regenerate with `UPDATE_CATALOG=1 cargo test -p srelens-sidecar-protocol --test schema`,
/// the knob the manifest schema and the capability catalog use.
#[test]
fn committed_protocol_schema_matches_the_types() {
    let path = committed_path();
    let generated = schema();
    if std::env::var("UPDATE_CATALOG").is_ok() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&generated).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing: run UPDATE_CATALOG=1 cargo test -p srelens-sidecar-protocol --test schema",
            path.display()
        )
    });
    let committed: Value = serde_json::from_str(&committed).unwrap();
    // Parsed values, never text: key order depends on the workspace's serde_json features.
    assert!(
        committed == generated,
        "{} is stale: run UPDATE_CATALOG=1 cargo test -p srelens-sidecar-protocol --test schema",
        path.display()
    );
}

/// Every string under `key`, anywhere in `value`.
fn strings_under<'a>(value: &'a Value, key: &str, out: &mut Vec<&'a str>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(found)) = map.get(key) {
                out.push(found);
            }
            map.values().for_each(|v| strings_under(v, key, out));
        }
        Value::Array(items) => items.iter().for_each(|v| strings_under(v, key, out)),
        _ => {}
    }
}

#[test]
fn the_schema_is_plain_draft_07() {
    let schema = schema();
    assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
    let mut formats = Vec::new();
    strings_under(&schema, "format", &mut formats);
    const DRAFT_07: &[&str] = &[
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
    for format in formats {
        assert!(
            DRAFT_07.contains(&format),
            "a format draft-07 does not define: {format}"
        );
    }
}

/// Go's RE2 has no lookaround and no backreferences; neither has the `regex`
/// crate. A pattern it compiles, the Go SDK can compile.
#[test]
fn every_pattern_compiles_without_lookaround() {
    let schema = schema();
    let mut patterns = Vec::new();
    strings_under(&schema, "pattern", &mut patterns);
    // Identifier, object name, token, namespace, cluster ID, and the reserved-method prefixes.
    assert!(
        patterns.len() >= 7,
        "only {} patterns: {patterns:?}",
        patterns.len()
    );
    for pattern in patterns {
        regex::Regex::new(pattern).unwrap_or_else(|e| panic!("{pattern}: {e}"));
    }
}

#[test]
fn the_schema_lists_every_method_with_its_direction_kind_and_types() {
    let schema = schema();
    let table = schema["x-srelens-methods"]
        .as_object()
        .expect("a method table");
    assert_eq!(table.len(), METHODS.len());
    for spec in METHODS {
        let entry = &table[spec.name];
        assert_eq!(
            entry["direction"],
            spec.direction.wire_name(),
            "{}",
            spec.name
        );
        assert_eq!(entry["kind"], spec.kind.wire_name(), "{}", spec.name);
        assert!(entry.get("params").is_some(), "{}", spec.name);
        assert_eq!(
            entry.get("result").is_some(),
            spec.kind == Kind::Request,
            "{}",
            spec.name
        );
    }
    assert_eq!(
        table["initialize"]["params"]["$ref"],
        "#/definitions/InitializeParams"
    );
    assert_eq!(
        table["host/action"]["params"]["$ref"],
        "#/definitions/HostActionParams"
    );
}

#[test]
fn the_schema_carries_what_an_sdk_needs_beside_the_types() {
    let schema = schema();
    assert_eq!(
        schema["x-srelens-apiVersion"],
        *SIDECAR_API_VERSIONS.last().unwrap()
    );
    assert_eq!(schema["x-srelens-maxMessageBytes"], MAX_MESSAGE_BYTES);
    for (name, value) in code::ALL {
        assert_eq!(schema["x-srelens-errorCodes"][name], *value, "{name}");
    }
    assert!(schema["$id"]
        .as_str()
        .unwrap()
        .ends_with(&format!("/schemas/{}", schema_file())));
    for definition in [
        "HostMessage",
        "SidecarMessage",
        "CallContext",
        "RpcError",
        "RequestId",
    ] {
        assert!(
            schema["definitions"].get(definition).is_some(),
            "no definition {definition}"
        );
    }
    assert_eq!(schema_file(), "sidecar-protocol.v0.1.json");
}
